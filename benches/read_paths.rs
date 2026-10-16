//! Read-path benchmark: point lookups and range scans against a seeded,
//! compacted multi-level database (no dependencies).
//!
//! The database is seeded once into `KIBAN_BENCH_DIR` (default: a temp
//! directory) by inserting keys in a pseudo-random permutation, so
//! compaction produces real multi-level overlap rather than trivially moved
//! sequential runs. Later runs reuse the seeded directory; every workload is
//! read-only.
//!
//! Environment:
//! - `KIBAN_BENCH_DIR`: seeded database directory (reused when present).
//! - `KIBAN_BENCH_KEYS`: key count (default 1,000,000).
//! - `KIBAN_BLOCK_CACHE_BYTES`: block cache budget (default 32 MiB).
//! - `KIBAN_RANGE_CACHE_BYTES`: range cache budget (default 0 = disabled).
//! - `KIBAN_BENCH_SAMPLES`: median samples per workload (default 3).
//! - `KIBAN_BENCH_THREADS`: comma-separated thread counts (default 1,8).
//! - `KIBAN_BENCH_COLD`: before every sample, evict the SST files from the
//!   OS page cache (`posix_fadvise(DONTNEED)`), so reads reach the device.
//!   Meaningful only on a disk-backed directory, not tmpfs.
//!
//! Absolute numbers depend on the OS page cache: a seeded database smaller
//! than RAM is served from page-cache-resident bytes after the first pass,
//! so these measure engine CPU and syscall cost, not device latency.

use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use kiban::db::{KibanOptions, SharedKiban};

const VALUE_LEN: usize = 100;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn key(index: usize) -> Vec<u8> {
    format!("user{index:010}").into_bytes()
}

/// Sorts between `key(index)` and `key(index + 1)`, so it passes every
/// table key-range check and must be rejected by filters or block search.
fn absent_key(index: usize) -> Vec<u8> {
    format!("user{index:010}~").into_bytes()
}

/// Deterministic permutation step for an odd multiplier and power-of-two
/// modulus wide enough for `count`.
fn permuted(i: usize, count: usize) -> usize {
    let modulus = count.next_power_of_two();
    let mut x = i;
    loop {
        x = (x.wrapping_mul(0x9E37_79B1).wrapping_add(0x7F4A_7C15)) & (modulus - 1);
        if x < count {
            return x;
        }
    }
}

/// Applies the range-cache budget. The baseline tree has no such option.
fn set_range_cache(options: &mut KibanOptions, bytes: usize) {
    options.range_cache_bytes = bytes;
}

fn options() -> KibanOptions {
    let mut options = KibanOptions {
        block_cache_bytes: env_usize("KIBAN_BLOCK_CACHE_BYTES", 32 << 20),
        ..KibanOptions::default()
    };
    set_range_cache(&mut options, env_usize("KIBAN_RANGE_CACHE_BYTES", 0));
    options
}

fn seed(dir: &std::path::Path, count: usize) {
    let marker = dir.join("SEEDED");
    if marker.exists() {
        return;
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let started = Instant::now();
    {
        let db = SharedKiban::open_with_options(dir, KibanOptions::default()).unwrap();
        let value = [b'v'; VALUE_LEN];
        for i in 0..count {
            db.put(key(permuted(i, count)), value).unwrap();
        }
        db.checkpoint().unwrap();
        db.close().unwrap();
    }
    std::fs::write(&marker, count.to_string()).unwrap();
    eprintln!("seeded {count} keys in {:?}", started.elapsed());
}

unsafe extern "C" {
    fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
}

/// In cold mode, drops every SST file's clean pages from the OS page cache.
fn cold_reset(dir: &std::path::Path) {
    if std::env::var_os("KIBAN_BENCH_COLD").is_none() {
        return;
    }
    use std::os::unix::io::AsRawFd;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "sst") {
            let file = std::fs::File::open(&path).unwrap();
            // POSIX_FADV_DONTNEED = 4
            let rc = unsafe { posix_fadvise(file.as_raw_fd(), 0, 0, 4) };
            assert_eq!(rc, 0, "posix_fadvise failed for {path:?}");
        }
    }
}

fn run_threads<F>(threads: usize, total: usize, op: F) -> (Duration, u64)
where
    F: Fn(usize, usize) -> u64 + Send + Sync + 'static,
{
    let op = Arc::new(op);
    let barrier = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let op = op.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let begin = total * t / threads;
                let end = total * (t + 1) / threads;
                barrier.wait();
                let mut observed = 0;
                for i in begin..end {
                    observed += op(t, i);
                }
                observed
            })
        })
        .collect();
    let timer = Instant::now();
    barrier.wait();
    let observed = handles.into_iter().map(|h| h.join().unwrap()).sum();
    (timer.elapsed(), observed)
}

