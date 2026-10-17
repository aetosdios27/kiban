//! Same W2/W3 API, payload, verifier, histogram and allocator as enginebench.
//! Helix is excluded so read engineering does not require a graph-engine build.
#[path = "../../src/api.rs"]
mod api;
#[allow(dead_code)]
mod backend;
mod block_compare;
mod block_index;
mod cold;
#[path = "../../src/embedded.rs"]
#[allow(dead_code)]
mod embedded;
mod index_hints;
mod inline_index;
#[path = "../../../../examples/scale/metrics.rs"]
#[allow(dead_code)]
mod metrics;
mod micro;
#[path = "../../src/model.rs"]
mod model;
#[path = "../../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;
#[path = "../../src/runner.rs"]
#[allow(dead_code)]
mod runner;

use api::{Backend, Result};
use kiban::db::{Kiban, KibanOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{hint::black_box, io::Write, path::PathBuf, time::Instant};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    #[serde(default)]
    verify_only: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(flatten)]
    suite: Config,
    action: String,
    layout: String,
    cache_mib: Option<usize>,
    #[serde(default)]
    mmap_max_level: Option<u32>,
    #[serde(default = "one_reader")]
    readers: usize,
    #[serde(default)]
    point_index: bool,
    #[serde(default)]
    stage: u64,
    #[serde(default)]
    durable: bool,
    #[serde(default)]
    micro: String,
}

fn one_reader() -> usize {
    1
}

fn open(o: &Options) -> Result<Box<dyn Backend>> {
    Ok(match o.suite.backend.as_str() {
        "kiban" => Box::new(backend::Embedded::with_options(
            &o.suite.db,
            KibanOptions {
                point_index: o.point_index,
                block_cache_bytes: o.cache_mib.map_or_else(
                    || KibanOptions::default().block_cache_bytes,
                    |mib| mib * 1024 * 1024,
                ),
                mmap_max_level: o.mmap_max_level,
                ..KibanOptions::default()
            },
        )?) as Box<dyn Backend>,
        "lmdb" => Box::new(backend::Lmdb::open(&o.suite.db, Mode::Fast)?.0),
        _ => return Err("only kiban/lmdb supported".into()),
    })
}

fn layout(c: &Config) -> Result<Value> {
    if c.backend != "kiban" {
        return Ok(Value::Null);
    }
    let manifest = kiban::manifest::Manifest::load(&c.db)?.ok_or("missing manifest")?;
    let mut levels = std::collections::BTreeMap::<u32, Vec<u64>>::new();
    for t in manifest.tables {
        levels.entry(t.level).or_default().push(t.number);
    }
    Ok(json!(levels))
}

fn prepare(o: &Options) -> Result<()> {
    let c = &o.suite;
    if c.db.exists() {
        return Err("prepare requires a fresh database".into());
    }
    if c.backend == "kiban" && o.layout == "normal" {
        // Direct Kiban leaves flush policy to its caller. Flush at its default
        // write-buffer budget and let flush run the normal compaction picker.
        let mut db = Kiban::open_with_options(
            &c.db,
            KibanOptions {
                point_index: o.point_index,
                ..KibanOptions::default()
            },
        )?;
        let mut bytes = 0;
        for id in 0..c.n {
            let memory = model::Memory::new(id, 0, c.seed);
            let key = memory.key();
            let value = serde_json::to_vec(&memory)?;
            bytes += key.len() + value.len() + 16;
            db.put(key, value)?;
            if bytes >= db.options().write_buffer_bytes {
                db.flush()?;
                bytes = 0;
            }
        }
        db.close()?;
    } else {
        let mut b = open(o)?;
        b.bulk_load(c.n, c.seed)?;
        b.finish()?;
    }
    let mut b = open(o)?;
    runner::verify_initial(b.as_mut(), c)?;
    println!(
        "{}",
        json!({"type":"prepared", "verified_records":c.n,"layout":layout(c)?})
    );
    Ok(())
}

fn perf_control(command: &str) -> Result<()> {
    if let Some(path) = std::env::var_os("READBENCH_PERF_CTL") {
        use std::io::BufRead;
        let ack =
            std::fs::File::open(std::env::var_os("READBENCH_PERF_ACK").ok_or("missing ack")?)?;
        let mut ctl = std::fs::OpenOptions::new().write(true).open(path)?;
        writeln!(ctl, "{command}")?;
        let mut reply = String::new();
        std::io::BufReader::new(ack).read_line(&mut reply)?;
        if reply.trim() != "ack" {
            return Err(format!("perf control: {reply:?}").into());
        }
    }
    Ok(())
}

#[inline(never)]
fn profile(b: &mut dyn Backend, c: &Config, stage: u64) -> Result<()> {
    let ids: Vec<_> = (0..c.operations)
        .map(|op| random::mix(c.seed ^ random::mix(stage) ^ op) % c.n)
        .collect();
    perf_control("enable")?;
    let start = Instant::now();
    let mut digest = 0;
    for id in ids {
        if stage == 2 {
            let records = b.recent((id % model::AGENTS) as u32, c.k)?;
            if records.len() != c.k {
                return Err("profile recent length mismatch".into());
            }
            digest ^= black_box(records).iter().map(|m| m.created_at).sum::<u64>();
        } else {
            let record = b.get(id)?;
            if record.created_at != id {
                return Err("profile get mismatch".into());
            }
            digest ^= black_box(record).created_at;
        }
    }
    let elapsed = start.elapsed().as_nanos();
    perf_control("disable")?;
    println!(
        "{}",
        json!({"type":"profile","operations":c.operations,"ns":elapsed,"digest":digest,"errors":0})
    );
    Ok(())
}

