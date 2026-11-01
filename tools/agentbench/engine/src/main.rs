#[path = "../../src/api.rs"]
mod api;
mod backend;
#[path = "../../src/embedded.rs"]
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
mod runner;

use api::{Backend, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Durable,
    Fast,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(flatten)]
    suite: Config,
    mode: Mode,
    write_operations: Option<u64>,
    #[serde(default = "warmup_count")]
    warmup: u64,
}

fn warmup_count() -> u64 {
    100
}

fn open(backend: &str, path: &Path, mode: Mode) -> Result<(Box<dyn Backend>, serde_json::Value)> {
    match backend {
        "kiban" => {
            let b = if mode == Mode::Durable {
                backend::Embedded::open(path)?
            } else {
                backend::Embedded::with_durability(path, false)?
            };
            Ok((
                Box::new(b),
                serde_json::json!({"per_write_sync":mode == Mode::Durable,
                "durable_call":"put + sync_data on WAL", "fast_call":"put; default maintenance can still sync"}),
            ))
        }
        "helix" => {
            let (b, details) = backend::Helix::open(path, mode)?;
            Ok((Box::new(b), details))
        }
        "lmdb" => {
            let (b, details) = backend::Lmdb::open(path, mode)?;
            Ok((Box::new(b), details))
        }
        _ => Err("unknown engine (use kiban, helix, lmdb)".into()),
    }
}

fn resources(details: serde_json::Value) -> Result<serde_json::Value> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let kib = |field: &str| -> Result<u64> {
        let line = status
            .lines()
            .find(|line| line.starts_with(field))
            .ok_or("missing proc field")?;
        Ok(line
            .split_whitespace()
            .nth(1)
            .ok_or("invalid proc field")?
            .parse::<u64>()?
            * 1024)
    };
    let affinity = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))
        .ok_or("missing affinity")?;
    Ok(
        serde_json::json!({"rss_bytes":kib("VmRSS:")?,"peak_rss_bytes":kib("VmHWM:")?,
        "cpus_allowed_list":affinity,"engine_settings":details,
        "proc_io":std::fs::read_to_string("/proc/self/io")?,
        "scope":"whole engine process including oracle, warmup and final verification; sampled before close"}),
    )
}

fn bulk_load(b: &mut dyn Backend, c: &Config) -> Result<serde_json::Value> {
    let started = std::time::Instant::now();
    b.prepare()?;
    b.bulk_load(c.n, c.seed)?;
    b.finish()?;
    let load_seconds = started.elapsed().as_secs_f64();
    let verify = std::time::Instant::now();
    runner::verify_initial(b, c)?;
    Ok(
        serde_json::json!({"base_records":c.n,"kiban_batch_records":10000,"lmdb_helix_bulk_transactions":1,"load_and_final_sync_seconds":load_seconds,
        "initial_verification_seconds":verify.elapsed().as_secs_f64(),"durability":"relaxed bulk batches; explicit sync at end, all outside measurement"}),
    )
}

fn load_sample() -> Result<serde_json::Value> {
    unsafe extern "C" {
        fn getloadavg(values: *mut f64, count: i32) -> i32;
    }
    let mut load = [0.0_f64; 3];
    // Exactly three writable doubles; libc writes at most the requested count.
    if unsafe { getloadavg(load.as_mut_ptr(), 3) } != 3 {
        return Err("getloadavg failed".into());
    }
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    Ok(serde_json::json!({"epoch":epoch,"load":load}))
}

