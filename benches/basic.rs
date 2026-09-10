//! Hand-rolled benchmark harness (no dependencies).
//!
//! Run with `cargo bench`. Set `KIBAN_BENCH_QUICK=1` to reduce the
//! operation counts and samples for a fast smoke run.

use std::sync::{Arc, Barrier, RwLock};
use std::time::{Duration, Instant};

use kiban::cache::{BlockCache, BlockMeta, CachedBlock};
use kiban::db::{Kiban, KibanOptions, KibanStats, SharedKiban, SharedSnapshot};
use kiban::memtable::Memtable;

const THREAD_COUNTS: &[usize] = &[1, 2, 4, 8, 16, 32];

/// p50/p95/p99 over a flat pool of per-op latency samples (nanoseconds).
fn percentiles(mut samples: Vec<u64>) -> (u64, u64, u64) {
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    (at(0.50), at(0.95), at(0.99))
}

/// Bet A mechanism-level control (11.17 read-gate removal shootout):
/// `readers` threads hammer `Memtable::get` through only a per-memtable
/// `RwLock` (no outer engine-wide gate at all — this is the target
/// architecture's "concurrently readable mutable memtable" in isolation),
/// while `writers` threads concurrently `put` fresh sequence numbers into
/// the SAME memtable. Reports reader-side throughput and latency
/// percentiles; the writer keeps running for the full measured window so
/// the numbers reflect genuine concurrent read/write pressure, not a
/// read-only best case.
fn memtable_rwlock_mixed(
    readers: usize,
    writers: usize,
    reader_ops_total: usize,
    key_space: usize,
) -> (f64, u64, u64, u64) {
    let all_keys = keys("k", key_space);
    let mut seed = Memtable::new();
    for (i, k) in all_keys.iter().enumerate() {
        seed.put(k.clone(), [b'v'; 40], (i + 1) as u64);
    }
    let mem = Arc::new(RwLock::new(seed));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let start = Arc::new(Barrier::new(readers + writers + 1));

    let writer_handles: Vec<_> = (0..writers)
        .map(|w| {
            let mem = mem.clone();
            let stop = stop.clone();
            let start = start.clone();
            let all_keys = all_keys.clone();
            std::thread::spawn(move || {
                start.wait();
                let mut seq = (key_space as u64 + 1) + w as u64 * 10_000_000;
                let mut i: usize = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let k = &all_keys[i % key_space];
                    mem.write().unwrap().put(k.clone(), [b'w'; 40], seq);
                    seq += 1;
                    i += 1;
                }
            })
        })
        .collect();

    let per_reader = reader_ops_total / readers;
    let reader_handles: Vec<_> = (0..readers)
        .map(|r| {
            let mem = mem.clone();
            let start = start.clone();
            let all_keys = all_keys.clone();
            std::thread::spawn(move || {
                start.wait();
                let mut samples = Vec::with_capacity(per_reader);
                for i in 0..per_reader {
                    let index = (r * per_reader + i).wrapping_mul(8191) % key_space;
                    let k = &all_keys[index];
                    let t0 = Instant::now();
                    let got = mem.read().unwrap().get(k);
                    samples.push(t0.elapsed().as_nanos() as u64);
                    assert!(got.is_some());
                }
                samples
            })
        })
        .collect();

    let timer = Instant::now();
    start.wait();
    let mut all_samples = Vec::with_capacity(reader_ops_total);
    for h in reader_handles {
        all_samples.extend(h.join().unwrap());
    }
    let elapsed = timer.elapsed();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for h in writer_handles {
        h.join().unwrap();
    }
    let throughput = all_samples.len() as f64 / elapsed.as_secs_f64();
    let (p50, p95, p99) = percentiles(all_samples);
    (throughput, p50, p95, p99)
}

/// Runs an equivalent prepared workload once as warmup, then takes the
/// middle of independent timed samples. Each closure owns any setup and
/// returns only its timed duration and validated work count.
fn measure<F>(label: &str, units: usize, unit_label: &str, samples: usize, mut sample: F) -> f64
where
    F: FnMut() -> (Duration, u64),
{
    let (_, warmup_work) = sample();
    assert_eq!(warmup_work, units as u64, "{label}: warmup work mismatch");

    let mut durations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let (elapsed, work) = sample();
        assert_eq!(work, units as u64, "{label}: work mismatch");
        durations.push(elapsed);
    }
    durations.sort_unstable();
    let median = durations[durations.len() / 2];
    let throughput = units as f64 / median.as_secs_f64();
    println!("{label:<38} {throughput:>12.0} {unit_label}/s   ({median:?} median)",);
    throughput
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiban-bench-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn drop_dir(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn key(prefix: &str, index: usize) -> Vec<u8> {
    format!("{prefix}{index:08}").into_bytes()
}

fn keys(prefix: &str, count: usize) -> Arc<Vec<Vec<u8>>> {
    Arc::new((0..count).map(|i| key(prefix, i)).collect())
}

fn seed_shared(
    dir: &std::path::Path,
    options: KibanOptions,
    count: usize,
    prefix: &str,
) -> SharedKiban {
    {
        let mut db = Kiban::open_with_options(dir, options.clone()).unwrap();
        for i in 0..count {
            db.put(key(prefix, i), [b'v'; 40]).unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();
    }
    SharedKiban::open_with_options(dir, options).unwrap()
}

fn parallel_gets(
    db: &SharedKiban,
    keys: Arc<Vec<Vec<u8>>>,
    total: usize,
    readers: usize,
    misses: bool,
) -> (Duration, u64) {
    let start = Arc::new(Barrier::new(readers + 1));
    let handles: Vec<_> = (0..readers)
        .map(|thread| {
            let db = db.clone();
            let keys = keys.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let begin = total * thread / readers;
                let end = total * (thread + 1) / readers;
                start.wait();
                let mut observed = 0u64;
                for i in begin..end {
                    let index = (i.wrapping_mul(8191)) % keys.len();
                    let got = db.get(&keys[index]).unwrap();
                    if misses {
                        assert!(got.is_none());
                        observed += 1;
                    } else {
                        let value = got.expect("seeded key must exist");
                        observed += value.len() as u64;
                    }
                }
                observed
            })
        })
        .collect();
    let timer = Instant::now();
    start.wait();
    let observed = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .sum::<u64>();
    let elapsed = timer.elapsed();
    let expected = if misses {
        total as u64
    } else {
        (total * 40) as u64
    };
    assert_eq!(observed, expected);
    (elapsed, total as u64)
}

