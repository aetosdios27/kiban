//! Hand-rolled benchmark harness (no dependencies).
//!
//! Run with `cargo bench`. Numbers are single-run wall-clock medians of
//! repeated batches on the development machine; treat them as baselines
//! for relative comparison, not absolute truth.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kiban::db::{Kiban, KibanOptions, SharedKiban};

fn measure<F: FnMut()>(label: &str, units: usize, unit_label: &str, mut f: F) -> f64 {
    // warmup
    f();
    let runs = 3;
    let mut elapsed = Duration::ZERO;
    for _ in 0..runs {
        let start = Instant::now();
        f();
        elapsed += start.elapsed();
    }
    let per_run = elapsed / runs;
    let throughput = units as f64 / per_run.as_secs_f64();
    println!(
        "{:<38} {:>12.0} {}/s   ({:?} for {} {})",
        label, throughput, unit_label, per_run, units, unit_label
    );
    throughput
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiban-bench-{}-{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn main() {
    const N: usize = 20_000;

    println!("== WAL append path ==");
    let dir = temp_dir("append");
    {
        let mut db = Kiban::open(&dir).unwrap();
        let mut i = 0usize;
        measure("put, buffered (no sync)", N, "ops", || {
            let end = i + N;
            while i < end {
                db.put(format!("k{i:08}"), vec![b'v'; 40]).unwrap();
                i += 1;
            }
        });
    }
    drop_dir(&dir);

    println!("== durability cost ==");
    let dir = temp_dir("group");
    {
        let db = SharedKiban::open(&dir).unwrap();
        let mut i = 0usize;
        const BATCH: usize = 500;
        measure("put+sync, group commit (batch 500)", N, "ops", || {
            let end = i + N;
            while i < end {
                for _ in 0..BATCH {
                    db.put(format!("g{i:08}"), vec![b'v'; 40]).unwrap();
                    i += 1;
                }
                db.sync().unwrap();
            }
        });
    }
    drop_dir(&dir);

    let dir = temp_dir("per-op");
    {
        let db = SharedKiban::open(&dir).unwrap();
        let mut i = 0usize;
        const SMALL: usize = 300;
        measure("put+sync, per operation", SMALL, "ops", || {
            let end = i + SMALL;
            while i < end {
                db.put(format!("p{i:08}"), vec![b'v'; 40]).unwrap();
                db.sync().unwrap();
                i += 1;
            }
        });
    }
    drop_dir(&dir);

    println!("== reads ==");
    let dir = temp_dir("reads");
    {
        let options = KibanOptions::default();
        let mut db = Kiban::open_with_options(&dir, options).unwrap();
        for i in 0..N {
            db.put(format!("k{i:08}"), vec![b'v'; 40]).unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();

        let hits = 10_000usize;
        let mut i = 0usize;
        measure("get, sstable hit", hits, "gets", || {
            let end = i + hits;
            while i < end {
                let _ = db.get(format!("k{:08}", i % N)).unwrap();
                i += 1;
            }
        });

        let misses = 10_000usize;
        let mut j = 0usize;
        measure("get, bloom-rejected miss", misses, "gets", || {
            let end = j + misses;
            while j < end {
                let _ = db.get(format!("zzz{j:08}")).unwrap();
                j += 1;
            }
        });

        let scans = 100usize;
        let mut s = 0usize;
        measure("range scan, 1000 keys", scans, "scans", || {
            let end = s + scans;
            while s < end {
                // wrap the window so it always fits inside the keyspace
                // (half-open ranges correctly shrink near the end)
                let base = (s % 381) * 50;
                let start = format!("k{base:08}");
                let stop = format!("k{:08}", base + 1000);
                let count = db.range(start.as_bytes(), stop.as_bytes()).count();
                assert_eq!(count, 1000);
                s += 1;
            }
        });
    }
    drop_dir(&dir);

    println!("== flush + compaction latency ==");
    let dir = temp_dir("flush");
    {
        let mut db = Kiban::open_with_options(&dir, KibanOptions::default()).unwrap();
        let entries: BTreeMap<String, Vec<u8>> = (0..N)
            .map(|i| (format!("k{i:08}"), vec![b'v'; 40]))
            .collect();
        let mut round = 0u32;
        measure("flush 20k entries (with L0 trigger)", 1, "flush", || {
            for (k, v) in &entries {
                db.put(k.clone(), v.clone()).unwrap();
            }
            db.sync().unwrap();
            db.flush().unwrap();
            round += 1;
        });
        let _ = round;
    }
    drop_dir(&dir);

    println!("== concurrent writers (4 threads, group commit) ==");
    let dir = temp_dir("concurrent");
    {
        let total = 40_000usize;
        let db = Arc::new(SharedKiban::open(&dir).unwrap());
        let mut i = 0usize;
        measure("4-thread puts with batched sync", total, "ops", || {
            let end = i + total;
            let handles: Vec<_> = (0..4)
                .map(|t| {
                    let db = db.clone();
                    let base = end.div_ceil(4) * t;
                    std::thread::spawn(move || {
                        let my_end = end.div_ceil(4) * (t + 1);
                        let mut local = base;
                        while local < my_end {
                            db.put(format!("c{local:08}"), vec![b'v'; 40]).unwrap();
                            if local % 250 == 249 {
                                db.sync().unwrap();
                            }
                            local += 1;
                        }
                        db.sync().unwrap();
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            i = end;
        });
    }
    drop_dir(&dir);
}

fn drop_dir(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}
