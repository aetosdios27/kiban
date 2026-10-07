//! Long-run steady-state torture bench (11.17-round-2, sections 1/2/8).
//!
//! Deliberately separate from `basic.rs`: this is meant to run long
//! enough (hundreds of thousands of ops, many flush/compaction cycles)
//! to force Kiban past whatever startup/empty-database advantages a
//! short microbenchmark never loses, and to compare compaction
//! scheduler configurations (A/B/C/D) against the SAME sustained
//! workload rather than a single short snapshot.
//!
//! Run with `cargo bench --bench steady_state`. Set
//! `KIBAN_BENCH_QUICK=1` for a fast smoke run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use kiban::db::{CompactionScheduler, KibanOptions, SharedKiban};

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiban-steady-{label}-{}", std::process::id()));
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

fn percentiles(mut samples: Vec<u64>) -> (u64, u64, u64, u64) {
    if samples.is_empty() {
        return (0, 0, 0, 0);
    }
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    (at(0.50), at(0.95), at(0.99), at(0.999))
}

/// Current process RSS in bytes, via `/proc/self/status` (Linux-only;
/// returns 0 if unavailable, which is fine — this is an observational
/// extra, not a correctness dependency).
fn current_rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .unwrap_or(0);
            return kb * 1024;
        }
    }
    0
}

/// Current process open-file count, via `/proc/self/fd` (Linux-only;
/// returns 0 if unavailable).
fn current_open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|it| it.count())
        .unwrap_or(0)
}

#[derive(Clone, Copy, Default, Debug)]
struct Sample {
    at: Duration,
    l0_files: usize,
    total_bytes: u64,
    write_stalls: u64,
    compactions: u64,
    rss_bytes: u64,
    open_fds: usize,
}

