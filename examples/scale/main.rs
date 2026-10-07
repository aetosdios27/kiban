mod cache;
mod config;
mod metrics;
#[path = "../support/random.rs"]
mod random;
mod telemetry;
mod workload;

use config::Config;
use kiban::db::SharedKiban;
use metrics::Metrics;
use std::io;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};
use telemetry::Reporter;

#[derive(Default)]
struct Published {
    total: Metrics,
    interval: Metrics,
}
fn publish(shared: &Mutex<Published>, local: &mut Metrics) {
    let mut p = shared.lock().unwrap();
    p.total.merge(local);
    p.interval.merge(local);
    *local = Metrics::default();
}
#[allow(clippy::too_many_arguments)]
fn run_stage(
    c: &Config,
    db: &SharedKiban,
    reporter: &mut Reporter,
    stage: &workload::Stage,
    index: usize,
    budget: Option<u64>,
    duration: Option<Duration>,
    previous: &Metrics,
) -> io::Result<(Metrics, f64)> {
    let shared = Arc::new(Mutex::new(Published::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(stage.threads + 1));
    let value = Arc::new(random::value(c.seed, c.value_size));
    let mut handles = Vec::new();
    for thread in 0..stage.threads {
        let (c, db, stage, shared, stop, barrier, value) = (
            c.clone(),
            db.clone(),
            stage.clone(),
            shared.clone(),
            stop.clone(),
            barrier.clone(),
            value.clone(),
        );
        handles.push(std::thread::spawn(move || -> io::Result<()> {
            let mut local = Metrics::default();
            let mut id = thread as u64;
            let mut writes = 0u64;
            let mut keyed = vec![0; c.value_size];
            barrier.wait();
            let start = Instant::now();
            let result = (|| {
                while budget.is_none_or(|n| id < n)
                    && duration.is_none_or(|d| start.elapsed() < d)
                    && !stop.load(Ordering::Relaxed)
                {
                    let (read, key) = workload::operation(&c, &stage, index as u64, id);
                    let expected = if c.value_pattern == "keyed" {
                        random::keyed_value(c.seed, u64::from_be_bytes(key), &mut keyed);
                        keyed.as_slice()
                    } else {
                        value.as_slice()
                    };
                    let started = Instant::now();
                    if read {
                        let got = db.get(key).map_err(io::Error::other)?;
                        local
                            .reads
                            .record(started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
                        if let Some(got) = got {
                            if got != expected {
                                return Err(io::Error::other(format!(
                                    "read content mismatch at stage={index} op={id}"
                                )));
                            }
                        } else {
                            if u64::from_be_bytes(key) < c.preload_keys {
                                return Err(io::Error::other("missing preloaded key"));
                            }
                            local.misses += 1;
                        }
                    } else {
                        db.put(key, expected)?;
                        writes += 1;
                        if c.sync_every > 0 && writes.is_multiple_of(c.sync_every) {
                            db.sync()?;
                            local.syncs += 1;
                        }
                        local
                            .writes
                            .record(started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
                        local.app_bytes += 8 + c.value_size as u64;
                    }
                    if (local.reads.count + local.writes.count) >= 256 {
                        publish(&shared, &mut local);
                    }
                    id = id
                        .checked_add(stage.threads as u64)
                        .ok_or_else(|| io::Error::other("operation ID overflow"))?;
                }
                Ok(())
            })();
            publish(&shared, &mut local);
            if result.is_err() {
                stop.store(true, Ordering::Relaxed);
            }
            result
        }));
    }
    let start = Instant::now();
    barrier.wait();
    let mut next_sample = Instant::now() + Duration::from_millis(c.sample_ms);
    let mut sampling_error = None;
    while !handles.iter().all(|h| h.is_finished()) {
        if Instant::now() >= next_sample {
            let (interval, mut total) = {
                let mut p = shared.lock().unwrap();
                (std::mem::take(&mut p.interval), p.total.clone())
            };
            total.merge(previous);
            if let Err(e) = reporter.sample(c, db, stage.name, &interval, &total) {
                sampling_error = Some(e);
                stop.store(true, Ordering::Relaxed);
                break;
            }
            next_sample = Instant::now() + Duration::from_millis(c.sample_ms);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let elapsed = start.elapsed().as_secs_f64();
    let mut worker_error = None;
    for h in handles {
        match h.join() {
            Ok(Ok(())) => (),
            Ok(Err(e)) => worker_error = Some(e),
            Err(_) => worker_error = Some(io::Error::other("worker panicked")),
        }
    }
    if let Some(e) = worker_error.or(sampling_error) {
        return Err(e);
    }
    let p = shared.lock().unwrap();
    let mut total = p.total.clone();
    total.merge(previous);
    reporter.sample(c, db, stage.name, &p.interval, &total)?;
    reporter.summary(stage.name, stage.threads, elapsed, &p.total)?;
    Ok((p.total.clone(), elapsed))
}

fn run(c: Config) -> io::Result<()> {
    std::fs::create_dir_all(&c.db)?;
    let db_path = std::fs::canonicalize(&c.db)?;
    if !c.reuse && std::fs::read_dir(&db_path)?.next().is_some() {
        return Err(io::Error::other(
            "database is nonempty; use a fresh directory or explicit --reuse true",
        ));
    }
    std::fs::create_dir_all(&c.output)?;
    let output_path = std::fs::canonicalize(&c.output)?;
    if output_path.starts_with(&db_path) || db_path.starts_with(&output_path) {
        return Err(io::Error::other(
            "database and output must be separate directories",
        ));
    }
    telemetry::write_config(&c)?;
    let mut status = telemetry::create(&c.output.join("status.csv"))?;
    writeln!(status, "status,detail")?;
    writeln!(
        status,
        "started,{}",
        telemetry::csv("run incomplete until complete record")
    )?;
    let result = measure(&c);
    match &result {
        Ok(()) => writeln!(status, "complete,success")?,
        Err(e) => writeln!(status, "failed,{}", telemetry::csv(&e.to_string()))?,
    }
    result
}
fn measure(c: &Config) -> io::Result<()> {
    let prep_start = Instant::now();
    let db = SharedKiban::open_with_options(&c.db, c.options.clone()).map_err(io::Error::other)?;
    let mut val = random::value(c.seed, c.value_size);
    for i in 0..c.preload_keys {
        if c.value_pattern == "keyed" {
            random::keyed_value(c.seed, i, &mut val);
        }
        if c.preload_mode == "verify" {
            if db
                .get(i.to_be_bytes())
                .map_err(io::Error::other)?
                .as_deref()
                != Some(val.as_slice())
            {
                return Err(io::Error::other("existing preload key/value mismatch"));
            }
        } else {
            db.put(i.to_be_bytes(), &val)?;
        }
    }
    if c.preload_keys > 0 && c.preload_mode == "write" {
        db.sync()?;
        db.flush().map_err(io::Error::other)?;
    }
    // Close joins the worker; reopening resets Kiban caches for both cache modes.
    drop(db);
    if c.cache == "reduced" {
        cache::reduce(&c.db)?;
    }
    let db = SharedKiban::open_with_options(&c.db, c.options.clone()).map_err(io::Error::other)?;
    if c.cache == "warm" {
        for i in 0..c.working_set {
            if c.value_pattern == "keyed" {
                random::keyed_value(c.seed, i, &mut val);
            }
            let got = db.get(i.to_be_bytes()).map_err(io::Error::other)?;
            if got.as_deref().is_some_and(|v| v != val) || (got.is_none() && i < c.preload_keys) {
                return Err(io::Error::other("warmup key/value mismatch"));
            }
        }
    }
    let mut prep = telemetry::create(&c.output.join("preparation.csv"))?;
    writeln!(
        prep,
        "elapsed_s,preload_keys,db_bytes,allocated_db_bytes\n{},{},{},{}",
        prep_start.elapsed().as_secs_f64(),
        c.preload_keys,
        telemetry::disk_bytes(&c.db)?,
        telemetry::allocated_bytes(&c.db)?
    )?;
    let mut reporter = Reporter::new(c, &db)?;
    let stages = workload::stages(c);
    let mut schedule = telemetry::create(&c.output.join("stages.csv"))?;
    writeln!(
        schedule,
        "index,name,threads,read_percent,operations_cap,duration_cap_s"
    )?;
    for (i, stage) in stages.iter().enumerate() {
        let cap = c.operations / stages.len() as u64
            + u64::from((i as u64) < c.operations % stages.len() as u64);
        writeln!(
            schedule,
            "{i},{},{},{},{},{}",
            stage.name,
            stage.threads,
            stage.reads,
            cap,
            c.duration / stages.len() as f64
        )?;
    }

    let mut total = Metrics::default();
    let mut wall = 0.0;
    reporter.sample(c, &db, "initial", &Metrics::default(), &total)?;
    for (i, stage) in stages.iter().enumerate() {
        let budget = if c.operations == 0 {
            None
        } else {
            Some(
                c.operations / stages.len() as u64
                    + u64::from((i as u64) < c.operations % stages.len() as u64),
            )
        };
        if budget == Some(0) {
            continue;
        }
        let duration = if c.duration == 0.0 {
            None
        } else {
            Some(Duration::from_secs_f64(c.duration / stages.len() as f64))
        };
        let (m, elapsed) = run_stage(c, &db, &mut reporter, stage, i, budget, duration, &total)?;
        total.merge(&m);
        wall += elapsed;
    }
    reporter.summary("total", c.threads, wall, &total)?;
    // Durability/final flush are visible separately, never folded into foreground ops/s.
    let started = Instant::now();
    db.sync()?;
    db.flush().map_err(io::Error::other)?;
    reporter.sample(c, &db, "final-flush", &Metrics::default(), &total)?;
    // Bounded observation of maintenance recovery. Does not force below-trigger L0 to compact.
    let drain_start = Instant::now();
    loop {
        let s = db.stats().map_err(io::Error::other)?;
        if !s.maintenance.compaction_running
            && !s.immutable_present
            && telemetry::debt(&s, c).2 == 0
        {
            break;
        }
        if drain_start.elapsed() > Duration::from_secs(30) {
            break;
        }
        std::thread::sleep(Duration::from_millis(c.sample_ms.min(1000)));
        reporter.sample(c, &db, "drain", &Metrics::default(), &total)?;
    }
    reporter.sample(c, &db, "final", &Metrics::default(), &total)?;
    let mut finish = telemetry::create(&c.output.join("finish.csv"))?;
    writeln!(
        finish,
        "final_sync_flush_drain_s,db_bytes\n{},{}",
        started.elapsed().as_secs_f64(),
        telemetry::disk_bytes(&c.db)?
    )?;
    Ok(())
}
fn main() {
    if std::env::args().any(|a| a == "--help") {
        println!("{}", config::HELP);
        return;
    }
    let result = Config::parse().map_err(io::Error::other).and_then(run);
    if let Err(e) = result {
        eprintln!("scale FAILED: {e}");
        std::process::exit(1);
    }
}