fn measure<F>(label: &str, units: usize, samples: usize, mut sample: F)
where
    F: FnMut() -> (Duration, u64),
{
    let mut rates = Vec::with_capacity(samples);
    let mut observed = 0;
    for _ in 0..samples {
        let (elapsed, seen) = sample();
        observed = seen;
        rates.push(units as f64 / elapsed.as_secs_f64());
    }
    rates.sort_by(f64::total_cmp);
    let median = rates[rates.len() / 2];
    println!(
        "{label:<36} {median:>14.0} ops/s  (min {:>12.0}, max {:>12.0}, observed {observed})",
        rates[0],
        rates[rates.len() - 1]
    );
}

fn main() {
    let count = env_usize("KIBAN_BENCH_KEYS", 1_000_000);
    let samples = env_usize("KIBAN_BENCH_SAMPLES", 3);
    let threads: Vec<usize> = std::env::var("KIBAN_BENCH_THREADS")
        .unwrap_or_else(|_| "1,8".to_string())
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let dir = std::env::var_os("KIBAN_BENCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("kiban-read-paths-{count}")));
    seed(&dir, count);

    let options = options();
    println!(
        "read_paths: {count} keys, block cache {} MiB, samples {samples}",
        options.block_cache_bytes >> 20
    );
    let db = SharedKiban::open_with_options(&dir, options).unwrap();
    db.checkpoint().unwrap();
    let stats = db.stats().unwrap();
    for level in &stats.levels {
        println!(
            "  L{}: {} tables, {} MiB",
            level.level,
            level.tables,
            level.bytes >> 20
        );
    }

    // Warm the OS page cache so every configuration sees identical
    // page-cache residency.
    let snap = db.snapshot().unwrap();
    let warmed = snap.iter().count();
    assert_eq!(warmed, count);
    drop(snap);

    let gets = env_usize("KIBAN_BENCH_GETS", 400_000);
    let short_scans = env_usize("KIBAN_BENCH_SHORT_SCANS", 100_000);
    let long_scans = env_usize("KIBAN_BENCH_LONG_SCANS", 2_000);

    for &t in &threads {
        let d = db.clone();
        measure(&format!("get hit / {t}t"), gets, samples, || {
            cold_reset(&dir);
            let d = d.clone();
            run_threads(t, gets, move |_, i| {
                let k = key(permuted(i.wrapping_mul(7919), count));
                d.get(&k).unwrap().expect("seeded key").len() as u64
            })
        });
        let d = db.clone();
        measure(&format!("get miss / {t}t"), gets, samples, || {
            cold_reset(&dir);
            let d = d.clone();
            run_threads(t, gets, move |_, i| {
                let k = absent_key(permuted(i.wrapping_mul(7919), count));
                u64::from(d.get(&k).unwrap().is_none())
            })
        });
        for &(name, len, ops) in &[
            ("scan 16", 16usize, short_scans),
            ("scan 1000", 1000, long_scans),
        ] {
            let snapshots: Arc<Vec<_>> = Arc::new((0..t).map(|_| db.snapshot().unwrap()).collect());
            measure(&format!("{name} / {t}t"), ops, samples, || {
                cold_reset(&dir);
                let snapshots = snapshots.clone();
                run_threads(t, ops, move |thread, i| {
                    let start = key(permuted(i.wrapping_mul(104_729), count));
                    snapshots[thread]
                        .range(&start, b"user~")
                        .take(len)
                        .map(|r| r.unwrap().1.len() as u64)
                        .sum::<u64>()
                })
            });
        }
    }
    let snap = db.snapshot().unwrap();
    measure("full scan / 1t", count, samples, || {
        cold_reset(&dir);
        let started = Instant::now();
        let seen = snap.iter().map(|r| r.unwrap().1.len() as u64).sum();
        (started.elapsed(), seen)
    });
    drop(snap);

    let stats = db.stats().unwrap();
    println!(
        "block cache: hits {} misses {} evictions {} coalesced {}",
        stats.block_cache.hits,
        stats.block_cache.misses,
        stats.block_cache.evictions,
        stats.block_cache.coalesced
    );
    println!(
        "file cache: hits {} misses {}; read amp: l0 {} leveled {}",
        stats.table_files.hits,
        stats.table_files.misses,
        stats.read_amp.l0_tables_probed,
        stats.read_amp.leveled_tables_probed
    );
    print_range_cache(&stats);
}

fn print_range_cache(stats: &kiban::db::KibanStats) {
    let r = &stats.range_cache;
    println!(
        "range cache: resident {} KiB in {} ranges, hits {} partial {} misses {} reads {} evictions {}",
        r.resident_bytes >> 10,
        r.ranges,
        r.hits,
        r.partial_hits,
        r.misses,
        r.source_reads,
        r.evictions
    );
}
