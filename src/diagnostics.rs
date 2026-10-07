//! Opt-in, process-wide sampled lock acquisition timings.
//!
//! Enabled only by `diagnostics`; normal builds contain neither clocks nor
//! sampling. Each thread samples one in 1,024 acquisitions of each kind.
//! Durations overlap across callers and are not CPU time or lock hold time.
//! Profiled throughput must be labeled separately from ordinary builds.
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum Phase {
    EngineWrite,
    MemtableWrite,
    MemtableRead,
    ReadViewRead,
}
const N: usize = 4;
const NAMES: [&str; N] = [
    "engine_write",
    "memtable_write",
    "memtable_read",
    "read_view_read",
];
thread_local! { static TICKS: Cell<[u16; N]> = const { Cell::new([0; N]) }; }
static COUNTS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static TIMES: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static MAX: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

/// Observed sample totals, not extrapolated full-population totals.
#[derive(Debug, Clone, Copy)]
pub struct AcquisitionStats {
    pub phase: &'static str,
    pub samples: u64,
    pub elapsed_ns: u64,
    pub maximum_ns: u64,
}

/// Cumulative statistics across all databases in this process.
pub fn stats() -> [AcquisitionStats; N] {
    std::array::from_fn(|i| AcquisitionStats {
        phase: NAMES[i],
        samples: COUNTS[i].load(Relaxed),
        elapsed_ns: TIMES[i].load(Relaxed),
        maximum_ns: MAX[i].load(Relaxed),
    })
}

pub(crate) fn measure<T>(phase: Phase, operation: impl FnOnce() -> T) -> T {
    let i = phase as usize;
    let sample = TICKS.with(|ticks| {
        let mut t = ticks.get();
        let sample = t[i] == 0;
        t[i] = (t[i] + 1) & 1023;
        ticks.set(t);
        sample
    });
    if !sample {
        return operation();
    }
    let started = Instant::now();
    let result = operation();
    let ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    TIMES[i].fetch_add(ns, Relaxed);
    MAX[i].fetch_max(ns, Relaxed);
    COUNTS[i].fetch_add(1, Relaxed);
    result
}
