//! Hand-rolled benchmark harness (no dependencies).
//!
//! Run with `cargo bench`. Set `KIBAN_BENCH_QUICK=1` to reduce the
//! operation counts and samples for a fast smoke run.

use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use kiban::cache::{BlockCache, BlockMeta, CachedBlock};
use kiban::db::{Kiban, KibanOptions, KibanStats, SharedKiban, SharedSnapshot};

const THREAD_COUNTS: &[usize] = &[1, 2, 4, 8];

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
