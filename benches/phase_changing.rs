//! Phase-changing torture bench (11.17-round-3, section 9).
//!
//! `steady_state.rs` compares schedulers on a *stationary* workload
//! (one shape, the whole run). The governor's whole premise is that it
//! adapts as pressure changes — a stationary benchmark cannot tell
//! `Governor` apart from "whichever static policy happens to suit that
//! one shape." This bench instead cycles one long run through six
//! workload phases and reports GET/PUT tail latency, write/read
//! amplification, L0/backlog, and (for `Governor`) mode occupancy and
//! adaptive-batch/trivial-move activity, bucketed PER PHASE so
//! oscillation and lag are visible, not just a final average.
//!
//! Run with `cargo bench --bench phase_changing`. Set
//! `KIBAN_BENCH_QUICK=1` for a fast smoke run.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use kiban::db::{CompactionScheduler, KibanOptions, SharedKiban};
use kiban::governor::GovernorMode;

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiban-phasechg-{label}-{}", std::process::id()));
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

fn current_open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|it| it.count())
        .unwrap_or(0)
}

/// A cheap, deterministic per-thread PRNG (xorshift64) — good enough
/// for duty-cycle gating and churn-key selection; no need for anything
/// cryptographic here.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() % 1_000_000) as f64 / 1_000_000.0
    }
}

#[derive(Clone, Copy)]
struct Phase {
    name: &'static str,
    /// Fraction of the time a writer thread actually performs a write
    /// this tick rather than a short idle sleep — how "write-burst" vs
    /// "read-heavy" this phase is.
    writer_duty: f64,
    reader_duty: f64,
    /// Narrow overlapping range + occasional deletes (churn) vs a
    /// monotonically advancing per-thread range (sequential/new data).
    churny: bool,
}

const PHASES: [Phase; 6] = [
    Phase {
        name: "1-read-heavy",
        writer_duty: 0.05,
        reader_duty: 1.0,
        churny: false,
    },
    Phase {
        name: "2-write-burst",
        writer_duty: 1.0,
        reader_duty: 0.05,
        churny: false,
    },
    Phase {
        name: "3-mixed",
        writer_duty: 0.6,
        reader_duty: 0.6,
        churny: false,
    },
    Phase {
        name: "4-overwrite-delete-churn",
        writer_duty: 1.0,
        reader_duty: 0.3,
        churny: true,
    },
    Phase {
        name: "5-sequential-new-writes",
        writer_duty: 1.0,
        reader_duty: 0.3,
        churny: false,
    },
    Phase {
        name: "6-read-heavy-recovery",
        writer_duty: 0.05,
        reader_duty: 1.0,
        churny: false,
    },
];

#[derive(Clone, Copy, Default, Debug)]
struct StructSample {
    phase: usize,
    l0_files: usize,
    total_bytes: u64,
    write_stalls: u64,
    compactions: u64,
    rss_bytes: u64,
    open_fds: usize,
    governor_mode: Option<GovernorMode>,
}

#[derive(Clone, Copy, Debug)]
struct OpSample {
    phase: usize,
    lat_ns: u64,
}

struct PhaseMetrics {
    name: &'static str,
    get_p: (u64, u64, u64, u64),
    put_p: (u64, u64, u64, u64),
    get_ops: usize,
    put_ops: usize,
    mode_ticks: [usize; 3],
    l0_min: usize,
    l0_max: usize,
    bytes_min: u64,
    bytes_max: u64,
    stalls_delta: u64,
    compactions_delta: u64,
}

struct RunResult {
    label: String,
    phases: Vec<PhaseMetrics>,
    write_amp: f64,
    read_amp_avg_tables_probed: f64,
    final_write_stalls: u64,
    max_l0: usize,
    governor_mode_switches: u64,
    governor_avg_batch_size: f64,
    governor_trivial_moves: u64,
    governor_trivial_bytes_avoided: u64,
    governor_predicted_actual_samples: u64,
    governor_predicted_actual_mape: f64,
    peak_rss_mb: f64,
    peak_fds: usize,
}

