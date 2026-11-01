use crate::{
    backend::Engine,
    metrics::Metrics,
    random,
    settings::Settings,
    telemetry::{self, Reporter},
    workload,
};
use serde_json::json;
use std::{
    io,
    sync::{
        Arc, Barrier, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Published {
    total: Metrics,
    interval: Metrics,
}
fn publish(p: &Mutex<Published>, m: &mut Metrics) {
    let mut p = p.lock().unwrap();
    p.total.merge(m);
    p.interval.merge(m);
    *m = Metrics::default();
}
struct Worker {
    operations: u64,
    elapsed: f64,
}
struct StageResult {
    metrics: Metrics,
    elapsed: f64,
    partitions: Vec<u64>,
}
fn stage(
    s: &Settings,
    db: &Arc<Engine>,
    r: &mut Reporter,
    stage: &workload::Stage,
    index: usize,
    cap: Option<u64>,
    duration: Option<Duration>,
) -> io::Result<StageResult> {
    let c = &s.workload;
    let p = Arc::new(Mutex::new(Published::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(stage.threads + 1));
    let start = Arc::new(OnceLock::<Instant>::new());
    let value = Arc::new(random::value(c.seed, c.value_size));
    let mut handles = Vec::new();
    for thread in 0..stage.threads {
        let (c, db, p, stop, barrier, start, value, stage) = (
            c.clone(),
            db.clone(),
            p.clone(),
            stop.clone(),
            barrier.clone(),
            start.clone(),
            value.clone(),
            stage.clone(),
        );
        handles.push(std::thread::spawn(move || -> io::Result<Worker> {
            let mut m = Metrics::default();
            let mut id = thread as u64;
            let mut writes = 0u64;
            let mut operations = 0;
            barrier.wait();
            let start = *start.get().unwrap();
            let result = (|| {
                while cap.is_none_or(|n| id < n)
                    && duration.is_none_or(|d| start.elapsed() < d)
                    && !stop.load(Ordering::Relaxed)
                {
                    let (read, key) = workload::operation(&c, &stage, index as u64, id);
                    let t = Instant::now();
                    if read {
                        let got = db.get(&key)?;
                        m.reads
                            .record(t.elapsed().as_nanos().min(u64::MAX as u128) as u64);
                        match got {
                            Some(v) if v != *value => {
                                return Err(io::Error::other(format!(
                                    "wrong read value: stage {index}, operation {id}"
                                )));
                            }
                            None if u64::from_be_bytes(key) < c.preload_keys => {
                                return Err(io::Error::other("missing preloaded key"));
                            }
                            None => m.misses += 1,
                            _ => (),
                        }
                    } else {
                        writes += 1;
                        db.put(&key, &value, c.sync_every == 1)?;
                        if c.sync_every == 1 {
                            m.syncs += 1;
                        } else if c.sync_every > 1 && writes.is_multiple_of(c.sync_every) {
                            db.sync()?;
                            m.syncs += 1;
                        }
                        m.writes
                            .record(t.elapsed().as_nanos().min(u64::MAX as u128) as u64);
                        m.app_bytes += 8 + c.value_size as u64;
                    }
                    operations += 1;
                    if m.reads.count + m.writes.count >= 4096 {
                        publish(&p, &mut m);
                    }
                    id = id
                        .checked_add(stage.threads as u64)
                        .ok_or_else(|| io::Error::other("operation ID overflow"))?;
                }
                Ok(())
            })();
            publish(&p, &mut m);
            if result.is_err() {
                stop.store(true, Ordering::Relaxed);
            }
            result?;
            Ok(Worker {
                operations,
                elapsed: start.elapsed().as_secs_f64(),
            })
        }));
    }
    start.set(Instant::now()).unwrap();
    barrier.wait();
    let mut last = Instant::now();
    let mut sampling_error = None;
    while !handles.iter().all(|h| h.is_finished()) {
        if last.elapsed() >= Duration::from_millis(c.sample_ms) {
            let interval = std::mem::take(&mut p.lock().unwrap().interval);
            if let Err(e) = r.sample(s, db, stage.name, &interval, last.elapsed().as_secs_f64()) {
                sampling_error = Some(e);
                stop.store(true, Ordering::Relaxed);
                break;
            }
            last = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut partitions = Vec::new();
    let mut elapsed = 0f64;
    let mut worker_error = None;
    for h in handles {
        match h.join() {
            Ok(Ok(w)) => {
                partitions.push(w.operations);
                elapsed = elapsed.max(w.elapsed);
            }
            Ok(Err(e)) => worker_error = Some(e),
            Err(_) => worker_error = Some(io::Error::other("worker panic")),
        }
    }
    if let Some(e) = worker_error.or(sampling_error) {
        return Err(e);
    }
    let p = p.lock().unwrap();
    r.sample(s, db, stage.name, &p.interval, last.elapsed().as_secs_f64())?;
    Ok(StageResult {
        metrics: p.total.clone(),
        elapsed,
        partitions,
    })
}
pub fn run(s: Settings) -> io::Result<()> {
    let c = &s.workload;
    std::fs::create_dir_all(&c.db)?;
    if std::fs::read_dir(&c.db)?.next().is_some() {
        return Err(io::Error::other("nonempty DB; use a fresh directory"));
    }
    std::fs::create_dir_all(&c.output)?;
    let dbpath = std::fs::canonicalize(&c.db)?;
    let outpath = std::fs::canonicalize(&c.output)?;
    if dbpath.starts_with(&outpath) || outpath.starts_with(&dbpath) {
        return Err(io::Error::other(
            "DB and output directories must be separate",
        ));
    }
    telemetry::write_json(&c.output.join("config.json"), &s.json())?;
    telemetry::write_json(&c.output.join("status.json"), &json!({"status":"started"}))?;
    let result = measure(&s);
    std::fs::write(
        c.output.join("status.json"),
        serde_json::to_string_pretty(&match &result {
            Ok(()) => json!({"status":"complete"}),
            Err(e) => json!({"status":"failed","error":e.to_string()}),
        })?,
    )?;
    result
}
fn measure(s: &Settings) -> io::Result<()> {
    let c = &s.workload;
    let prep = Instant::now();
    let db = Engine::open(s)?;
    let value = random::value(c.seed, c.value_size);
    for i in 0..c.preload_keys {
        db.put(&i.to_be_bytes(), &value, false)?;
    }
    db.sync()?;
    db.flush()?;
    drop(db);
    if c.cache == "reduced" {
        crate::cache::reduce(&c.db)?;
    }
    let db = Arc::new(Engine::open(s)?);
    if c.cache == "warm" {
        for key in 0..c.working_set {
            let _ = db.get(&key.to_be_bytes())?;
        }
    }
    telemetry::write_json(
        &c.output.join("preparation.json"),
        &json!({"elapsed_s":prep.elapsed().as_secs_f64(),"db_bytes":telemetry::disk_bytes(&c.db)?,"engine":db.stats()?}),
    )?;
    if s.engine == "rocksdb" {
        for e in std::fs::read_dir(&c.db)? {
            let e = e?;
            if e.file_name().to_string_lossy().starts_with("OPTIONS-") {
                std::fs::copy(e.path(), c.output.join(e.file_name()))?;
            }
        }
    }
    let mut reporter = Reporter::new(s, &db)?;
    reporter.sample(s, &db, "initial", &Metrics::default(), 1.0)?;
    let stages = workload::stages(c);
    let mut schedule = Vec::new();
    let mut summaries = Vec::new();
    let mut partitions = Vec::new();
    let mut total = Metrics::default();
    let mut seconds = 0.0;
    for (i, st) in stages.iter().enumerate() {
        let cap = (c.operations > 0).then(|| {
            c.operations / stages.len() as u64
                + u64::from((i as u64) < c.operations % stages.len() as u64)
        });
        let duration =
            (c.duration > 0.0).then(|| Duration::from_secs_f64(c.duration / stages.len() as f64));
        schedule.push(json!({"stage":st.name,"threads":st.threads,"reads_percent":st.reads,"operations_cap":cap,"duration_cap_s":duration.map(|d|d.as_secs_f64())}));
        if cap == Some(0) {
            partitions.push(vec![0; st.threads]);
            continue;
        }
        let result = stage(s, &db, &mut reporter, st, i, cap, duration)?;
        total.merge(&result.metrics);
        seconds += result.elapsed;
        summaries.push(json!({"stage":st.name,"threads":st.threads,"elapsed_s":result.elapsed,"metrics":telemetry::metrics(&result.metrics,result.elapsed),"partition_counts":result.partitions}));
        partitions.push(result.partitions);
    }
    telemetry::write_json(&c.output.join("stages.json"), &json!(schedule))?;
    let finish = Instant::now();
    db.sync()?;
    let final_sync_s = finish.elapsed().as_secs_f64();
    let flushing = Instant::now();
    db.flush()?;
    let final_flush_s = flushing.elapsed().as_secs_f64();
    let drain = Instant::now();
    let mut settled = db.settled()?;
    while !settled && drain.elapsed().as_secs_f64() < s.drain_seconds {
        std::thread::sleep(Duration::from_millis(c.sample_ms.min(1000)));
        reporter.sample(s, &db, "drain", &Metrics::default(), 1.0)?;
        settled = db.settled()?;
    }
    let final_sample = reporter.sample(s, &db, "final", &Metrics::default(), 1.0)?;
    let final_sync_flush_drain_s = finish.elapsed().as_secs_f64();
    let close = Instant::now();
    drop(db);
    let close_s = close.elapsed().as_secs_f64();
    let mut accounting_sample = final_sample.clone();
    accounting_sample["resources"] = telemetry::resources();
    let verification = Instant::now();
    let verified_keys = if s.verify {
        Some(verify(s, &stages, &partitions, &value)?)
    } else {
        None
    };
    let summary = json!({"engine":s.engine,"profile":s.profile,"elapsed_s":seconds,"stages":summaries,"metrics":telemetry::metrics(&total,seconds),
        "amplification":reporter.amplification(&accounting_sample,total.app_bytes),"final_sync_s":final_sync_s,"final_flush_s":final_flush_s,"durable_completion_ops_s":(total.reads.count+total.writes.count) as f64/(seconds+final_sync_s),"final_sync_flush_drain_s":final_sync_flush_drain_s,"close_s":close_s,"maintenance_settled":settled,
        "peak_sampled_rss":reporter.peak_rss,"peak_sampled_fds":reporter.peak_fds,"max_l0":reporter.max_l0,"db_bytes":final_sample["db_bytes"],
        "verification":{"enabled":s.verify,"verified_keys":verified_keys,"elapsed_s":verification.elapsed().as_secs_f64(),"kind":"close/reopen/full ordered scan, exact key set, values and selected point reads"}});
    telemetry::write_json(&c.output.join("summary.json"), &summary)?;
    println!("{}", summary);
    Ok(())
}
fn verify(
    s: &Settings,
    stages: &[workload::Stage],
    partitions: &[Vec<u64>],
    value: &[u8],
) -> io::Result<u64> {
    let c = &s.workload;
    let len = usize::try_from(c.keys.div_ceil(8)).map_err(io::Error::other)?;
    let mut expected = Vec::new();
    expected.try_reserve_exact(len).map_err(io::Error::other)?;
    expected.resize(len, 0u8);
    let mut mark = |key: u64| {
        expected[(key / 8) as usize] |= 1 << (key % 8);
    };
    for key in 0..c.preload_keys {
        mark(key);
    }
    for (index, (st, parts)) in stages.iter().zip(partitions).enumerate() {
        if st.reads == 100 {
            continue;
        }
        for (thread, &n) in parts.iter().enumerate() {
            for local in 0..n {
                let id = local
                    .checked_mul(st.threads as u64)
                    .and_then(|id| id.checked_add(thread as u64))
                    .ok_or_else(|| io::Error::other("verification ID overflow"))?;
                let (read, key) = workload::operation(c, st, index as u64, id);
                if !read {
                    mark(u64::from_be_bytes(key));
                }
            }
        }
    }
    let expected_count: u64 = expected.iter().map(|b| b.count_ones() as u64).sum();
    let mut previous = None;
    let mut count = 0u64;
    let mut check = |key: &[u8], got: &[u8]| -> io::Result<bool> {
        let key: [u8; 8] = key
            .try_into()
            .map_err(|_| io::Error::other("wrong key width"))?;
        let k = u64::from_be_bytes(key);
        if k >= c.keys
            || previous.is_some_and(|p| p >= k)
            || expected[(k / 8) as usize] & (1 << (k % 8)) == 0
            || got != value
        {
            return Err(io::Error::other(format!("reopen scan mismatch at key {k}")));
        }
        let point_check = count.is_multiple_of(4096);
        previous = Some(k);
        count += 1;
        Ok(point_check)
    };
    // Stream through an exclusive reopened owner; never collect the dataset in RAM.
    if s.engine == "kiban" {
        let db = kiban::db::Kiban::open_with_options(&c.db, c.options.clone())
            .map_err(io::Error::other)?;
        for entry in db.iter() {
            let (key, got) = entry.map_err(io::Error::other)?;
            if check(&key, &got)?
                && db.get(&key).map_err(io::Error::other)?.as_deref() != Some(value)
            {
                return Err(io::Error::other("reopen point/scan mismatch"));
            }
        }
    } else {
        let engine = Engine::open(s)?;
        let Engine::Rocks { db, .. } = engine else {
            return Err(io::Error::other("incorrect verifier adapter"));
        };
        for entry in db.iterator(rocksdb::IteratorMode::Start) {
            let (key, got) = entry.map_err(io::Error::other)?;
            if check(&key, &got)?
                && db.get(&key).map_err(io::Error::other)?.as_deref() != Some(value)
            {
                return Err(io::Error::other("reopen point/scan mismatch"));
            }
        }
    }
    if count != expected_count {
        return Err(io::Error::other(format!(
            "missing reopened keys: expected {expected_count}, found {count}"
        )));
    }
    Ok(count)
}