fn run() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: kiban-enginebench CONFIG.json")?;
    let options: Options = serde_json::from_slice(&std::fs::read(path)?)?;
    let c = &options.suite;
    let write_operations = options
        .write_operations
        .unwrap_or(if options.mode == Mode::Durable {
            c.operations.min(10000)
        } else {
            c.operations
        });
    if write_operations == 0
        || write_operations > c.operations
        || (options.mode == Mode::Durable && write_operations > 10000)
    {
        return Err("invalid write operation cap".into());
    }
    let plan = runner::Plan {
        base_population: c.n,
        insertions: write_operations,
        write_operations,
    };
    if !(64..=100_000_000).contains(&c.n)
        || c.operations < 1
        || !(1..=1000).contains(&c.k)
        || options.warmup < 64
    {
        return Err("invalid size, operations, k or warmup".into());
    }
    let mut preload = serde_json::Value::Null;
    if !c.verify_only {
        if c.db.exists() {
            return Err("benchmark database must be fresh".into());
        }
        let warmup = Config {
            n: options.warmup,
            operations: options.warmup,
            db: c.db.with_extension("warmup"),
            ..c.clone()
        };
        if warmup.db.exists() {
            return Err("warmup database must be fresh".into());
        }
        let (mut b, _) = open(&c.backend, &warmup.db, options.mode)?;
        runner::suite(b.as_mut(), &warmup, &mut std::io::sink())?;
        drop(b);
        std::fs::remove_dir_all(&warmup.db)?;
        let (mut bulk, _) = open(&c.backend, &c.db, Mode::Fast)?;
        preload = bulk_load(bulk.as_mut(), c)?;
        drop(bulk);
    }
    let (mut b, details) = open(&c.backend, &c.db, options.mode)?;
    if !c.verify_only {
        let verify = std::time::Instant::now();
        runner::verify_initial(b.as_mut(), c)?;
        preload["reopen_verification_seconds"] = serde_json::json!(verify.elapsed().as_secs_f64());
    }
    let mut out = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&c.output)?,
    );
    let start_load = load_sample()?;
    if c.verify_only {
        runner::verify_persisted_plan(b.as_mut(), c, plan, &mut out)?;
    } else {
        runner::suite_plan(b.as_mut(), c, plan, &mut out)?;
    }
    let end_load = load_sample()?;
    out.flush()?;
    let resource_path = c.output.with_extension("resources.json");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(resource_path)?;
    let mut resource = resources(details)?;
    resource["preload"] = preload;
    resource["base_records"] = serde_json::json!(c.n);
    resource["timed_write_operations"] = serde_json::json!(write_operations);
    resource["final_population"] = serde_json::json!(c.n + write_operations);
    resource["measurement_start_epoch"] = start_load["epoch"].clone();
    resource["measurement_end_epoch"] = end_load["epoch"].clone();
    resource["boundary_load_samples"] = serde_json::json!([start_load, end_load]);
    resource["proc_stat"] = serde_json::json!(std::fs::read_to_string("/proc/self/stat")?);
    serde_json::to_writer_pretty(&mut file, &resource)?;
    writeln!(file)?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("INVALID: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_engines_and_modes_verify_reopen_and_reject_corruption() -> Result<()> {
        for name in ["kiban", "helix", "lmdb"] {
            for mode in [Mode::Durable, Mode::Fast] {
                let path = std::env::temp_dir().join(format!(
                    "enginebench-{}-{name}-{mode:?}",
                    std::process::id()
                ));
                std::fs::create_dir(&path)?;
                let c = Config {
                    backend: name.into(),
                    db: path.join("db"),
                    endpoint: String::new(),
                    n: 128,
                    operations: 100,
                    k: 3,
                    seed: 20261016,
                    output: path.join("unused"),
                    verify_only: false,
                };
                let (mut bulk, _) = open(name, &c.db, Mode::Fast)?;
                bulk_load(bulk.as_mut(), &c)?;
                drop(bulk);
                let plan = runner::Plan {
                    base_population: c.n,
                    insertions: c.operations,
                    write_operations: c.operations,
                };
                let (mut b, _) = open(name, &c.db, mode)?;
                runner::verify_initial(b.as_mut(), &c)?;
                runner::suite_plan(b.as_mut(), &c, plan, &mut Vec::new())?;
                drop(b);
                let (mut b, _) = open(name, &c.db, mode)?;
                runner::verify_persisted_plan(b.as_mut(), &c, plan, &mut Vec::new())?;
                b.put(&model::Memory::new(5, 999, c.seed), false)?;
                assert!(
                    runner::verify_persisted_plan(b.as_mut(), &c, plan, &mut Vec::new()).is_err()
                );
                drop(b);
                std::fs::remove_dir_all(path)?;
            }
        }
        Ok(())
    }
}