fn parallel_snapshot_gets(
    snapshot: Arc<SharedSnapshot>,
    keys: Arc<Vec<Vec<u8>>>,
    total: usize,
    readers: usize,
    misses: bool,
) -> (Duration, u64) {
    let start = Arc::new(Barrier::new(readers + 1));
    let handles: Vec<_> = (0..readers)
        .map(|thread| {
            let snapshot = snapshot.clone();
            let keys = keys.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let begin = total * thread / readers;
                let end = total * (thread + 1) / readers;
                start.wait();
                let mut observed = 0u64;
                for i in begin..end {
                    let index = (i.wrapping_mul(8191)) % keys.len();
                    let got = snapshot.get(&keys[index]).unwrap();
                    if misses {
                        assert!(got.is_none());
                        observed += 1;
                    } else {
                        observed += got.expect("seeded key must exist").len() as u64;
                    }
                }
                observed
            })
        })
        .collect();
    let timer = Instant::now();
    start.wait();
    let observed = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .sum::<u64>();
    let elapsed = timer.elapsed();
    let expected = if misses {
        total as u64
    } else {
        (total * 40) as u64
    };
    assert_eq!(observed, expected);
    (elapsed, total as u64)
}

fn benchmark_block() -> CachedBlock {
    CachedBlock {
        data: Arc::<[u8]>::from(vec![0u8; 256]),
        meta: BlockMeta {
            entries_end: 256,
            restart_start: 0,
            num_restarts: 1,
        },
    }
}

fn parallel_cache_hits(
    cache: Arc<BlockCache>,
    keys: Arc<Vec<(u64, u64)>>,
    total: usize,
    readers: usize,
) -> (Duration, u64) {
    let start = Arc::new(Barrier::new(readers + 1));
    let handles: Vec<_> = (0..readers)
        .map(|thread| {
            let cache = cache.clone();
            let keys = keys.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let begin = total * thread / readers;
                let end = total * (thread + 1) / readers;
                start.wait();
                let mut bytes = 0u64;
                for i in begin..end {
                    let key = keys[(i.wrapping_mul(8191)) % keys.len()];
                    bytes += cache.get(&key).expect("warmed cache hit").data.len() as u64;
                }
                bytes
            })
        })
        .collect();
    let timer = Instant::now();
    start.wait();
    let bytes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .sum::<u64>();
    assert_eq!(bytes, (total * 256) as u64);
    (timer.elapsed(), total as u64)
}

fn parallel_micro<F>(threads: usize, total: usize, op: F) -> (Duration, u64)
where
    F: Fn() + Send + Sync + 'static,
{
    let op = Arc::new(op);
    let start = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|thread| {
            let op = op.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let begin = total * thread / threads;
                let end = total * (thread + 1) / threads;
                start.wait();
                for _ in begin..end {
                    op();
                }
                (end - begin) as u64
            })
        })
        .collect();
    let timer = Instant::now();
    start.wait();
    let work = handles.into_iter().map(|h| h.join().unwrap()).sum();
    (timer.elapsed(), work)
}

fn parallel_sharded_reads(shards: usize, threads: usize, total: usize) -> (Duration, u64) {
    let locks = Arc::new((0..shards).map(|_| RwLock::new(())).collect::<Vec<_>>());
    let start = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|thread| {
            let locks = locks.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let begin = total * thread / threads;
                let end = total * (thread + 1) / threads;
                let shard = thread % locks.len();
                start.wait();
                for _ in begin..end {
                    drop(locks[shard].read().unwrap());
                }
                (end - begin) as u64
            })
        })
        .collect();
    let timer = Instant::now();
    start.wait();
    let work = handles.into_iter().map(|h| h.join().unwrap()).sum();
    (timer.elapsed(), work)
}