fn run_phase_changing(
    label: &str,
    options: KibanOptions,
    phase_duration: Duration,
    writers: usize,
    readers: usize,
) -> RunResult {
    let dir = temp_dir(label);
    let db = SharedKiban::open_with_options(&dir, options).unwrap();

    let hot_keys = 20_000usize;
    for i in 0..hot_keys {
        db.put(key("hot", i), [b'h'; 64]).unwrap();
    }
    db.sync().unwrap();
    db.flush().unwrap();

    let current_phase = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let struct_samples: Arc<Mutex<Vec<StructSample>>> = Arc::new(Mutex::new(Vec::new()));
    let get_samples: Arc<Mutex<Vec<OpSample>>> = Arc::new(Mutex::new(Vec::new()));
    let put_samples: Arc<Mutex<Vec<OpSample>>> = Arc::new(Mutex::new(Vec::new()));
    // +3: clock, sampler, AND main itself (main also calls `.wait()`
    // below, right before joining) — all three are real rendezvous
    // parties, not just the two background threads. Getting this wrong
    // by one caused a real hang: `Barrier` is capacity-exact, so with
    // 11 real callers against a 10-capacity barrier, exactly one
    // (whichever happened to arrive last) would block forever waiting
    // for a second round that could never complete.
    let start_barrier = Arc::new(Barrier::new(writers + readers + 3));

    let before_stats = db.stats().unwrap();
    let before_read_amp = before_stats.read_amp;

    // Phase clock: advances `current_phase` on a wall-clock timer, then
    // signals stop once the last phase's duration has elapsed.
    let clock = {
        let current_phase = current_phase.clone();
        let stop = stop.clone();
        let start_barrier = start_barrier.clone();
        std::thread::spawn(move || {
            start_barrier.wait();
            for i in 0..PHASES.len() {
                current_phase.store(i, Ordering::Relaxed);
                std::thread::sleep(phase_duration);
            }
            stop.store(true, Ordering::Relaxed);
        })
    };

    let sampler = {
        let db = db.clone();
        let stop = stop.clone();
        let current_phase = current_phase.clone();
        let struct_samples = struct_samples.clone();
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
                    struct_samples.lock().unwrap().push(StructSample {
                        phase: current_phase.load(Ordering::Relaxed),
                        l0_files,
                        total_bytes,
                        write_stalls: stats.maintenance.write_stalls,
                        compactions: stats.maintenance.compactions_completed,
                        rss_bytes: current_rss_bytes(),
                        open_fds: current_open_fds(),
                        governor_mode: Some(stats.governor.mode),
                    });
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };

    let writer_handles: Vec<_> = (0..writers)
        .map(|w| {
            let db = db.clone();
            let current_phase = current_phase.clone();
            let stop = stop.clone();
            let start_barrier = start_barrier.clone();
            let put_samples = put_samples.clone();
            std::thread::spawn(move || {
                let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (w as u64).wrapping_mul(0xff51_afd7));
                let mut seq_cursor = 0usize;
                let mut local: Vec<OpSample> = Vec::new();
                start_barrier.wait();
                while !stop.load(Ordering::Relaxed) {
                    let p = current_phase.load(Ordering::Relaxed).min(PHASES.len() - 1);
                    let phase = PHASES[p];
                    if rng.next_f64() > phase.writer_duty {
                        std::thread::sleep(Duration::from_micros(200));
                        continue;
                    }
                    let t = Instant::now();
                    if phase.churny {
                        let idx = (rng.next_u64() as usize) % 2_000;
                        if idx.is_multiple_of(11) {
                            let _ = db.delete(key("churn", idx));
                        } else {
                            let _ = db.put(key("churn", idx), [b'w'; 80]);
                        }
                    } else {
                        let idx = w * 10_000_000 + seq_cursor;
                        seq_cursor += 1;
                        let _ = db.put(key("seq", idx), [b'w'; 80]);
                    }
                    local.push(OpSample {
                        phase: p,
                        lat_ns: t.elapsed().as_nanos() as u64,
                    });
                }
                put_samples.lock().unwrap().extend(local);
            })
        })
        .collect();

    let reader_handles: Vec<_> = (0..readers)
        .map(|r| {
            let db = db.clone();
            let current_phase = current_phase.clone();
            let stop = stop.clone();
            let start_barrier = start_barrier.clone();
            let get_samples = get_samples.clone();
            std::thread::spawn(move || {
                let mut rng = Rng(0x1234_5678_9abc_def0 ^ (r as u64).wrapping_mul(0xc2b2_ae3d));
                let mut i = 0usize;
                let mut local: Vec<OpSample> = Vec::new();
                start_barrier.wait();
                while !stop.load(Ordering::Relaxed) {
                    let p = current_phase.load(Ordering::Relaxed).min(PHASES.len() - 1);
                    let phase = PHASES[p];
                    if rng.next_f64() > phase.reader_duty {
                        std::thread::sleep(Duration::from_micros(200));
                        continue;
                    }
                    let idx = (i.wrapping_mul(8191) + r * 8191) % hot_keys;
                    i += 1;
                    let t = Instant::now();
                    let got = db.get(key("hot", idx)).unwrap();
                    local.push(OpSample {
                        phase: p,
                        lat_ns: t.elapsed().as_nanos() as u64,
                    });
                    assert!(got.is_some());
                }
                get_samples.lock().unwrap().extend(local);
            })
        })
        .collect();

    start_barrier.wait();
    clock.join().unwrap();
    for h in writer_handles {
        h.join().unwrap();
    }
    for h in reader_handles {
        h.join().unwrap();
    }
    sampler.join().unwrap();

    let after_stats = db.stats().unwrap();
    let after_read_amp = after_stats.read_amp;
    let get_samples = Arc::try_unwrap(get_samples).unwrap().into_inner().unwrap();
    let put_samples = Arc::try_unwrap(put_samples).unwrap().into_inner().unwrap();
    let struct_samples = Arc::try_unwrap(struct_samples)
        .unwrap()
        .into_inner()
        .unwrap();

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

    let mut phases = Vec::with_capacity(PHASES.len());
    for (idx, p) in PHASES.iter().enumerate() {
        let gets: Vec<u64> = get_samples
            .iter()
            .filter(|s| s.phase == idx)
            .map(|s| s.lat_ns)
            .collect();
        let puts: Vec<u64> = put_samples
            .iter()
            .filter(|s| s.phase == idx)
            .map(|s| s.lat_ns)
            .collect();
        let get_ops = gets.len();
        let put_ops = puts.len();
        let mut mode_ticks = [0usize; 3];
        let (mut l0_min, mut l0_max) = (usize::MAX, 0usize);
        let (mut bytes_min, mut bytes_max) = (u64::MAX, 0u64);
        let (mut stalls_lo, mut stalls_hi) = (u64::MAX, 0u64);
        let (mut compactions_lo, mut compactions_hi) = (u64::MAX, 0u64);
        for s in struct_samples.iter().filter(|s| s.phase == idx) {
            if let Some(m) = s.governor_mode {
                mode_ticks[mode_idx(m)] += 1;
            }
            l0_min = l0_min.min(s.l0_files);
            l0_max = l0_max.max(s.l0_files);
            bytes_min = bytes_min.min(s.total_bytes);
            bytes_max = bytes_max.max(s.total_bytes);
            stalls_lo = stalls_lo.min(s.write_stalls);
            stalls_hi = stalls_hi.max(s.write_stalls);
            compactions_lo = compactions_lo.min(s.compactions);
            compactions_hi = compactions_hi.max(s.compactions);
        }
        phases.push(PhaseMetrics {
            name: p.name,
            get_p: percentiles(gets),
            put_p: percentiles(puts),
            get_ops,
            put_ops,
            mode_ticks,
            l0_min: if l0_min == usize::MAX { 0 } else { l0_min },
            l0_max,
            bytes_min: if bytes_min == u64::MAX { 0 } else { bytes_min },
            bytes_max,
            stalls_delta: stalls_hi.saturating_sub(if stalls_lo == u64::MAX {
                0
            } else {
                stalls_lo
            }),
            compactions_delta: compactions_hi.saturating_sub(if compactions_lo == u64::MAX {
                0
            } else {
                compactions_lo
            }),
        });
    }

    let max_l0 = struct_samples.iter().map(|s| s.l0_files).max().unwrap_or(0);
    let peak_rss_mb = struct_samples
        .iter()
        .map(|s| s.rss_bytes)
        .max()
        .unwrap_or(0) as f64
        / 1e6;
    let peak_fds = struct_samples.iter().map(|s| s.open_fds).max().unwrap_or(0);
    let gstats = after_stats.governor;

    let result = RunResult {
        label: label.to_string(),
        phases,
        write_amp,
        read_amp_avg_tables_probed,
        final_write_stalls: after_stats.maintenance.write_stalls
            - before_stats.maintenance.write_stalls,
        max_l0,
        governor_mode_switches: gstats.mode_switches,
        governor_avg_batch_size: gstats.avg_batch_size,
        governor_trivial_moves: gstats.trivial_moves_performed,
        governor_trivial_bytes_avoided: gstats.trivial_move_bytes_avoided,
        governor_predicted_actual_samples: gstats.predicted_actual_samples,
        governor_predicted_actual_mape: gstats.predicted_actual_mean_abs_pct_error,
        peak_rss_mb,
        peak_fds,
    };

    drop(db);
    drop_dir(&dir);
    result
}

