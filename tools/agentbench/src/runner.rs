use crate::{
    Config,
    backend::{Backend, Result},
    metrics::Histogram,
    model::{AGENTS, Memory, recent_ids},
    random::mix,
};
use serde_json::json;
use std::{io::Write, time::Instant};

pub(crate) fn emit(output: &mut impl Write, row: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *output, row)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

pub(crate) fn check(actual: &Memory, id: u64, revision: u64, seed: u64) -> Result<()> {
    if *actual != Memory::new(id, revision, seed) {
        return Err(format!("value mismatch for id {id}, revision {revision}: {actual:?}").into());
    }
    Ok(())
}

fn fields(h: &Histogram) -> serde_json::Value {
    json!({"count": h.count, "p50_ns": h.percentile(50, 100),
        "p99_ns": h.percentile(99, 100), "p999_ns": h.percentile(999, 1000), "max_ns": h.max})
}

pub(crate) fn row(
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

#[derive(Clone, Copy)]
pub struct Plan {
    pub base_population: u64,
    pub insertions: u64,
    pub write_operations: u64,
}

pub fn suite(b: &mut dyn Backend, c: &Config, out: &mut impl Write) -> Result<()> {
    suite_plan(
        b,
        c,
        Plan {
            base_population: 0,
            insertions: c.n,
            write_operations: c.operations,
        },
        out,
    )
}

pub fn suite_plan(b: &mut dyn Backend, c: &Config, plan: Plan, out: &mut impl Write) -> Result<()> {
    if plan.base_population == 0 {
        b.prepare()?;
    }
    let population = plan.base_population + plan.insertions;
    let mut revisions = vec![0u64; population as usize];
    let recent: Vec<_> = (0..AGENTS as u32)
        .map(|a| recent_ids(population, a, c.k))
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
        let count = match stage {
            0 => plan.insertions,
            3 => plan.write_operations,
            _ => c.operations,
        };
        let mut all = Histogram::default();
        let mut reads = Histogram::default();
        let mut writes = Histogram::default();
        let mut call_ns = 0;
        let mut verification_ns = 0;
        let started = Instant::now();
        for op in 0..count {
            let id = if stage == 0 {
                plan.base_population + op
            } else {
                mix(c.seed ^ mix(stage as u64) ^ op) % population
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
            if (op + 1) % 10000 == 0 && op + 1 < count {
                eprintln!("{}: {workload} progress {}/{count}", c.backend, op + 1);
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
    verify(b, c, population, &revisions)?;
    emit(
        out,
        &json!({"type": "complete", "verified_records": population, "end_sync_flush_verify_seconds": start.elapsed().as_secs_f64()}),
    )
}

fn verify(b: &mut dyn Backend, c: &Config, population: u64, revisions: &[u64]) -> Result<()> {
    for id in 0..population {
        check(&b.get(id)?, id, revisions[id as usize], c.seed)?;
    }
    for agent in 0..AGENTS as u32 {
        let expected = recent_ids(population, agent, c.k);
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

#[allow(dead_code)] // Used only by the optional bulk-preloaded engine path.
pub fn verify_initial(b: &mut dyn Backend, c: &Config) -> Result<()> {
    verify(b, c, c.n, &vec![0; c.n as usize])
}

#[allow(dead_code)] // Original empty-database HTTP path; also exercised by shared tests.
pub fn verify_persisted(b: &mut dyn Backend, c: &Config, out: &mut impl Write) -> Result<()> {
    verify_persisted_plan(
        b,
        c,
        Plan {
            base_population: 0,
            insertions: c.n,
            write_operations: c.operations,
        },
        out,
    )
}

pub fn verify_persisted_plan(
    b: &mut dyn Backend,
    c: &Config,
    plan: Plan,
    out: &mut impl Write,
) -> Result<()> {
    let population = plan.base_population + plan.insertions;
    let mut revisions = vec![0; population as usize];
    for stage in [3, 4] {
        let count = if stage == 3 {
            plan.write_operations
        } else {
            c.operations
        };
        for op in 0..count {
            if stage == 3 || op % 10 == 9 {
                let id = mix(c.seed ^ mix(stage) ^ op) % population;
                revisions[id as usize] += 1;
            }
        }
    }
    verify(b, c, population, &revisions)?;
    emit(
        out,
        &json!({"type":"reopen_verified","verified_records":population}),
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
