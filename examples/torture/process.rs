use super::{
    config::Config,
    model::{self, Allowed, Model},
    telemetry, workload,
};
use kiban::db::{Kiban, SharedKiban};
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn mutation(c: &Config, id: u64) -> (Vec<u8>, Option<Vec<u8>>) {
    let (k, v) = model::mutation(c.seed, id, c.keys);
    // Disjoint per-thread key partitions make each thread's mutation order
    // reproducible despite concurrent engine-lock acquisition order.
    let thread = id % c.threads as u64;
    let slots = c.keys / c.threads as u64;
    let key = (u64::from_be_bytes(k.try_into().unwrap()) % slots) * c.threads as u64 + thread;
    (key.to_be_bytes().to_vec(), v)
}
fn round(c: &Config, db: &SharedKiban, start: u64, count: u64) -> io::Result<()> {
    let snap = if c.snapshots {
        Some(db.snapshot().map_err(io::Error::other)?)
    } else {
        None
    };
    let before = if let Some(snap) = &snap {
        snap.scan().map_err(io::Error::other)?
    } else {
        Vec::new()
    };
    std::thread::scope(|scope| -> io::Result<()> {
        let mut handles = Vec::new();
        for thread in 0..c.threads {
            handles.push(scope.spawn(move || -> io::Result<()> {
                for offset in (thread as u64..count).step_by(c.threads) {
                    let (k, v) = mutation(c, start + offset);
                    match v {
                        Some(v) => db.put(k, v)?,
                        None => db.delete(k)?,
                    }
                }
                Ok(())
            }));
        }
        for h in handles {
            h.join()
                .map_err(|_| io::Error::other("child writer panicked"))??;
        }
        Ok(())
    })?;
    if let Some(snap) = snap {
        if snap.scan().map_err(io::Error::other)? != before {
            return Err(io::Error::other(
                "shared snapshot changed during concurrent mutation",
            ));
        }
        for (k, v) in before {
            if snap.get(k).map_err(io::Error::other)? != Some(v) {
                return Err(io::Error::other("shared snapshot get/scan mismatch"));
            }
        }
    }
    Ok(())
}
// Internal child mode is selected by environment, keeping the public CLI strict.
pub fn child(c: &Config, cycle: u64) -> io::Result<()> {
    let db = SharedKiban::open_with_options(&c.db, model::options()).map_err(io::Error::other)?;
    let batch = c
        .sync_every
        .checked_mul(c.threads as u64)
        .ok_or_else(|| io::Error::other("round size overflow"))?;
    let mut offset = 0u64;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        let count = batch.min(c.operations - offset);
        if count == 0 {
            return Err(io::Error::other("parent requested beyond cycle budget"));
        }
        let start = cycle * c.operations + offset;
        round(c, &db, start, count)?;
        db.sync()?;
        if let Some(e) = db.maintenance_error() {
            return Err(io::Error::other(e));
        }
        println!("ACK {start} {count}");
        io::stdout().flush()?;
        offset += count;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        match line.trim() {
            "NEXT" => (),
            "TAIL" => {
                let tail_count = batch.min(c.operations - offset);
                round(c, &db, cycle * c.operations + offset, tail_count)?;
                // Keep the process alive, with no final sync or orderly drop.
                loop {
                    std::thread::park();
                }
            }
            _ => return Err(io::Error::other("invalid parent protocol")),
        }
    }
}
pub fn run(c: &Config) -> io::Result<()> {
    if c.keys < c.threads as u64 {
        return Err(io::Error::other("process mode requires keys >= threads"));
    }
    let batch = c
        .sync_every
        .checked_mul(c.threads as u64)
        .ok_or_else(|| io::Error::other("round size overflow"))?;
    let rounds = c.operations.div_ceil(batch);
    let mut floor = Model::new();
    let mut out = telemetry::create(&c.output.join("cycles.csv"))?;
    writeln!(
        out,
        "cycle,acknowledged_ops,tail_budget,kill_delay_ms,reopen_s,verify_s,live_keys,verification"
    )?;
    for cycle in 0..c.cycles {
        let target_rounds = 1 + workload::mix(c.seed ^ cycle) % rounds;
        let mut cmd = Command::new(std::env::current_exe()?);
        cmd.args(std::env::args().skip(1))
            .env("KIBAN_TORTURE_CHILD", cycle.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        let mut child = cmd.spawn()?;
        // Always kill/reap even if verification or protocol handling fails.
        let result = (|| -> io::Result<(u64, u64, u64, Allowed)> {
            let mut input = child.stdin.take().unwrap();
            let mut output = io::BufReader::new(child.stdout.take().unwrap());
            let mut acknowledged = 0u64;
            for r in 0..target_rounds {
                let mut line = String::new();
                output.read_line(&mut line)?;
                let p: Vec<_> = line.split_whitespace().collect();
                if p.len() != 3 || p[0] != "ACK" {
                    return Err(io::Error::other(format!(
                        "child did not acknowledge durable round: {line}"
                    )));
                }
                let start: u64 = p[1].parse().map_err(io::Error::other)?;
                let count: u64 = p[2].parse().map_err(io::Error::other)?;
                if start != cycle * c.operations + acknowledged
                    || count != batch.min(c.operations - acknowledged)
                {
                    return Err(io::Error::other("invalid ACK range"));
                }
                for id in start..start + count {
                    let (k, v) = mutation(c, id);
                    model::apply(&mut floor, k, v);
                }
                acknowledged += count;
                if r + 1 < target_rounds {
                    writeln!(input, "NEXT")?;
                    input.flush()?;
                }
            }
            let tail = batch.min(c.operations - acknowledged);
            let mut allowed = Allowed::new();
            for id in
                cycle * c.operations + acknowledged..cycle * c.operations + acknowledged + tail
            {
                let (k, v) = mutation(c, id);
                allowed.entry(k).or_default().push(v);
            }
            // ACK receipt is in the parent before any unacknowledged tail starts.
            // SIGKILL may land during a writer, flush, compaction, or idle tail wait.
            if tail > 0 {
                writeln!(input, "TAIL")?;
                input.flush()?;
            }
            let delay = workload::mix(c.seed ^ cycle ^ 0xdead) % 11;
            std::thread::sleep(Duration::from_millis(delay));
            Ok((acknowledged, tail, delay, allowed))
        })();
        let kill_result = child.kill();
        let status = child.wait()?;
        kill_result?;
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if status.signal() != Some(9) {
                return Err(io::Error::other(format!(
                    "child exited unexpectedly: {status}"
                )));
            }
        }
        let (ack, tail, delay, allowed) = result?;
        let t = Instant::now();
        let db = Kiban::open_with_options(&c.db, model::options()).map_err(io::Error::other)?;
        let reopen = t.elapsed().as_secs_f64();
        let t = Instant::now();
        floor = model::verify(&db, &floor, &allowed, c.keys)
            .map_err(|e| io::Error::other(format!("process cycle={cycle}: {e}")))?;
        let verify = t.elapsed().as_secs_f64();
        drop(db);
        writeln!(
            out,
            "{cycle},{ack},{tail},{delay},{reopen},{verify},{},durable-floor-and-attempt-band",
            floor.len()
        )?;
        if cycle.is_multiple_of(25) {
            eprintln!("process recovery cycle {cycle}/{} verified", c.cycles);
        }
    }
    Ok(())
}