fn parallel_writes(
    db: &SharedKiban,
    total: usize,
    writers: usize,
    sync_every: Option<usize>,
    prefix: &'static str,
) -> (Duration, u64) {
    let start = Arc::new(Barrier::new(writers + 1));
    let handles: Vec<_> = (0..writers)
        .map(|thread| {
            let db = db.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let begin = total * thread / writers;
                let end = total * (thread + 1) / writers;
                start.wait();
                for i in begin..end {
                    db.put(key(prefix, i), [b'w'; 40]).unwrap();
                    if let Some(cadence) = sync_every
                        && (i + 1) % cadence == 0
                    {
                        db.sync().unwrap();
                    }
                }
                if sync_every.is_some() {
                    db.sync().unwrap();
                }
                (end - begin) as u64
            })
        })
        .collect();
    let timer = Instant::now();
    start.wait();
    let writes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .sum();
    (timer.elapsed(), writes)
}

/// Same shape as `memtable_rwlock_mixed`, but through the real, current
/// `SharedKiban` (engine-wide `ShardedRwLock` gate) — every read is a
/// pure memtable hit (freshly seeded, never flushed), so this isolates
/// the gate's own contribution under concurrent writers as it exists in
/// production today, directly comparable to the gate-free control above.
fn shared_kiban_mixed(
    db: &SharedKiban,
    keys: Arc<Vec<Vec<u8>>>,
    readers: usize,
    writers: usize,
    reader_ops_total: usize,
) -> (f64, u64, u64, u64) {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let start = Arc::new(Barrier::new(readers + writers + 1));

    let writer_handles: Vec<_> = (0..writers)
        .map(|w| {
            let db = db.clone();
            let stop = stop.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let mut i: usize = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    db.put(key(&format!("mw{w}"), i), [b'w'; 40]).unwrap();
                    i += 1;
                }
            })
        })
        .collect();

    let per_reader = reader_ops_total / readers;
    let reader_handles: Vec<_> = (0..readers)
        .map(|r| {
            let db = db.clone();
            let keys = keys.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let mut samples = Vec::with_capacity(per_reader);
                for i in 0..per_reader {
                    let index = (r * per_reader + i).wrapping_mul(8191) % keys.len();
                    let t0 = Instant::now();
                    let got = db.get(&keys[index]).unwrap();
                    samples.push(t0.elapsed().as_nanos() as u64);
                    assert!(got.is_some());
                }
                samples
            })
        })
        .collect();

    let timer = Instant::now();
    start.wait();
    let mut all_samples = Vec::with_capacity(reader_ops_total);
    for h in reader_handles {
        all_samples.extend(h.join().unwrap());
    }
    let elapsed = timer.elapsed();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for h in writer_handles {
        h.join().unwrap();
    }
    let throughput = all_samples.len() as f64 / elapsed.as_secs_f64();
    let (p50, p95, p99) = percentiles(all_samples);
    (throughput, p50, p95, p99)
}

fn print_file_delta(before: &KibanStats, after: &KibanStats) {
    let b = before.table_files;
    let a = after.table_files;
    println!(
        "  table files: hits +{}, misses +{}, evictions +{}, waits +{}",
        a.hits - b.hits,
        a.misses - b.misses,
        a.evictions - b.evictions,
        a.waits - b.waits,
    );
}

fn print_maintenance_delta(before: &KibanStats, after: &KibanStats) {
    let b = before.maintenance;
    let a = after.maintenance;
    println!(
        "  maintenance: flushes +{}, compactions +{}, write stalls +{}, input +{} B, output +{} B",
        a.flushes_completed - b.flushes_completed,
        a.compactions_completed - b.compactions_completed,
        a.write_stalls - b.write_stalls,
        a.compaction_input_bytes - b.compaction_input_bytes,
        a.compaction_output_bytes - b.compaction_output_bytes,
    );
    let tables = after.levels.iter().map(|level| level.tables).sum::<usize>();
    let bytes = after.levels.iter().map(|level| level.bytes).sum::<u64>();
    println!("  final tables: {tables}, final table bytes: {bytes}");
}

