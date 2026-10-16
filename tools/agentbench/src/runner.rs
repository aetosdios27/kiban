use crate::{
    Config,
    backend::{Backend, Helix, Result},
    metrics::Histogram,
    model::{AGENTS, Memory, recent_ids},
    random::mix,
};
use serde_json::json;
use std::{io::Write, time::Instant};

fn emit(output: &mut impl Write, row: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *output, row)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn check(actual: &Memory, id: u64, revision: u64, seed: u64) -> Result<()> {
    if *actual != Memory::new(id, revision, seed) {
        return Err(format!("value mismatch for id {id}, revision {revision}: {actual:?}").into());
    }
    Ok(())
}

fn fields(h: &Histogram) -> serde_json::Value {
    json!({"count": h.count, "p50_ns": h.percentile(50, 100),
        "p99_ns": h.percentile(99, 100), "p999_ns": h.percentile(999, 1000), "max_ns": h.max})
}

fn row(
    workload: &str,
    h: &Histogram,
    reads: &Histogram,
    writes: &Histogram,
    started: Instant,
    call_ns: u128,
    verification_ns: u128,
) -> serde_json::Value {
    let wall_seconds = started.elapsed().as_secs_f64();
    json!({"type": "stage", "workload": workload, "latency": fields(h),
        "reads": fields(reads), "writes": fields(writes), "errors": 0,
        "wall_seconds": wall_seconds, "verified_ops_s": h.count as f64 / wall_seconds,
        "call_seconds": call_ns as f64 / 1e9,
        "call_ops_s": h.count as f64 / (call_ns as f64 / 1e9),
        "write_readback_seconds": verification_ns as f64 / 1e9})
}

fn id_for(c: &Config, stage: u64, op: u64) -> u64 {
    mix(c.seed ^ mix(stage) ^ op) % c.n
}

pub fn suite(b: &mut dyn Backend, c: &Config, out: &mut impl Write) -> Result<()> {
    b.prepare()?;
    let mut revisions = vec![0u64; c.n as usize];
    let recent: Vec<_> = (0..AGENTS as u32)
        .map(|a| recent_ids(c.n, a, c.k))
        .collect();
    for (stage, workload) in [
        "remember",
        "recall-by-id",
        "recent-for-agent",
        "update",
        "mixed-90-10",
    ]
    .iter()
    .enumerate()
    {
        let count = if stage == 0 { c.n } else { c.operations };
        let mut all = Histogram::default();
        let mut reads = Histogram::default();
        let mut writes = Histogram::default();
        let mut call_ns = 0;
        let mut verification_ns = 0;
        let started = Instant::now();
        for op in 0..count {
            let id = if stage == 0 {
                op
            } else {
                id_for(c, stage as u64, op)
            };
            let write = stage == 0 || stage == 3 || (stage == 4 && op % 10 == 9);
            let revision = revisions[id as usize] + u64::from(stage != 0 && write);
            let memory = Memory::new(id, revision, c.seed);
            let agent = (id % AGENTS) as u32;
            let start = Instant::now();
            let (one, many) = if write {
                b.put(&memory, stage == 0)?;
                (None, None)
            } else if stage == 2 {
                (None, Some(b.recent(agent, c.k)?))
            } else {
                (Some(b.get(id)?), None)
            };
            let elapsed = start.elapsed().as_nanos();
            call_ns += elapsed;
            let ns = u64::try_from(elapsed)?;
            all.record(ns);
            if write {
                writes.record(ns);
                revisions[id as usize] = revision;
                let verification = Instant::now();
                check(&b.get(id)?, id, revision, c.seed)?;
                verification_ns += verification.elapsed().as_nanos();
            } else {
                reads.record(ns);
            }
            if let Some(actual) = one {
                check(&actual, id, revision, c.seed)?;
            }
            if let Some(actual) = many {
                let expected_ids = &recent[agent as usize];
                if actual.len() != expected_ids.len() {
                    return Err("recent count mismatch".into());
                }
                for (memory, &expected_id) in actual.iter().zip(expected_ids) {
                    check(memory, expected_id, revisions[expected_id as usize], c.seed)?;
                }
            }
        }
        emit(
            out,
            &row(
                workload,
                &all,
                &reads,
                &writes,
                started,
                call_ns,
                verification_ns,
            ),
        )?;
        eprintln!("{}: {workload} {count} verified calls", c.backend);
    }
    let start = Instant::now();
    b.finish()?;
    verify(b, c, &revisions)?;
    emit(
        out,
        &json!({"type": "complete", "verified_records": c.n, "end_sync_flush_verify_seconds": start.elapsed().as_secs_f64()}),
    )
}

fn verify(b: &mut dyn Backend, c: &Config, revisions: &[u64]) -> Result<()> {
    for id in 0..c.n {
        check(&b.get(id)?, id, revisions[id as usize], c.seed)?;
    }
    for agent in 0..AGENTS as u32 {
        let expected = recent_ids(c.n, agent, c.k);
        let actual = b.recent(agent, c.k)?;
        if actual.len() != expected.len() {
            return Err("final recent count mismatch".into());
        }
        for (memory, id) in actual.iter().zip(expected) {
            check(memory, id, revisions[id as usize], c.seed)?;
        }
    }
    Ok(())
}

pub fn verify_persisted(b: &mut dyn Backend, c: &Config, out: &mut impl Write) -> Result<()> {
    let mut revisions = vec![0; c.n as usize];
    for stage in [3, 4] {
        for op in 0..c.operations {
            if stage == 3 || op % 10 == 9 {
                revisions[id_for(c, stage, op) as usize] += 1;
            }
        }
    }
    verify(b, c, &revisions)?;
    emit(
        out,
        &json!({"type": "reopen_verified", "verified_records": c.n}),
    )
}

pub fn floor(b: &Helix, c: &Config, out: &mut impl Write) -> Result<()> {
    b.noop()?; // Establish this client’s persistent connection outside timing.
    let started = Instant::now();
    let mut h = Histogram::default();
    let mut call_ns = 0;
    for _ in 0..c.operations {
        let start = Instant::now();
        b.noop()?;
        let elapsed = start.elapsed().as_nanos();
        call_ns += elapsed;
        h.record(u64::try_from(elapsed)?);
    }
    emit(
        out,
        &row(
            "transport-floor",
            &h,
            &h,
            &Histogram::default(),
            started,
            call_ns,
            0,
        ),
    )?;
    emit(
        out,
        &json!({"type": "complete", "verified_responses": c.operations}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Embedded;

    #[test]
    fn verifies_real_database_across_reopen_and_rejects_corruption() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "agentbench-test-{}-{}",
            std::process::id(),
            mix(17)
        ));
        std::fs::create_dir(&path)?;
        let c = Config {
            backend: "kiban".into(),
            db: path.join("db"),
            endpoint: String::new(),
            n: 128,
            operations: 100,
            k: 3,
            seed: 17,
            output: path.join("unused"),
            verify_only: false,
        };
        let mut out = Vec::new();
        {
            let mut b = Embedded::open(&c.db)?;
            suite(&mut b, &c, &mut out)?;
        }
        let mut b = Embedded::open(&c.db)?;
        verify_persisted(&mut b, &c, &mut out)?;
        let wrong = Memory::new(5, 999, c.seed);
        b.put(&wrong, false)?;
        assert!(verify_persisted(&mut b, &c, &mut Vec::new()).is_err());
        drop(b);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