fn reads(o: &Options) -> Result<()> {
    let c = &o.suite;
    let mut b = open(o)?;
    runner::verify_initial(b.as_mut(), c)?;
    let before = layout(c)?;
    if o.action == "profile" {
        profile(b.as_mut(), c, o.stage)?;
    } else {
        let mut out = std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&c.output)?,
        );
        let recent: Vec<_> = (0..model::AGENTS as u32)
            .map(|a| model::recent_ids(c.n, a, c.k))
            .collect();
        for (stage, name) in [(1, "recall-by-id"), (2, "recent-for-agent")] {
            let mut histogram = metrics::Histogram::default();
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
                histogram.record(u64::try_from(elapsed)?);
                if let Some(record) = one {
                    runner::check(&record, id, 0, c.seed)?;
                }
                if let Some(records) = many {
                    let ids = &recent[(id % model::AGENTS) as usize];
                    if records.len() != ids.len() {
                        return Err("recent count mismatch".into());
                    }
                    for (record, id) in records.iter().zip(ids) {
                        runner::check(record, *id, 0, c.seed)?;
                    }
                }
            }
            runner::emit(
                &mut out,
                &runner::row(
                    name,
                    &histogram,
                    &histogram,
                    &metrics::Histogram::default(),
                    started,
                    ns,
                    0,
                ),
            )?;
        }
        runner::emit(
            &mut out,
            &json!({"type":"complete","verified_records":c.n,"errors":0,"timed_writes":0,"layout":before,"proc_status":std::fs::read_to_string("/proc/self/status")?}),
        )?;
    }
    runner::verify_initial(b.as_mut(), c)?;
    drop(b);
    if layout(c)? != before {
        return Err("read-only suite changed layout".into());
    }
    Ok(())
}

fn writes(o: &Options) -> Result<()> {
    let c = &o.suite;
    let mut b: Box<dyn Backend> = if c.backend == "kiban" {
        Box::new(backend::Embedded::with_options_and_durability(
            &c.db,
            KibanOptions {
                point_index: o.point_index,
                ..KibanOptions::default()
            },
            o.durable,
        )?)
    } else {
        Box::new(backend::Lmdb::open(&c.db, if o.durable { Mode::Durable } else { Mode::Fast })?.0)
    };
    runner::verify_initial(b.as_mut(), c)?;
    let mut out = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&c.output)?,
    );
    runner::suite_plan(
        b.as_mut(),
        c,
        runner::Plan {
            base_population: c.n,
            insertions: c.operations,
            write_operations: c.operations,
        },
        &mut out,
    )
}

fn run() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: kiban-readbench CONFIG.json")?;
    let o: Options = serde_json::from_slice(&std::fs::read(path)?)?;
    let c = &o.suite;
    if c.n < 640
        || c.operations == 0
        || c.k == 0
        || c.k > (c.n / model::AGENTS) as usize
        || !matches!(o.cache_mib, None | Some(32 | 512))
        || !matches!(o.layout.as_str(), "bulk" | "normal")
    {
        return Err("invalid readbench options".into());
    }
    match o.action.as_str() {
        "prepare" => prepare(&o),
        "read" | "profile" => reads(&o),
        "write" => writes(&o),
        "cold" => cold::run(&o),
        "micro" => micro::run(&o),
        _ => Err("unknown action".into()),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("INVALID: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_suite_checks_payloads_after_reopen_and_rejects_wrong_revisions() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "readbench-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&root)?;
        for backend in ["kiban", "lmdb"] {
            for shape in ["bulk", "normal"] {
                let o = Options {
                    suite: Config {
                        backend: backend.into(),
                        db: root.join(format!("{backend}-{shape}")),
                        n: 640,
                        operations: 100,
                        k: 10,
                        seed: 20261016,
                        endpoint: String::new(),
                        output: root.join(format!("{backend}-{shape}.jsonl")),
                        verify_only: false,
                    },
                    action: "read".into(),
                    layout: shape.into(),
                    point_index: true,
                    cache_mib: Some(32),
                    mmap_max_level: None,
                    readers: 1,
                    stage: 0,
                    durable: false,
                    micro: String::new(),
                };
                prepare(&o)?;
                reads(&o)?;
                let rows: Vec<Value> = std::fs::read_to_string(&o.suite.output)?
                    .lines()
                    .map(serde_json::from_str)
                    .collect::<std::result::Result<_, _>>()?;
                assert_eq!(rows.len(), 3);
                for row in &rows[..2] {
                    assert_eq!(row["reads"]["count"], 100);
                    assert_eq!(row["errors"], 0);
                }
                let mut b = open(&o)?;
                b.put(&model::Memory::new(5, 99, o.suite.seed), false)?;
                b.finish()?;
                drop(b);
                assert!(
                    reads(&o)
                        .unwrap_err()
                        .to_string()
                        .contains("value mismatch")
                );
            }
        }
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