fn main() {
    let (samples, writes, reads) = if std::env::var_os("KIBAN_BENCH_QUICK").is_some() {
        (3, 4_000, 20_000)
    } else {
        (5, 20_000, 100_000)
    };
    let wide_keys = writes * 2;
    let mixed_reads = reads * 4 / 5;
    println!("Kiban benchmarks: {samples} median samples");

    println!("\n== Tiny-read synchronization controls ==");
    for &threads in THREAD_COUNTS {
        let label = format!("raw RwLock read / {threads} threads");
        measure(&label, reads, "ops", samples, || {
            let lock = Arc::new(RwLock::new(()));
            parallel_micro(threads, reads, move || drop(lock.read().unwrap()))
        });
    }
    for &threads in THREAD_COUNTS {
        let label = format!("raw Arc clone / {threads} threads");
        measure(&label, reads, "ops", samples, || {
            let value = Arc::new(());
            parallel_micro(threads, reads, move || drop(Arc::clone(&value)))
        });
    }
    for &threads in THREAD_COUNTS {
        let label = format!("RwLock + Arc / {threads} threads");
        measure(&label, reads, "ops", samples, || {
            let value = Arc::new(RwLock::new(Arc::new(())));
            parallel_micro(threads, reads, move || {
                let cloned = Arc::clone(&value.read().unwrap());
                drop(cloned);
            })
        });
    }

    println!("\n== Benchmark-only sharded gate control ==");
    for &shards in &[4, 8, 16] {
        for &threads in THREAD_COUNTS {
            let label = format!("sharded gate {shards} / {threads} threads");
            measure(&label, reads, "ops", samples, || {
                parallel_sharded_reads(shards, threads, reads)
            });
        }
    }

    println!("\n== Bet A: memtable-only, gate-free RwLock<Memtable> (11.17 mechanism control) ==");
    println!("  (Memtable::get through its own RwLock, no engine-wide gate at all)");
    let key_space = 20_000;
    for &(rname, _r_num, w_num) in &[
        ("100R/0W", 1.0, 0.0),
        ("99R/1W", 0.99, 0.01),
        ("95R/5W", 0.95, 0.05),
        ("90R/10W", 0.90, 0.10),
    ] {
        for &threads in THREAD_COUNTS {
            let writers = ((threads as f64 * w_num).round() as usize).clamp(
                if w_num > 0.0 { 1 } else { 0 },
                threads.saturating_sub(1).max(1),
            );
            let readers = (threads - writers).max(1);
            let (throughput, p50, p95, p99) =
                memtable_rwlock_mixed(readers, writers, reads, key_space);
            println!(
                "  {rname:<8} {threads:>2} threads ({readers}R/{writers}W)  {throughput:>12.0} gets/s   p50={:>6.2}us p95={:>7.2}us p99={:>8.2}us",
                p50 as f64 / 1000.0,
                p95 as f64 / 1000.0,
                p99 as f64 / 1000.0,
            );
        }
    }

    println!(
        "\n== Bet A: same workload through the real gated SharedKiban (memtable hits only) =="
    );
    {
        let dir = temp_dir("gate-mixed");
        let db = SharedKiban::open(&dir).unwrap();
        for i in 0..key_space {
            db.put(key("k", i), [b'v'; 40]).unwrap();
        }
        let read_keys = keys("k", key_space);
        for &(rname, _r_num, w_num) in &[
            ("100R/0W", 1.0, 0.0),
            ("99R/1W", 0.99, 0.01),
            ("95R/5W", 0.95, 0.05),
            ("90R/10W", 0.90, 0.10),
        ] {
            for &threads in THREAD_COUNTS {
                let writers = ((threads as f64 * w_num).round() as usize).clamp(
                    if w_num > 0.0 { 1 } else { 0 },
                    threads.saturating_sub(1).max(1),
                );
                let readers = (threads - writers).max(1);
                let before = db.stats().unwrap();
                let (throughput, p50, p95, p99) =
                    shared_kiban_mixed(&db, read_keys.clone(), readers, writers, reads);
                let after = db.stats().unwrap();
                println!(
                    "  {rname:<8} {threads:>2} threads ({readers}R/{writers}W)  {throughput:>12.0} gets/s   p50={:>6.2}us p95={:>7.2}us p99={:>8.2}us  [flushes +{} compactions +{} stalls +{} l0={}]",
                    p50 as f64 / 1000.0,
                    p95 as f64 / 1000.0,
                    p99 as f64 / 1000.0,
                    after.maintenance.flushes_completed - before.maintenance.flushes_completed,
                    after.maintenance.compactions_completed
                        - before.maintenance.compactions_completed,
                    after.maintenance.write_stalls - before.maintenance.write_stalls,
                    after
                        .levels
                        .iter()
                        .find(|l| l.level == 0)
                        .map(|l| l.tables)
                        .unwrap_or(0),
                );
            }
        }
        drop(db);
        drop_dir(&dir);
    }

    println!("\n== Empty and active SharedKiban controls ==");
    for &threads in THREAD_COUNTS {
        let label = format!("empty Shared miss / {threads} threads");
        measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("empty-shared");
            let db = SharedKiban::open(&dir).unwrap();
            let result = parallel_gets(&db, keys("empty", 1), reads, threads, true);
            drop(db);
            drop_dir(&dir);
            result
        });
    }

    println!("\n== Direct BlockCache hit control ==");
    for &readers in THREAD_COUNTS {
        let label = format!("cache same block / {readers} readers");
        measure(&label, reads, "hits", samples, || {
            let cache = Arc::new(BlockCache::new(1024));
            cache.insert((1, 0), benchmark_block());
            parallel_cache_hits(cache, Arc::new(vec![(1, 0)]), reads, readers)
        });
    }
    for &readers in THREAD_COUNTS {
        let label = format!("cache scattered / {readers} readers");
        measure(&label, reads, "hits", samples, || {
            let cache = Arc::new(BlockCache::new(1024 * 256));
            let keys: Vec<_> = (0..1024u64).map(|i| (1, i)).collect();
            for key in &keys {
                cache.insert(*key, benchmark_block());
            }
            parallel_cache_hits(cache, Arc::new(keys), reads, readers)
        });
    }

    println!("\n== Buffered write baseline ==");
    measure("put, buffered (Kiban)", writes, "ops", samples, || {
        let dir = temp_dir("buffered-write");
        let mut db = Kiban::open(&dir).unwrap();
        let timer = Instant::now();
        for i in 0..writes {
            db.put(key("bw", i), [b'v'; 40]).unwrap();
        }
        let elapsed = timer.elapsed();
        drop(db);
        drop_dir(&dir);
        (elapsed, writes as u64)
    });

    println!("\n== Durability cost ==");
    let per_op = (writes / 40).max(100);
    measure("put + sync every operation", per_op, "ops", samples, || {
        let dir = temp_dir("per-op-sync");
        let db = SharedKiban::open(&dir).unwrap();
        let timer = Instant::now();
        for i in 0..per_op {
            db.put(key("ps", i), [b'v'; 40]).unwrap();
            db.sync().unwrap();
        }
        let elapsed = timer.elapsed();
        drop(db);
        drop_dir(&dir);
        (elapsed, per_op as u64)
    });
    measure(
        "put + sync every 500 writes",
        writes,
        "ops",
        samples,
        || {
            let dir = temp_dir("batch-sync");
            let db = SharedKiban::open(&dir).unwrap();
            let timer = Instant::now();
            for i in 0..writes {
                db.put(key("bs", i), [b'v'; 40]).unwrap();
                if (i + 1) % 500 == 0 {
                    db.sync().unwrap();
                }
            }
            db.sync().unwrap();
            let elapsed = timer.elapsed();
            drop(db);
            drop_dir(&dir);
            (elapsed, writes as u64)
        },
    );

    println!("\n== Shared point reads: hot block-cache working set ==");
    let mut hot_one = 0.0;
    for &readers in THREAD_COUNTS {
        let label = format!("hot / {readers} readers");
        let throughput = measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("hot-reads");
            let db = seed_shared(&dir, KibanOptions::default(), writes, "hot");
            let keys = keys("hot", writes);
            for key in keys.iter() {
                assert!(db.get(key).unwrap().is_some());
            }
            let result = parallel_gets(&db, keys, reads, readers, false);
            drop(db);
            drop_dir(&dir);
            result
        });
        if readers == 1 {
            hot_one = throughput;
        }
        println!("  scaling: {:.2}x", throughput / hot_one);
    }

    println!("\n== Shared point reads: wide working set (Kiban cache misses) ==");
    let mut wide_one = 0.0;
    for &readers in THREAD_COUNTS {
        let label = format!("wide / {readers} readers");
        let throughput = measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("wide-reads");
            let options = KibanOptions {
                block_cache_bytes: 4 * 1024,
                ..KibanOptions::default()
            };
            let db = seed_shared(&dir, options, wide_keys, "wide");
            let result = parallel_gets(&db, keys("wide", wide_keys), reads, readers, false);
            drop(db);
            drop_dir(&dir);
            result
        });
        if readers == 1 {
            wide_one = throughput;
        }
        println!("  scaling: {:.2}x", throughput / wide_one);
    }

    println!("\n== Shared point reads: wide working set, mmap_max_level enabled ==");
    let mut wide_mmap_one = 0.0;
    for &readers in THREAD_COUNTS {
        let label = format!("wide+mmap / {readers} readers");
        let throughput = measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("wide-reads-mmap");
            let options = KibanOptions {
                block_cache_bytes: 4 * 1024,
                mmap_max_level: Some(u32::MAX),
                ..KibanOptions::default()
            };
            let db = seed_shared(&dir, options, wide_keys, "wide");
            let result = parallel_gets(&db, keys("wide", wide_keys), reads, readers, false);
            drop(db);
            drop_dir(&dir);
            result
        });
        if readers == 1 {
            wide_mmap_one = throughput;
        }
        println!("  scaling: {:.2}x", throughput / wide_mmap_one);
    }

    println!("\n== Shared Bloom-rejected misses ==");
    for &readers in &[1, 4, 8] {
        let label = format!("bloom miss / {readers} readers");
        measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("bloom-miss");
            let db = seed_shared(&dir, KibanOptions::default(), writes, "present");
            let result = parallel_gets(&db, keys("missing", writes), reads, readers, true);
            drop(db);
            drop_dir(&dir);
            result
        });
    }

    println!("\n== FD-cache pressure ==");
    for &readers in &[1, 4, 8] {
        let label = format!("FD pressure / {readers} readers");
        let mut final_stats = None;
        measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("fd-pressure");
            let options = KibanOptions {
                max_open_table_files: 2,
                l0_compaction_trigger: 1_000,
                l0_write_stall_trigger: 2_000,
                block_cache_bytes: 0,
                ..KibanOptions::default()
            };
            {
                let mut db = Kiban::open_with_options(&dir, options.clone()).unwrap();
                for i in 0..64 {
                    db.put(key("fd", i), [b'v'; 40]).unwrap();
                    db.sync().unwrap();
                    db.flush().unwrap();
                }
            }
            let db = SharedKiban::open_with_options(&dir, options).unwrap();
            let before = db.stats().unwrap();
            let result = parallel_gets(&db, keys("fd", 64), reads, readers, false);
            let after = db.stats().unwrap();
            final_stats = Some((before, after));
            drop(db);
            drop_dir(&dir);
            result
        });
        let (before, after) = final_stats.expect("FD sample must run");
        print_file_delta(&before, &after);
    }

    println!("\n== Bet F: single-flight cache-miss coalescing ==");
    {
        // Same cold key, hammered by every thread at once: without
        // coalescing this is N physical block reads; with it, ~1.
        let dir = temp_dir("single-flight-stampede");
        let options = KibanOptions {
            block_cache_bytes: 64 * 1024 * 1024,
            ..KibanOptions::default()
        };
        let mut db = Kiban::open_with_options(&dir, options.clone()).unwrap();
        for i in 0..2_000 {
            db.put(key("sf", i), [b'v'; 200]).unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();
        drop(db);
        for &readers in THREAD_COUNTS {
            let db = SharedKiban::open_with_options(&dir, options.clone()).unwrap();
            let before = db.stats().unwrap();
            let target = key("sf", 1_000);
            let start = Arc::new(Barrier::new(readers + 1));
            let handles: Vec<_> = (0..readers)
                .map(|_| {
                    let db = db.clone();
                    let target = target.clone();
                    let start = start.clone();
                    std::thread::spawn(move || {
                        start.wait();
                        db.get(&target).unwrap().unwrap().len()
                    })
                })
                .collect();
            let timer = Instant::now();
            start.wait();
            for h in handles {
                h.join().unwrap();
            }
            let elapsed = timer.elapsed();
            let after = db.stats().unwrap();
            println!(
                "  stampede {readers:>2} threads on 1 cold key  {:>8.2?}   [block-cache misses +{} coalesced +{} | file hits +{} misses +{}]",
                elapsed,
                after.block_cache.misses - before.block_cache.misses,
                after.block_cache.coalesced - before.block_cache.coalesced,
                after.table_files.hits - before.table_files.hits,
                after.table_files.misses - before.table_files.misses,
            );
            drop(db);
        }
        drop_dir(&dir);
    }
    {
        // Control: every thread reads a DIFFERENT cold key — nothing
        // should collide, so coalesced must stay at 0 and throughput
        // should track plain concurrent cold reads (no added overhead
        // from the single-flight bookkeeping in the common case).
        let dir = temp_dir("single-flight-no-collision");
        let options = KibanOptions {
            block_cache_bytes: 64 * 1024 * 1024,
            ..KibanOptions::default()
        };
        let mut db = Kiban::open_with_options(&dir, options.clone()).unwrap();
        for i in 0..2_000 {
            db.put(key("nc", i), [b'v'; 200]).unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();
        drop(db);
        for &readers in THREAD_COUNTS {
            let db = SharedKiban::open_with_options(&dir, options.clone()).unwrap();
            let before = db.stats().unwrap();
            let start = Arc::new(Barrier::new(readers + 1));
            let handles: Vec<_> = (0..readers)
                .map(|t| {
                    let db = db.clone();
                    let start = start.clone();
                    std::thread::spawn(move || {
                        start.wait();
                        let target = key("nc", (t * 37) % 2_000);
                        db.get(&target).unwrap().unwrap().len()
                    })
                })
                .collect();
            let timer = Instant::now();
            start.wait();
            for h in handles {
                h.join().unwrap();
            }
            let elapsed = timer.elapsed();
            let after = db.stats().unwrap();
            println!(
                "  no-collision {readers:>2} threads, distinct cold keys  {:>8.2?}   [coalesced +{}]",
                elapsed,
                after.block_cache.coalesced - before.block_cache.coalesced,
            );
            drop(db);
        }
        drop_dir(&dir);
    }

    println!("\n== Shared writer scaling: buffered ==");
    let mut writer_one = 0.0;
    for &writers in THREAD_COUNTS {
        let label = format!("buffered / {writers} writers");
        let throughput = measure(&label, writes, "ops", samples, || {
            let dir = temp_dir("writer-buffered");
            let db = SharedKiban::open(&dir).unwrap();
            let result = parallel_writes(&db, writes, writers, None, "wb");
            drop(db);
            drop_dir(&dir);
            result
        });
        if writers == 1 {
            writer_one = throughput;
        }
        println!("  scaling: {:.2}x", throughput / writer_one);
    }

    println!("\n== Shared writer scaling: sync every 250 writes ==");
    let mut durable_one = 0.0;
    for &writers in THREAD_COUNTS {
        let label = format!("durable / {writers} writers");
        let throughput = measure(&label, writes, "ops", samples, || {
            let dir = temp_dir("writer-durable");
            let db = SharedKiban::open(&dir).unwrap();
            let result = parallel_writes(&db, writes, writers, Some(250), "wd");
            drop(db);
            drop_dir(&dir);
            result
        });
        if writers == 1 {
            durable_one = throughput;
        }
        println!("  scaling: {:.2}x", throughput / durable_one);
    }

    println!("\n== Mixed workload: 4 readers + 1 writer ==");
    let mut mixed_stats = None;
    measure(
        "mixed SST reads and updates",
        mixed_reads + writes,
        "ops",
        samples,
        || {
            let dir = temp_dir("mixed");
            let options = KibanOptions {
                write_buffer_bytes: 16 * 1024,
                l0_compaction_trigger: 2,
                l0_write_stall_trigger: 8,
                ..KibanOptions::default()
            };
            let db = seed_shared(&dir, options, writes, "mix");
            let before = db.stats().unwrap();
            let start = Arc::new(Barrier::new(6));
            let mut handles = Vec::new();
            for thread in 0..4 {
                let db = db.clone();
                let start = start.clone();
                let read_keys = keys("mix", writes);
                let each = mixed_reads / 4;
                handles.push(std::thread::spawn(move || {
                    start.wait();
                    let mut bytes = 0u64;
                    for i in 0..each {
                        let index = ((i + thread * each).wrapping_mul(8191)) % read_keys.len();
                        bytes += db.get(&read_keys[index]).unwrap().unwrap().len() as u64;
                    }
                    bytes
                }));
            }
            let writer = db.clone();
            let writer_start = start.clone();
            handles.push(std::thread::spawn(move || {
                writer_start.wait();
                for i in 0..writes {
                    writer.put(key("update", i), [b'u'; 40]).unwrap();
                }
                writes as u64
            }));
            let timer = Instant::now();
            start.wait();
            let observed: u64 = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .sum();
            let elapsed = timer.elapsed();
            assert_eq!(observed, (mixed_reads * 40 + writes) as u64);
            let after = db.stats().unwrap();
            mixed_stats = Some((before, after));
            drop(db);
            drop_dir(&dir);
            (elapsed, (mixed_reads + writes) as u64)
        },
    );
    let (before, after) = mixed_stats.expect("mixed sample must run");
    print_maintenance_delta(&before, &after);
    print_file_delta(&before, &after);
    println!(
        "  block cache: hits +{}, misses +{}",
        after.block_cache.hits - before.block_cache.hits,
        after.block_cache.misses - before.block_cache.misses,
    );

    println!("\n== 8 readers / 1 writer liveness control (Phase 11.15) ==");
    let mut heavy_mixed_stats = None;
    measure(
        "8R/1W writer progress under read pressure",
        mixed_reads + writes,
        "ops",
        samples,
        || {
            let dir = temp_dir("heavy-mixed");
            let options = KibanOptions {
                write_buffer_bytes: 16 * 1024,
                l0_compaction_trigger: 2,
                l0_write_stall_trigger: 8,
                ..KibanOptions::default()
            };
            let db = seed_shared(&dir, options, writes, "heavy");
            let before = db.stats().unwrap();
            let start = Arc::new(Barrier::new(10));
            let mut handles = Vec::new();
            for thread in 0..8 {
                let db = db.clone();
                let start = start.clone();
                let read_keys = keys("heavy", writes);
                let each = mixed_reads / 8;
                handles.push(std::thread::spawn(move || {
                    start.wait();
                    let mut bytes = 0u64;
                    for i in 0..each {
                        let index = ((i + thread * each).wrapping_mul(8191)) % read_keys.len();
                        bytes += db.get(&read_keys[index]).unwrap().unwrap().len() as u64;
                    }
                    bytes
                }));
            }
            let writer = db.clone();
            let writer_start = start.clone();
            let writer_handle = std::thread::spawn(move || {
                writer_start.wait();
                for i in 0..writes {
                    writer.put(key("heavy-update", i), [b'u'; 40]).unwrap();
                }
                writes as u64
            });
            let timer = Instant::now();
            start.wait();
            let reader_bytes: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
            // Writer completion is the liveness proof itself: if the
            // sharded gate let 8 continuously-arriving readers starve the
            // writer, this join would hang (or take drastically longer
            // than the writer-alone baseline) rather than merely score
            // lower.
            let writer_ops = writer_handle.join().unwrap();
            let elapsed = timer.elapsed();
            assert_eq!(reader_bytes, mixed_reads as u64 * 40);
            assert_eq!(writer_ops, writes as u64);
            let after = db.stats().unwrap();
            heavy_mixed_stats = Some((before, after));
            drop(db);
            drop_dir(&dir);
            (elapsed, (mixed_reads + writes) as u64)
        },
    );
    let (before, after) = heavy_mixed_stats.expect("8R/1W sample must run");
    print_maintenance_delta(&before, &after);
    println!("  writer completed all {writes} ops under 8-reader pressure without hanging");

    println!("\n== Maintenance pressure ==");
    let mut pressure_stats = None;
    measure(
        "small buffers under sustained writes",
        writes,
        "ops",
        samples,
        || {
            let dir = temp_dir("maintenance-pressure");
            let options = KibanOptions {
                write_buffer_bytes: 4 * 1024,
                l0_compaction_trigger: 2,
                l0_write_stall_trigger: 6,
                target_file_size: 4 * 1024,
                base_level_bytes: 8 * 1024,
                level_multiplier: 2,
                ..KibanOptions::default()
            };
            let db = SharedKiban::open_with_options(&dir, options).unwrap();
            let before = db.stats().unwrap();
            let timer = Instant::now();
            for i in 0..writes {
                db.put(key("pressure", i), [b'p'; 80]).unwrap();
            }
            let elapsed = timer.elapsed();
            let after = db.stats().unwrap();
            pressure_stats = Some((before, after));
            drop(db);
            drop_dir(&dir);
            (elapsed, writes as u64)
        },
    );
    let (before, after) = pressure_stats.expect("pressure sample must run");
    print_maintenance_delta(&before, &after);

    println!("\n== Bet D: foreground GET p99 under sustained compaction debt (Bet C context) ==");
    {
        let dir = temp_dir("compaction-debt-foreground");
        let options = KibanOptions {
            write_buffer_bytes: 2 * 1024,
            l0_compaction_trigger: 4,
            l0_write_stall_trigger: 60,
            target_file_size: 2 * 1024,
            base_level_bytes: 8 * 1024,
            level_multiplier: 2,
            block_cache_bytes: 256 * 1024,
            ..KibanOptions::default()
        };
        let seed_count = 4_000;
        let mut seed = Kiban::open_with_options(&dir, options.clone()).unwrap();
        for i in 0..seed_count {
            seed.put(key("hot", i), [b'h'; 80]).unwrap();
        }
        seed.sync().unwrap();
        seed.flush().unwrap();
        drop(seed);

        let db = SharedKiban::open_with_options(&dir, options).unwrap();
        let hot_keys = keys("hot", seed_count);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let l0_samples: Arc<std::sync::Mutex<Vec<usize>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        // Foreground: continuous GETs against the already-flushed,
        // never-rewritten "hot" keyspace — a real workload's steady
        // read traffic, unrelated to the writer's own keys, while
        // compaction debt from the writer piles up underneath it.
        let reader = {
            let db = db.clone();
            let hot_keys = hot_keys.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut samples = Vec::new();
                let mut i = 0usize;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let k = &hot_keys[i % hot_keys.len()];
                    let t0 = Instant::now();
                    let got = db.get(k).unwrap();
                    samples.push(t0.elapsed().as_nanos() as u64);
                    assert!(got.is_some());
                    i += 1;
                }
                samples
            })
        };
        // A second thread just samples L0 file count over the run so
        // we can correlate the GET tail with actual compaction debt,
        // not just elapsed time.
        let sampler = {
            let db = db.clone();
            let stop = stop.clone();
            let l0_samples = l0_samples.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(stats) = db.stats() {
                        let l0 = stats
                            .levels
                            .iter()
                            .find(|l| l.level == 0)
                            .map(|l| l.tables)
                            .unwrap_or(0);
                        l0_samples.lock().unwrap().push(l0);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            })
        };

        let write_count = writes * 2;
        let before = db.stats().unwrap();
        let mut write_samples = Vec::with_capacity(write_count);
        let timer = Instant::now();
        for i in 0..write_count {
            let t0 = Instant::now();
            db.put(key("debt", i), [b'w'; 80]).unwrap();
            write_samples.push(t0.elapsed().as_nanos() as u64);
        }
        let write_elapsed = timer.elapsed();
        let after = db.stats().unwrap();

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let read_samples = reader.join().unwrap();
        sampler.join().unwrap();

        let (put_p50, put_p95, put_p99) = percentiles(write_samples);
        let (p50, p95, p99) = percentiles(read_samples.clone());
        let mut l0_sorted = l0_samples.lock().unwrap().clone();
        l0_sorted.sort_unstable();
        let max_l0 = l0_sorted.last().copied().unwrap_or(0);
        let median_l0 = l0_sorted.get(l0_sorted.len() / 2).copied().unwrap_or(0);
        let read_throughput = read_samples.len() as f64 / write_elapsed.as_secs_f64();
        println!(
            "  {write_count} writes in {write_elapsed:?}; concurrent GET: {} ops, {read_throughput:>10.0} ops/s",
            read_samples.len()
        );
        println!(
            "  GET p50={:>6.2}us p95={:>8.2}us p99={:>9.2}us   L0 files: median={median_l0} max={max_l0}",
            p50 as f64 / 1000.0,
            p95 as f64 / 1000.0,
            p99 as f64 / 1000.0,
        );
        println!(
            "  PUT p50={:>6.2}us p95={:>8.2}us p99={:>9.2}us",
            put_p50 as f64 / 1000.0,
            put_p95 as f64 / 1000.0,
            put_p99 as f64 / 1000.0,
        );
        print_maintenance_delta(&before, &after);
        drop(db);
        drop_dir(&dir);
    }

    println!("\n== Range scan baseline (direct Kiban) ==");
    let scans = (reads / 1_000).max(20);
    measure("range scan, 1000 keys", scans, "scans", samples, || {
        let dir = temp_dir("range");
        let mut db = Kiban::open(&dir).unwrap();
        for i in 0..writes {
            db.put(key("range", i), [b'r'; 40]).unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();
        let timer = Instant::now();
        let mut observed = 0u64;
        for i in 0..scans {
            let base = (i * 8191) % (writes - 1_000);
            let start = key("range", base);
            let end = key("range", base + 1_000);
            let count = db.range(&start, &end).count();
            assert_eq!(count, 1_000);
            observed += 1;
        }
        let elapsed = timer.elapsed();
        drop(db);
        drop_dir(&dir);
        (elapsed, observed)
    });

    println!("\n== SharedSnapshot read-scaling control ==");
    for &readers in THREAD_COUNTS {
        let label = format!("snapshot hot / {readers} readers");
        measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("snapshot-hot");
            let db = seed_shared(&dir, KibanOptions::default(), writes, "snap-hot");
            let snapshot = Arc::new(db.snapshot().unwrap());
            let result = parallel_snapshot_gets(
                snapshot.clone(),
                keys("snap-hot", writes),
                reads,
                readers,
                false,
            );
            drop(snapshot);
            drop(db);
            drop_dir(&dir);
            result
        });
    }
    for &readers in &[1, 4, 8] {
        let label = format!("snapshot Bloom miss / {readers} readers");
        measure(&label, reads, "gets", samples, || {
            let dir = temp_dir("snapshot-bloom");
            let db = seed_shared(&dir, KibanOptions::default(), writes, "snap-present");
            let snapshot = Arc::new(db.snapshot().unwrap());
            let result = parallel_snapshot_gets(
                snapshot.clone(),
                keys("snap-missing", writes),
                reads,
                readers,
                true,
            );
            drop(snapshot);
            drop(db);
            drop_dir(&dir);
            result
        });
    }
}
