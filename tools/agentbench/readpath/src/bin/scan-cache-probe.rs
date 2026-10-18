//! Cache/page diagnostics only; latency results use the frozen readbench binaries.
#[path = "../../../src/model.rs"]
#[allow(dead_code)]
mod model;
#[path = "../../../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;

use kiban::db::{KibanOptions, SharedKiban, SharedSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{error::Error, path::PathBuf};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Config {
    db: PathBuf,
    n: u64,
    seed: u64,
    cache_mib: Option<usize>,
    mmap_max_level: Option<u32>,
    operations: u64,
}

#[derive(Clone, Serialize)]
struct State {
    capacity_bytes: usize,
    resident_bytes: usize,
    resident_entries: usize,
    point_index_bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    minor_faults: u64,
    major_faults: u64,
    storage_read_bytes: u64,
}

fn state(db: &SharedKiban) -> Result<State> {
    let c = db.stats()?.block_cache;
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .ok_or("bad proc stat")?
        .1
        .split_whitespace()
        .collect();
    let io = std::fs::read_to_string("/proc/self/io")?;
    let read_bytes: u64 = io
        .lines()
        .find_map(|s| s.strip_prefix("read_bytes: "))
        .ok_or("missing read_bytes")?
        .trim()
        .parse()?;
    Ok(State {
        capacity_bytes: c.capacity_bytes,
        resident_bytes: c.resident_bytes,
        resident_entries: c.resident_entries,
        point_index_bytes: c.point_index_bytes,
        hits: c.hits,
        misses: c.misses,
        evictions: c.evictions,
        minor_faults: fields[7].parse()?,
        major_faults: fields[9].parse()?,
        storage_read_bytes: read_bytes,
    })
}

fn emit(phase: &str, operations: u64, before: &State, after: &State) {
    println!(
        "{}",
        json!({"phase":phase,"operations":operations,"state":after,
        "delta":{"hits":after.hits-before.hits,"misses":after.misses-before.misses,
        "evictions":after.evictions-before.evictions,
        "minor_faults":after.minor_faults-before.minor_faults,
        "major_faults":after.major_faults-before.major_faults,
        "storage_read_bytes":after.storage_read_bytes-before.storage_read_bytes}})
    );
}

fn verify(db: &SharedKiban, c: &Config) -> Result<()> {
    for id in 0..c.n {
        let raw = db.get(model::key(id))?.ok_or("missing record")?;
        let record: model::Memory = serde_json::from_slice(&raw)?;
        if record != model::Memory::new(id, 0, c.seed) {
            return Err(format!("wrong record {id}").into());
        }
    }
    Ok(())
}

fn recent(snapshot: &SharedSnapshot, agent: u32, expected: &[model::Memory]) -> Result<()> {
    let start = format!("agent/{agent:04}/memory/");
    let end = format!("agent/{agent:04}/memory0");
    let records = snapshot
        .range(start.as_bytes(), end.as_bytes())
        .take(10)
        .map(|item| -> Result<model::Memory> {
            let (_, value) = item?;
            Ok(serde_json::from_slice(&value)?)
        })
        .collect::<Result<Vec<_>>>()?;
    if records != expected {
        return Err("wrong latest-ten records".into());
    }
    Ok(())
}

fn main() -> Result<()> {
    let path = std::env::args_os().nth(1).ok_or("expected config path")?;
    let c: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let db = SharedKiban::open_with_options(
        &c.db,
        KibanOptions {
            point_index: true,
            block_cache_bytes: c
                .cache_mib
                .map_or_else(|| KibanOptions::default().block_cache_bytes, |n| n << 20),
            mmap_max_level: c.mmap_max_level,
            ..KibanOptions::default()
        },
    )?;
    verify(&db, &c)?;
    let before = state(&db)?;
    for op in 0..100000 {
        let id = random::mix(c.seed ^ random::mix(1) ^ op) % c.n;
        let raw = db.get(model::key(id))?.ok_or("missing point warmup")?;
        let record: model::Memory = serde_json::from_slice(&raw)?;
        if record != model::Memory::new(id, 0, c.seed) {
            return Err("wrong point warmup".into());
        }
    }
    let after = state(&db)?;
    emit("point-warmup", 100000, &before, &after);
    let snapshot = db.snapshot()?;
    let expected: Vec<Vec<_>> = (0..model::AGENTS as u32)
        .map(|agent| {
            model::recent_ids(c.n, agent, 10)
                .into_iter()
                .map(|id| model::Memory::new(id, 0, c.seed))
                .collect()
        })
        .collect();
    for agent in 0..model::AGENTS as u32 {
        recent(&snapshot, agent, &expected[agent as usize])?;
    }
    let warm = state(&db)?;
    emit("latest-first-64", 64, &after, &warm);
    for op in 0..c.operations {
        let id = random::mix(c.seed ^ random::mix(2) ^ op) % c.n;
        let agent = (id % model::AGENTS) as u32;
        recent(&snapshot, agent, &expected[agent as usize])?;
    }
    let done = state(&db)?;
    emit("latest-hot", c.operations, &warm, &done);
    verify(&db, &c)?;
    println!(
        "{}",
        json!({"verified_records":c.n,"verified_latest_calls":c.operations+model::AGENTS,"errors":0})
    );
    Ok(())
}
