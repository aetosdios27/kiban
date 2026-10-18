//! Cold-start point latency. Dataset creation and full verification happen in
//! the main suite; this action must not warm the dataset before timing it.
use crate::{Options, api::Result, backend::Lmdb, metrics::Histogram, model, random, runner};
use kiban::db::{Kiban, KibanOptions};
use serde_json::json;
use std::{fs::File, os::fd::AsRawFd, sync::Barrier, time::Instant};

unsafe extern "C" {
    fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
    fn madvise(addr: *mut std::ffi::c_void, len: usize, advice: i32) -> i32;
}

fn evict(path: &std::path::Path) -> Result<()> {
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "sst")
            || path.file_name().is_some_and(|name| name == "data.mdb")
        {
            let file = File::open(path)?;
            // These private prepared datasets have been synced and closed.
            // DONTNEED affects these file pages, never the global page cache.
            let code = unsafe { posix_fadvise(file.as_raw_fd(), 0, 0, 4) };
            if code != 0 {
                return Err(std::io::Error::from_raw_os_error(code).into());
            }
        }
    }
    Ok(())
}

fn evict_mappings(path: &std::path::Path) -> Result<usize> {
    let root = path.canonicalize()?;
    let mut count = 0;
    for line in std::fs::read_to_string("/proc/self/maps")?.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let Some(file) = fields.get(5) else { continue };
        if !std::path::Path::new(file).starts_with(&root) {
            continue;
        }
        let (start, end) = fields[0].split_once('-').ok_or("bad map range")?;
        let start = usize::from_str_radix(start, 16)?;
        let end = usize::from_str_radix(end, 16)?;
        // Only this process's live file mappings under its private dataset.
        // DONTNEED drops PTEs, not file contents. No reader has started yet;
        // the Reader below keeps every advised mapping alive across the call.
        if unsafe { madvise(start as *mut std::ffi::c_void, end - start, 4) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        count += 1;
    }
    Ok(count)
}

fn faults_and_io() -> Result<(u64, u64, u64)> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .ok_or("bad proc stat")?
        .1
        .split_whitespace()
        .collect();
    let io = std::fs::read_to_string("/proc/self/io")?;
    let read_bytes = io
        .lines()
        .find_map(|line| line.strip_prefix("read_bytes: "))
        .ok_or("missing read_bytes")?
        .trim()
        .parse()?;
    Ok((fields[7].parse()?, fields[9].parse()?, read_bytes))
}

enum Reader {
    Kiban(Box<Kiban>),
    Lmdb(Lmdb),
}

impl Reader {
    fn get(&self, id: u64) -> Result<model::Memory> {
        match self {
            Self::Kiban(db) => {
                let bytes = db.get(model::key(id))?.ok_or("missing cold Kiban memory")?;
                Ok(serde_json::from_slice(&bytes)?)
            }
            Self::Lmdb(db) => db.point(id),
        }
    }
}

pub(super) fn run(o: &Options) -> Result<()> {
    let c = &o.suite;
    if !matches!(o.readers, 1 | 4) || !c.operations.is_multiple_of(o.readers as u64) {
        return Err("cold readers must be 1 or 4 and divide operations".into());
    }
    evict(&c.db)?;
    let reader = match c.backend.as_str() {
        "kiban" => Reader::Kiban(Box::new(Kiban::open_with_options(
            &c.db,
            KibanOptions {
                block_cache_bytes: o.cache_mib.map_or_else(
                    || KibanOptions::default().block_cache_bytes,
                    |mib| mib << 20,
                ),
                mmap_max_level: o.mmap_max_level,
                ..KibanOptions::default()
            },
        )?)),
        "lmdb" => Reader::Lmdb(Lmdb::open(&c.db, crate::Mode::Fast)?.0),
        _ => return Err("unknown cold backend".into()),
    };
    // Boundary reads during open can read around an entire SST. Drop PTEs
    // first so DONTNEED can evict those pages before any timed point call.
    let evicted_mappings = evict_mappings(&c.db)?;
    evict(&c.db)?;
    let before = faults_and_io()?;
    let barrier = Barrier::new(o.readers);
    let histograms = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..o.readers)
            .map(|thread| {
                let reader = &reader;
                let barrier = &barrier;
                scope.spawn(move || -> std::result::Result<Histogram, String> {
                    let mut histogram = Histogram::default();
                    barrier.wait();
                    for op in 0..c.operations / o.readers as u64 {
                        let id = random::mix(c.seed ^ random::mix(thread as u64) ^ op) % c.n;
                        let tick = Instant::now();
                        let value = reader.get(id).map_err(|error| error.to_string())?;
                        histogram.record(
                            tick.elapsed()
                                .as_nanos()
                                .try_into()
                                .map_err(|_| "latency overflow")?,
                        );
                        runner::check(&value, id, 0, c.seed).map_err(|error| error.to_string())?;
                    }
                    Ok(histogram)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "cold reader panicked".to_string())?
            })
            .collect::<std::result::Result<Vec<_>, String>>()
    })?;
    let after = faults_and_io()?;
    let mut histogram = Histogram::default();
    for local in histograms {
        histogram.merge(&local);
    }
    assert_eq!(histogram.count, c.operations);
    let row = json!({"type":"cold-start", "backend":c.backend,
        "mmap_max_level":o.mmap_max_level, "readers":o.readers,
        "operations":histogram.count, "verified_calls":histogram.count, "errors":0,
        "latency":{"p50_ns":histogram.percentile(50,100), "p99_ns":histogram.percentile(99,100),
            "p999_ns":histogram.percentile(999,1000)},
        "minor_faults":after.0-before.0, "major_faults":after.1-before.1,
        "storage_read_bytes":after.2-before.2, "evicted_mappings":evicted_mappings});
    std::fs::write(&c.output, format!("{row}\n"))?;
    Ok(())
}