struct RunResult {
    label: String,
    get_p: (u64, u64, u64, u64),
    put_p: (u64, u64, u64, u64),
    get_ops: usize,
    put_ops: usize,
    wall: Duration,
    write_amp: f64,
    read_amp_avg_tables_probed: f64,
    final_write_stalls: u64,
    final_l0: usize,
    max_l0: usize,
    samples: Vec<Sample>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkloadShape {
    /// Small, overlapping key range with heavy overwrite/delete churn
    /// — the shape every run in this file used until this option
    /// existed. Stresses compaction the way a hot, narrow working set
    /// does.
    Churny,
    /// Monotonically increasing keys across a range far wider than any
    /// one compaction's overlap window, minimal overwrite — the
    /// opposite shape: mostly-new data, little redundant rewriting.
    /// Exists to check that the scored scheduler's write-amp win on
    /// `Churny` isn't a workload-specific artifact.
    Sequential,
}

fn run_steady_state(
    label: &str,
    options: KibanOptions,
    total_writes: usize,
    total_reads: usize,
    writers: usize,
    readers: usize,
) -> RunResult {
    run_steady_state_shaped(
        label,
        options,
        total_writes,
        total_reads,
        writers,
        readers,
        WorkloadShape::Churny,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_steady_state_shaped(
    label: &str,
    options: KibanOptions,
    total_writes: usize,
    total_reads: usize,
    writers: usize,
    readers: usize,
    shape: WorkloadShape,
) -> RunResult {
    let dir = temp_dir(label);
    let db = SharedKiban::open_with_options(&dir, options).unwrap();

    // A stable, pre-seeded "hot" keyspace the readers hammer throughout
    // the run — foreground read latency against data that already
    // exists, exactly what a real workload's steady-state traffic
    // looks like, not reads racing their own writer for freshly
    // inserted keys.
    let hot_keys = 20_000;
    for i in 0..hot_keys {
        db.put(key("hot", i), [b'h'; 64]).unwrap();
    }
    db.sync().unwrap();
    db.flush().unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let samples: Arc<Mutex<Vec<Sample>>> = Arc::new(Mutex::new(Vec::new()));
    let start_barrier = Arc::new(Barrier::new(writers + readers + 2));
    let t0 = Instant::now();

    let sampler = {
        let db = db.clone();
        let stop = stop.clone();
        let samples = samples.clone();
        let start_barrier = start_barrier.clone();
        std::thread::spawn(move || {
            start_barrier.wait();
            while !stop.load(Ordering::Relaxed) {
                if let Ok(stats) = db.stats() {
                    let l0_files = stats
                        .levels
                        .iter()
                        .find(|l| l.level == 0)
                        .map(|l| l.tables)
                        .unwrap_or(0);
                    let total_bytes = stats.levels.iter().map(|l| l.bytes).sum();
                    samples.lock().unwrap().push(Sample {
                        at: t0.elapsed(),
                        l0_files,
                        total_bytes,
                        write_stalls: stats.maintenance.write_stalls,
                        compactions: stats.maintenance.compactions_completed,
                        rss_bytes: current_rss_bytes(),
                        open_fds: current_open_fds(),
                    });
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };

    let before_stats = db.stats().unwrap();
    let before_read_amp = before_stats.read_amp;

    let writer_handles: Vec<_> = (0..writers)
        .map(|w| {
            let db = db.clone();
            let start_barrier = start_barrier.clone();
            let per_writer = total_writes / writers;
            std::thread::spawn(move || {
                start_barrier.wait();
                let mut lat = Vec::with_capacity(per_writer);
                for i in 0..per_writer {
                    let t = Instant::now();
                    match shape {
                        WorkloadShape::Churny => {
                            let idx = (i * 7919 + w * 104_729) % (per_writer * 4).max(1);
                            if idx.is_multiple_of(11) {
                                db.delete(key("churn", idx)).unwrap();
                            } else {
                                db.put(key("churn", idx), [b'w'; 80]).unwrap();
                            }
                        }
                        WorkloadShape::Sequential => {
                            let idx = w * per_writer + i;
                            db.put(key("seq", idx), [b'w'; 80]).unwrap();
                        }
                    }
                    lat.push(t.elapsed().as_nanos() as u64);
                }
                lat
            })
        })
        .collect();

    let reader_handles: Vec<_> = (0..readers)
        .map(|r| {
            let db = db.clone();
            let start_barrier = start_barrier.clone();
            let per_reader = total_reads / readers;
            std::thread::spawn(move || {
                start_barrier.wait();
                let mut lat = Vec::with_capacity(per_reader);
                for i in 0..per_reader {
                    let idx = (i.wrapping_mul(8191) + r * 8191) % hot_keys;
                    let t = Instant::now();
                    let got = db.get(key("hot", idx)).unwrap();
                    lat.push(t.elapsed().as_nanos() as u64);
                    assert!(got.is_some());
                }
                lat
            })
        })
        .collect();

    start_barrier.wait();
    let wall_start = Instant::now();
    let mut put_samples = Vec::with_capacity(total_writes);
    for h in writer_handles {
        put_samples.extend(h.join().unwrap());
    }
    let mut get_samples = Vec::with_capacity(total_reads);
    for h in reader_handles {
        get_samples.extend(h.join().unwrap());
    }
    let wall = wall_start.elapsed();

    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();

    let after_stats = db.stats().unwrap();
    let after_read_amp = after_stats.read_amp;

    let logical_bytes_written = put_samples.len() as u64 * 80;
    let physical_bytes_written = after_stats.maintenance.flush_output_bytes
        + after_stats.maintenance.compaction_output_bytes
        - before_stats.maintenance.flush_output_bytes
        - before_stats.maintenance.compaction_output_bytes;
    let write_amp = if logical_bytes_written > 0 {
        physical_bytes_written as f64 / logical_bytes_written as f64
    } else {
        0.0
    };

    let sst_gets = (after_read_amp.gets_total - before_read_amp.gets_total)
        .saturating_sub(after_read_amp.resolved_in_memtable - before_read_amp.resolved_in_memtable)
        .saturating_sub(
            after_read_amp.resolved_in_immutable - before_read_amp.resolved_in_immutable,
        );
    let tables_probed = (after_read_amp.l0_tables_probed - before_read_amp.l0_tables_probed)
        + (after_read_amp.leveled_tables_probed - before_read_amp.leveled_tables_probed);
    let read_amp_avg_tables_probed = if sst_gets > 0 {
        tables_probed as f64 / sst_gets as f64
    } else {
        0.0
    };

    let samples = Arc::try_unwrap(samples).unwrap().into_inner().unwrap();
    let max_l0 = samples.iter().map(|s| s.l0_files).max().unwrap_or(0);

    let result = RunResult {
        label: label.to_string(),
        get_p: percentiles(get_samples.clone()),
        put_p: percentiles(put_samples.clone()),
        get_ops: get_samples.len(),
        put_ops: put_samples.len(),
        wall,
        write_amp,
        read_amp_avg_tables_probed,
        final_write_stalls: after_stats.maintenance.write_stalls
            - before_stats.maintenance.write_stalls,
        final_l0: after_stats
            .levels
            .iter()
            .find(|l| l.level == 0)
            .map(|l| l.tables)
            .unwrap_or(0),
        max_l0,
        samples,
    };

    drop(db);
    drop_dir(&dir);
    result
}

fn print_result(r: &RunResult) {
    println!("\n-- {} --", r.label);
    println!(
        "  {} puts in {:?} ({:.0} ops/s), {} gets ({:.0} ops/s)",
        r.put_ops,
        r.wall,
        r.put_ops as f64 / r.wall.as_secs_f64(),
        r.get_ops,
        r.get_ops as f64 / r.wall.as_secs_f64(),
    );
    println!(
        "  GET p50={:>7.2}us p95={:>8.2}us p99={:>9.2}us p999={:>10.2}us",
        r.get_p.0 as f64 / 1000.0,
        r.get_p.1 as f64 / 1000.0,
        r.get_p.2 as f64 / 1000.0,
        r.get_p.3 as f64 / 1000.0,
    );
    println!(
        "  PUT p50={:>7.2}us p95={:>8.2}us p99={:>9.2}us p999={:>10.2}us",
        r.put_p.0 as f64 / 1000.0,
        r.put_p.1 as f64 / 1000.0,
        r.put_p.2 as f64 / 1000.0,
        r.put_p.3 as f64 / 1000.0,
    );
    println!(
        "  write_amp={:.2}x  avg SST tables probed/get={:.2}  write_stalls={}  L0 final={} max={}",
        r.write_amp, r.read_amp_avg_tables_probed, r.final_write_stalls, r.final_l0, r.max_l0,
    );
    if let (Some(first), Some(last)) = (r.samples.first(), r.samples.last()) {
        let peak_rss = r.samples.iter().map(|s| s.rss_bytes).max().unwrap_or(0);
        let peak_fds = r.samples.iter().map(|s| s.open_fds).max().unwrap_or(0);
        println!(
            "  RSS: start={:.1}MB peak={:.1}MB end={:.1}MB   open FDs: start={} peak={} end={}",
            first.rss_bytes as f64 / 1e6,
            peak_rss as f64 / 1e6,
            last.rss_bytes as f64 / 1e6,
            first.open_fds,
            peak_fds,
            last.open_fds,
        );
    }
    // Sawtooth check: bucket samples into deciles of elapsed time.
    // Healthy steady state shows a narrow, stable band in each row;
    // sawtooth shows buckets alternating between low and high with
    // growing amplitude (good -> debt builds -> latency explodes ->
    // emergency compaction -> repeat).
    if !r.samples.is_empty() {
        let total = r.samples.last().unwrap().at.as_secs_f64().max(0.001);
        #[derive(Clone, Copy)]
        struct Bucket {
            l0_lo: usize,
            l0_hi: usize,
            bytes_lo: u64,
            bytes_hi: u64,
            stalls_start: u64,
            stalls_end: u64,
            compactions_start: u64,
            compactions_end: u64,
        }
        let empty = Bucket {
            l0_lo: usize::MAX,
            l0_hi: 0,
            bytes_lo: u64::MAX,
            bytes_hi: 0,
            stalls_start: u64::MAX,
            stalls_end: 0,
            compactions_start: u64::MAX,
            compactions_end: 0,
        };
        let mut buckets = vec![empty; 10];
        for s in &r.samples {
            let b = (((s.at.as_secs_f64() / total) * 9.999) as usize).min(9);
            let e = &mut buckets[b];
            e.l0_lo = e.l0_lo.min(s.l0_files);
            e.l0_hi = e.l0_hi.max(s.l0_files);
            e.bytes_lo = e.bytes_lo.min(s.total_bytes);
            e.bytes_hi = e.bytes_hi.max(s.total_bytes);
            e.stalls_start = e.stalls_start.min(s.write_stalls);
            e.stalls_end = e.stalls_end.max(s.write_stalls);
            e.compactions_start = e.compactions_start.min(s.compactions);
            e.compactions_end = e.compactions_end.max(s.compactions);
        }
        print!("  L0 files/decile (min-max):     ");
        for e in &buckets {
            if e.l0_lo == usize::MAX {
                print!("      .");
            } else {
                print!(" {:>2}-{:<3}", e.l0_lo, e.l0_hi);
            }
        }
        println!();
        print!("  total MB/decile (min-max):     ");
        for e in &buckets {
            if e.bytes_lo == u64::MAX {
                print!("      .");
            } else {
                print!(
                    " {:>2.0}-{:<3.0}",
                    e.bytes_lo as f64 / 1e6,
                    e.bytes_hi as f64 / 1e6
                );
            }
        }
        println!();
        print!("  compactions completed/decile:  ");
        for e in &buckets {
            if e.compactions_start == u64::MAX {
                print!("      .");
            } else {
                print!(" {:>6}", e.compactions_end - e.compactions_start);
            }
        }
        println!();
        print!("  write stalls/decile:           ");
        for e in &buckets {
            if e.stalls_start == u64::MAX {
                print!("      .");
            } else {
                print!(" {:>6}", e.stalls_end - e.stalls_start);
            }
        }
        println!();
    }
}

fn main() {
    let quick = std::env::var_os("KIBAN_BENCH_QUICK").is_some();
    let (total_writes, total_reads) = if quick {
        (20_000, 20_000)
    } else {
        (300_000, 300_000)
    };
    let writers = 4;
    let readers = 4;

    // Deliberately tight buffers (not the defaults) — this is a
    // maintenance-churn torture bench, the same shape as the prior
    // round's 11.17-D stress config, just run far longer so steady
    // state (not startup transients) dominates the numbers.
    let base = KibanOptions {
        write_buffer_bytes: 64 * 1024,
        target_file_size: 64 * 1024,
        base_level_bytes: 256 * 1024,
        level_multiplier: 4,
        l0_compaction_trigger: 4,
        l0_write_stall_trigger: 16,
        block_cache_bytes: 2 * 1024 * 1024,
        ..KibanOptions::default()
    };

    println!(
        "Kiban steady-state torture: {total_writes} writes ({writers} writers) / {total_reads} gets ({readers} readers)"
    );

    let a = run_steady_state(
        "A: fixed priority, no pacing",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::FixedPriority,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: false,
            ..base.clone()
        },
        total_writes,
        total_reads,
        writers,
        readers,
    );
    print_result(&a);

    let b = run_steady_state(
        "B: fixed priority + pacing",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::FixedPriority,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: true,
            ..base.clone()
        },
        total_writes,
        total_reads,
        writers,
        readers,
    );
    print_result(&b);

    let c = run_steady_state(
        "C: scored scheduler + pacing",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Scored,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: true,
            ..base.clone()
        },
        total_writes,
        total_reads,
        writers,
        readers,
    );
    print_result(&c);

    let d = run_steady_state(
        "D: scored scheduler + batching(4) + pacing",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Scored,
            compaction_batch_size: 4,
            maintenance_pacing_enabled: true,
            ..base.clone()
        },
        total_writes,
        total_reads,
        writers,
        readers,
    );
    print_result(&d);

    let e = run_steady_state(
        "E: governor (11.17-round-3)",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Governor,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: true,
            ..base
        },
        total_writes,
        total_reads,
        writers,
        readers,
    );
    print_result(&e);

    println!("\n== Summary ==");
    println!(
        "{:<45} {:>12} {:>12} {:>10} {:>10}",
        "config", "GET p99(us)", "PUT p99(us)", "WA", "stalls"
    );
    for r in [&a, &b, &c, &d, &e] {
        println!(
            "{:<45} {:>12.2} {:>12.2} {:>10.2} {:>10}",
            r.label,
            r.get_p.2 as f64 / 1000.0,
            r.put_p.2 as f64 / 1000.0,
            r.write_amp,
            r.final_write_stalls,
        );
    }

    // Robustness check: is the scored scheduler's write-amp win on the
    // churny workload specific to that shape, or does it hold (or at
    // least not regress) on a mostly-new-data, low-overwrite workload
    // too? A vs D only, same options otherwise.
    println!(
        "\n\nKiban steady-state torture (SEQUENTIAL, low-churn workload): {total_writes} writes / {total_reads} gets"
    );
    let a_seq = run_steady_state_shaped(
        "A-seq: fixed priority, no pacing",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::FixedPriority,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: false,
            ..base.clone()
        },
        total_writes,
        total_reads,
        writers,
        readers,
        WorkloadShape::Sequential,
    );
    print_result(&a_seq);
    let d_seq = run_steady_state_shaped(
        "D-seq: scored scheduler + batching(4) + pacing",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Scored,
            compaction_batch_size: 4,
            maintenance_pacing_enabled: true,
            ..base.clone()
        },
        total_writes,
        total_reads,
        writers,
        readers,
        WorkloadShape::Sequential,
    );
    print_result(&d_seq);
    let e_seq = run_steady_state_shaped(
        "E-seq: governor (11.17-round-3)",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Governor,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: true,
            ..base
        },
        total_writes,
        total_reads,
        writers,
        readers,
        WorkloadShape::Sequential,
    );
    print_result(&e_seq);
    println!("\n== Sequential-workload summary ==");
    println!(
        "{:<45} {:>12} {:>12} {:>10} {:>10}",
        "config", "GET p99(us)", "PUT p99(us)", "WA", "stalls"
    );
    for r in [&a_seq, &d_seq, &e_seq] {
        println!(
            "{:<45} {:>12.2} {:>12.2} {:>10.2} {:>10}",
            r.label,
            r.get_p.2 as f64 / 1000.0,
            r.put_p.2 as f64 / 1000.0,
            r.write_amp,
            r.final_write_stalls,
        );
    }
}