fn mode_idx(m: GovernorMode) -> usize {
    match m {
        GovernorMode::Balanced => 0,
        GovernorMode::ReadProtect => 1,
        GovernorMode::WriteEmergency => 2,
    }
}

fn mode_label(idx: usize) -> &'static str {
    match idx {
        0 => "BAL",
        1 => "RDP",
        2 => "WEM",
        _ => "?",
    }
}

fn print_result(r: &RunResult) {
    println!("\n-- {} --", r.label);
    println!(
        "  write_amp={:.2}x  avg SST tables probed/get={:.2}  write_stalls={}  max L0={}",
        r.write_amp, r.read_amp_avg_tables_probed, r.final_write_stalls, r.max_l0,
    );
    println!(
        "  RSS peak={:.1}MB  open FDs peak={}",
        r.peak_rss_mb, r.peak_fds
    );
    if r.governor_mode_switches > 0
        || r.governor_trivial_moves > 0
        || r.governor_predicted_actual_samples > 0
    {
        println!(
            "  governor: {} mode switches, avg batch size={:.2}, {} trivial moves ({} bytes avoided), predicted-vs-actual: {} samples, MAPE={:.1}%",
            r.governor_mode_switches,
            r.governor_avg_batch_size,
            r.governor_trivial_moves,
            r.governor_trivial_bytes_avoided,
            r.governor_predicted_actual_samples,
            r.governor_predicted_actual_mape * 100.0,
        );
    }
    println!(
        "  {:<28} {:>10} {:>10} {:>10} {:>10} {:>10} {:>9} {:>9} {:>9}",
        "phase", "GET p50", "p99", "p999", "PUT p50", "p99", "get n", "put n", "mode(BAL/RDP/WEM)"
    );
    for p in &r.phases {
        let total_ticks: usize = p.mode_ticks.iter().sum();
        let mode_str = if let Some(total) = std::num::NonZeroUsize::new(total_ticks) {
            format!(
                "{:>3}%/{:>3}%/{:>3}%",
                p.mode_ticks[0] * 100 / total,
                p.mode_ticks[1] * 100 / total,
                p.mode_ticks[2] * 100 / total,
            )
        } else {
            "  .  /  .  /  . ".to_string()
        };
        println!(
            "  {:<28} {:>8.2}us {:>7.2}us {:>7.2}us {:>8.2}us {:>7.2}us {:>10} {:>9} {}",
            p.name,
            p.get_p.0 as f64 / 1000.0,
            p.get_p.2 as f64 / 1000.0,
            p.get_p.3 as f64 / 1000.0,
            p.put_p.0 as f64 / 1000.0,
            p.put_p.2 as f64 / 1000.0,
            p.get_ops,
            p.put_ops,
            mode_str,
        );
    }
    println!(
        "  {:<28} {:>12} {:>16} {:>10} {:>14}",
        "phase", "L0 (min-max)", "total MB (min-max)", "+stalls", "+compactions"
    );
    for p in &r.phases {
        println!(
            "  {:<28} {:>5}-{:<6} {:>7.1}-{:<7.1} {:>10} {:>14}",
            p.name,
            p.l0_min,
            p.l0_max,
            p.bytes_min as f64 / 1e6,
            p.bytes_max as f64 / 1e6,
            p.stalls_delta,
            p.compactions_delta,
        );
    }
    let _ = mode_label; // used only if per-tick detail is added later
}

