//! Read-only cache-budget diagnosis. Original benchmark executable stays frozen.
#[path = "../../src/api.rs"]
mod api;
#[allow(dead_code)]
mod backend;
#[path = "../../src/embedded.rs"]
#[allow(dead_code)]
mod embedded;
#[path = "../../../../examples/scale/metrics.rs"]
#[allow(dead_code)]
mod metrics;
#[path = "../../src/model.rs"]
mod model;
#[path = "../../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;
#[path = "../../src/runner.rs"]
#[allow(dead_code)]
mod runner;

use api::{Backend, Result};
use kiban::db::{KibanOptions, SharedKiban};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Durable,
    Fast,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    backend: String,
    db: PathBuf,
    #[serde(default)]
    endpoint: String,
    n: u64,
    operations: u64,
    k: usize,
    seed: u64,
    output: PathBuf,
    verify_only: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(flatten)]
    suite: Config,
    cache_mib: Option<usize>,
}

fn kiban_options(mib: usize) -> KibanOptions {
    KibanOptions {
        block_cache_bytes: mib * 1024 * 1024,
        ..KibanOptions::default()
    }
}
fn open(o: &Options, path: &Path) -> Result<Box<dyn Backend>> {
    match o.suite.backend.as_str() {
        "kiban" => Ok(Box::new(backend::Embedded::with_options(
            path,
            kiban_options(o.cache_mib.ok_or("missing cache_mib")?),
        )?)),
        "lmdb" => {
            if o.cache_mib.is_some() {
                return Err("LMDB does not take a Kiban cache budget".into());
            }
            Ok(Box::new(backend::Lmdb::open(path, Mode::Fast)?.0))
        }
        _ => Err("cache diagnosis supports kiban/lmdb".into()),
    }
}
fn proc_memory() -> Result<Value> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let mut values = serde_json::Map::new();
    for line in status.lines() {
        let (name, value) = line.split_once(':').ok_or("invalid proc status")?;
        if ["VmRSS", "VmHWM", "RssAnon", "RssFile", "VmSwap"].contains(&name) {
            values.insert(
                name.into(),
                json!(
                    value
                        .split_whitespace()
                        .next()
                        .ok_or("missing proc memory")?
                        .parse::<u64>()?
                        * 1024
                ),
            );
        } else if name == "Cpus_allowed_list" {
            values.insert(name.into(), json!(value.trim()));
        }
    }
    Ok(Value::Object(values))
}
fn load_sample() -> Result<Value> {
    unsafe extern "C" {
        fn getloadavg(values: *mut f64, count: i32) -> i32;
    }
    let mut load = [0_f64; 3];
    // Three writable doubles; libc writes at most the requested count.
    if unsafe { getloadavg(load.as_mut_ptr(), 3) } != 3 {
        return Err("getloadavg failed".into());
    }
    Ok(
        json!({"epoch":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),"load":load}),
    )
}
fn layout(path: &Path) -> Result<Value> {
    let m = kiban::manifest::Manifest::load(path)?.ok_or("missing manifest")?;
    let mut levels: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
    for table in m.tables {
        levels.entry(table.level).or_default().push(table.number);
    }
    Ok(json!(levels))
}
fn verify(b: &mut dyn Backend, c: &Config) -> Result<()> {
    runner::verify_initial(b, c)
}
fn execute(b: &mut dyn Backend, c: &Config, out: &mut impl Write) -> Result<()> {
    let recent: Vec<_> = (0..model::AGENTS as u32)
        .map(|agent| model::recent_ids(c.n, agent, c.k))
        .collect();
    for (stage, name) in [
        (1_u64, "recall-by-id"),
        (2, "recent-for-agent"),
        (4, "mixed-key-reads-only"),
    ] {
        let mut h = metrics::Histogram::default();
        let writes = metrics::Histogram::default();
        let mut ns = 0;
        let started = Instant::now();
        for op in 0..c.operations {
            let id = random::mix(c.seed ^ random::mix(stage) ^ op) % c.n;
            let tick = Instant::now();
            let (one, many) = if stage == 2 {
                (None, Some(b.recent((id % model::AGENTS) as u32, c.k)?))
            } else {
                (Some(b.get(id)?), None)
            };
            let elapsed = tick.elapsed().as_nanos();
            ns += elapsed;
            h.record(u64::try_from(elapsed)?);
            if let Some(actual) = one {
                runner::check(&actual, id, 0, c.seed)?;
            }
            if let Some(actual) = many {
                let ids = &recent[(id % model::AGENTS) as usize];
                if actual.len() != ids.len() {
                    return Err("recent length mismatch".into());
                }
                for (record, id) in actual.iter().zip(ids) {
                    runner::check(record, *id, 0, c.seed)?;
                }
            }
        }
        let mut row = runner::row(name, &h, &h, &writes, started, ns, 0);
        row["memory_after_stage"] = proc_memory()?;
        runner::emit(out, &row)?;
    }
    Ok(())
}
fn cache_stats(db: &SharedKiban) -> Result<Value> {
    let s = db.stats()?;
    let c = s.block_cache;
    Ok(
        json!({"hits":c.hits,"misses":c.misses,"evictions":c.evictions,"resident_bytes":c.resident_bytes,"capacity_bytes":c.capacity_bytes,
        "resident_entries":c.resident_entries,"table_files":format!("{:?}",s.table_files),"levels":s.levels.iter().map(|l|json!({"level":l.level,"tables":l.tables,"bytes":l.bytes})).collect::<Vec<_>>()}),
    )
}
fn shared_counter_diagnostic(o: &Options) -> Result<Value> {
    let c = &o.suite;
    let original = layout(&c.db)?;
    let db =
        SharedKiban::open_with_options(&c.db, kiban_options(o.cache_mib.ok_or("missing cache")?))?;
    // Match the timed direct path's full sequential read-back warmup, on a fresh cache.
    for id in 0..c.n {
        let bytes = db
            .get(model::key(id))?
            .ok_or("missing shared warmup value")?;
        runner::check(&serde_json::from_slice(&bytes)?, id, 0, c.seed)?;
    }
    for agent in 0..model::AGENTS as u32 {
        let prefix = format!("agent/{agent:04}/memory/");
        let end = format!("agent/{agent:04}/memory0");
        for (entry, id) in db
            .snapshot()?
            .range(prefix.as_bytes(), end.as_bytes())
            .take(c.k)
            .zip(model::recent_ids(c.n, agent, c.k))
        {
            let (_, bytes) = entry?;
            runner::check(&serde_json::from_slice(&bytes)?, id, 0, c.seed)?;
        }
    }
    let mut stages = Vec::new();
    for (stage, name) in [
        (1_u64, "recall-by-id"),
        (2, "recent-for-agent"),
        (4, "mixed-key-reads-only"),
    ] {
        let before = cache_stats(&db)?;
        for op in 0..c.operations {
            let id = random::mix(c.seed ^ random::mix(stage) ^ op) % c.n;
            if stage == 2 {
                let agent = (id % model::AGENTS) as u32;
                let prefix = format!("agent/{agent:04}/memory/");
                let end = format!("agent/{agent:04}/memory0");
                let actual = db
                    .snapshot()?
                    .range(prefix.as_bytes(), end.as_bytes())
                    .take(c.k)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let ids = model::recent_ids(c.n, agent, c.k);
                if actual.len() != ids.len() {
                    return Err("shared recent count mismatch".into());
                }
                for ((_, bytes), id) in actual.iter().zip(ids) {
                    runner::check(&serde_json::from_slice(bytes)?, id, 0, c.seed)?;
                }
            } else {
                let bytes = db.get(model::key(id))?.ok_or("missing shared value")?;
                runner::check(&serde_json::from_slice(&bytes)?, id, 0, c.seed)?;
            }
        }
        let after = cache_stats(&db)?;
        let hits = after["hits"].as_u64().ok_or("missing hits")?
            - before["hits"].as_u64().ok_or("missing hits")?;
        let misses = after["misses"].as_u64().ok_or("missing misses")?
            - before["misses"].as_u64().ok_or("missing misses")?;
        stages.push(json!({"workload":name,"hits":hits,"misses":misses,"hit_rate":if hits+misses==0{None}else{Some(hits as f64/(hits+misses) as f64)},"before":before,"after":after}));
    }
    db.close()?;
    if layout(&c.db)? != original {
        return Err("shared counter diagnostic changed SST layout".into());
    }
    Ok(
        json!({"scope":"Excluded SharedKiban counter replay, not timed direct Kiban calls; identical keys, preload verification and unchanged SST layout","stages":stages}),
    )
}
fn run() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: kiban-cachebench CONFIG.json")?;
    let o: Options = serde_json::from_slice(&std::fs::read(path)?)?;
    let c = &o.suite;
    if c.n < 64
        || c.operations < 1
        || !(1..=1000).contains(&c.k)
        || !matches!(o.cache_mib, None | Some(32 | 256 | 512 | 1024))
    {
        return Err("invalid cache diagnosis options".into());
    }
    let mut preload = json!({});
    if !c.verify_only {
        if c.db.exists() {
            return Err("diagnosis requires fresh database".into());
        }
        let warm = c.db.with_extension("warmup");
        if warm.exists() {
            return Err("warmup database exists".into());
        }
        let w = Config {
            db: warm.clone(),
            n: 100,
            operations: 100,
            ..c.clone()
        };
        let mut b = open(&o, &warm)?;
        b.bulk_load(w.n, w.seed)?;
        b.finish()?;
        verify(b.as_mut(), &w)?;
        execute(b.as_mut(), &w, &mut std::io::sink())?;
        drop(b);
        std::fs::remove_dir_all(warm)?;
        let start = Instant::now();
        let mut b = open(&o, &c.db)?;
        b.bulk_load(c.n, c.seed)?;
        b.finish()?;
        verify(b.as_mut(), c)?;
        drop(b);
        preload = json!({"base_records":c.n,"load_sync_verify_close_seconds":start.elapsed().as_secs_f64()});
    }
    let mut b = open(&o, &c.db)?;
    verify(b.as_mut(), c)?;
    let pre_layout = if c.backend == "kiban" {
        layout(&c.db)?
    } else {
        Value::Null
    };
    let mut out = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&c.output)?,
    );
    if c.verify_only {
        runner::emit(
            &mut out,
            &json!({"type":"reopen_verified","verified_records":c.n}),
        )?;
        return Ok(());
    }
    let start = load_sample()?;
    let before = proc_memory()?;
    execute(b.as_mut(), c, &mut out)?;
    let end = load_sample()?;
    let after = proc_memory()?;
    verify(b.as_mut(), c)?;
    let after_verification = proc_memory()?;
    drop(b);
    if c.backend == "kiban" && layout(&c.db)? != pre_layout {
        return Err("SST layout changed during read-only suite".into());
    }
    runner::emit(
        &mut out,
        &json!({"type":"complete","verified_records":c.n,"timed_writes":0}),
    )?;
    out.flush()?;
    let stats = if c.backend == "kiban" {
        shared_counter_diagnostic(&o)?
    } else {
        Value::Null
    };
    let defaults = kiban_options(o.cache_mib.unwrap_or(32));
    let resource = json!({"measurement_start_epoch":start["epoch"],"measurement_end_epoch":end["epoch"],"boundary_load_samples":[start,end],"memory_before_timing":before,"memory_after_timing":after,"memory_after_final_verification":after_verification,"preload":preload,"sst_layout_after_preload":pre_layout,"cache_counters":stats,"settings":{"mode":"fast","block_cache_mib":o.cache_mib,"range_cache_bytes":defaults.range_cache_bytes,"range_readahead_bytes":defaults.range_readahead_bytes,"max_open_table_files":defaults.max_open_table_files,"mmap_max_level":defaults.mmap_max_level,"lmdb_flags_bits":if c.backend=="lmdb"{Some(327680)}else{None}},"rss_scope":"Direct benchmark process before SharedKiban counter replay; includes untimed preload; no isolated heap claim"});
    std::fs::write(
        c.output.with_extension("resources.json"),
        serde_json::to_vec_pretty(&resource)?,
    )?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("INVALID: {error}");
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_suite_verifies_real_values_and_preserves_revisions() -> Result<()> {
        for (backend, cache_mib) in [("kiban", Some(32)), ("kiban", Some(1024)), ("lmdb", None)] {
            let path = std::env::temp_dir().join(format!(
                "cachebench-{}-{backend}-{cache_mib:?}",
                std::process::id()
            ));
            let c = Config {
                backend: backend.into(),
                db: path.clone(),
                endpoint: String::new(),
                n: 128,
                operations: 100,
                k: 3,
                seed: 20261016,
                output: path.with_extension("unused"),
                verify_only: false,
            };
            let o = Options {
                suite: c.clone(),
                cache_mib,
            };
            let mut b = open(&o, &path)?;
            b.bulk_load(c.n, c.seed)?;
            b.finish()?;
            verify(b.as_mut(), &c)?;
            let mut out = Vec::new();
            execute(b.as_mut(), &c, &mut out)?;
            let rows = String::from_utf8(out)?
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert_eq!(rows.len(), 3);
            for row in rows {
                assert_eq!(row["writes"]["count"], 0);
                assert_eq!(row["reads"]["count"], 100);
            }
            drop(b);
            let mut b = open(&o, &path)?;
            verify(b.as_mut(), &c)?;
            b.put(&model::Memory::new(5, 99, c.seed), false)?;
            assert!(verify(b.as_mut(), &c).is_err());
            drop(b);
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}