fn main() {
    let quick = std::env::var_os("KIBAN_BENCH_QUICK").is_some();
    let phase_duration = if quick {
        Duration::from_millis(500)
    } else {
        Duration::from_secs(6)
    };
    let writers = 4;
    let readers = 4;

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
        "Kiban phase-changing torture: {} phases x {:?} each, {writers} writers / {readers} readers",
        PHASES.len(),
        phase_duration
    );
    for p in &PHASES {
        println!(
            "  {} (writer_duty={:.2} reader_duty={:.2} churny={})",
            p.name, p.writer_duty, p.reader_duty, p.churny
        );
    }

    let a = run_phase_changing(
        "A-fixed",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::FixedPriority,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: true,
            ..base.clone()
        },
        phase_duration,
        writers,
        readers,
    );
    print_result(&a);

    let b = run_phase_changing(
        "B-scored+batch4",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Scored,
            compaction_batch_size: 4,
            maintenance_pacing_enabled: true,
            ..base.clone()
        },
        phase_duration,
        writers,
        readers,
    );
    print_result(&b);

    let c = run_phase_changing(
        "C-governor",
        KibanOptions {
            compaction_scheduler: CompactionScheduler::Governor,
            compaction_batch_size: 1,
            maintenance_pacing_enabled: true,
            ..base
        },
        phase_duration,
        writers,
        readers,
    );
    print_result(&c);

    println!("\n== Overall summary (all phases combined) ==");
    println!(
        "{:<20} {:>10} {:>10} {:>10} {:>10}",
        "config", "WA", "avg probe", "stalls", "max L0"
    );
    for r in [&a, &b, &c] {
        println!(
            "{:<20} {:>10.2} {:>10.2} {:>10} {:>10}",
            r.label, r.write_amp, r.read_amp_avg_tables_probed, r.final_write_stalls, r.max_l0,
        );
    }

    println!("\n== Per-phase GET p99 (us) ==");
    print!("{:<20}", "config");
    for p in &PHASES {
        print!(" {:>18}", p.name);
    }
    println!();
    for r in [&a, &b, &c] {
        print!("{:<20}", r.label);
        for p in &r.phases {
            print!(" {:>16.2}us", p.get_p.2 as f64 / 1000.0);
        }
        println!();
    }

    println!("\n== Per-phase PUT p99 (us) ==");
    print!("{:<20}", "config");
    for p in &PHASES {
        print!(" {:>18}", p.name);
    }
    println!();
    for r in [&a, &b, &c] {
        print!("{:<20}", r.label);
        for p in &r.phases {
            print!(" {:>16.2}us", p.put_p.2 as f64 / 1000.0);
        }
        println!();
    }
}
