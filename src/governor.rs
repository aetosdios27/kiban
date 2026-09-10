//! Adaptive compaction governor (11.17-round-3).
//!
//! `FixedPriority` and `Scored` (+ `compaction_batch_size`) are each a
//! single, permanent objective: tight predictable L0 (good GET tail,
//! high write-amp) or aggressive amortization (low write-amp, wider L0
//! tail). Measured on `benches/steady_state.rs`, neither dominates the
//! other. The governor is a third scheduler that tracks engine pressure
//! and switches among three objectives — `Balanced`, `ReadProtect`,
//! `WriteEmergency` — instead of committing to one tradeoff for the
//! whole process lifetime.
//!
//! Kept as a self-contained, pure module wherever the logic allows it:
//! `GovernorState::observe` (the mode FSM) and the candidate
//! generation/estimation/scoring functions all operate on plain owned
//! data (`TableInfo`, not `Arc<TableEntry>`), so they are unit-tested
//! directly with synthetic inputs — no real `Kiban`, no disk, no
//! engine lock. `crate::db` adapts real engine state into these plain
//! types and back into a `CompactionPlan`; see
//! `Kiban::plan_next_compaction_governor`.

use crate::db::ReadAmpStats;

/// Which objective the governor is currently pursuing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GovernorMode {
    #[default]
    Balanced,
    ReadProtect,
    WriteEmergency,
}

impl GovernorMode {
    fn idx(self) -> usize {
        match self {
            GovernorMode::Balanced => 0,
            GovernorMode::ReadProtect => 1,
            GovernorMode::WriteEmergency => 2,
        }
    }
}

/// One PLAN-time observation of engine pressure. Cheap to build from
/// state `Kiban` already has in hand — no new I/O, no per-get hot-path
/// cost (see `GovernorState::observe_engine`, which derives
/// `tables_probed_per_get` from the existing `ReadAmpCounters`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PressureSample {
    pub l0_count: usize,
    pub l0_write_stall_trigger: usize,
    /// Worst (bytes / budget) ratio over every level >= 1 currently
    /// over budget; 0.0 if none are.
    pub max_level_debt_ratio: f64,
    /// Tables probed per SST-resolved get, *since the last sample* (a
    /// window, not the cumulative average) — the direct, already-
    /// measured driver of GET tail latency in this engine (every extra
    /// table probed on the SST path is real added foreground work), so
    /// it stands in for a live GET p99 signal without adding any new
    /// per-get instrumentation.
    pub tables_probed_per_get: f64,
}

// ---- FSM tuning constants -------------------------------------------
//
// Chosen from `benches/steady_state.rs` (churny + sequential) and
// `benches/phase_changing.rs` (all six phases), not from any single
// benchmark — see `docs/design/compaction.md` governor section and the
// session's final report for the evidence. Kept few and named for what
// they mean, not fit to squeeze one more point out of one workload.

/// EWMA smoothing factor for `tables_probed_per_get`. 0.3 gives roughly
/// a 3-sample memory: responsive enough to notice a real shift within a
/// handful of PLAN calls, slow enough that one noisy window can't flip
/// the read-pressure signal by itself (that's what the streak counters
/// below are for, on top of this).
const EWMA_ALPHA: f64 = 0.3;

/// WRITE_EMERGENCY is a survival mode, not a smoothed objective: it
/// reacts to the instantaneous (unsmoothed) L0-vs-stall-trigger
/// fraction and level debt ratio, because by the time an EWMA would
/// catch up, backpressure may already be stalling writers.
const WRITE_EMERGENCY_ENTER_L0_FRACTION: f64 = 0.75;
const WRITE_EMERGENCY_ENTER_DEBT_RATIO: f64 = 3.0;
/// Exit thresholds are strictly lower than entry (hysteresis band) and
/// additionally require `WRITE_EMERGENCY_EXIT_WINDOWS` consecutive
/// samples below them — "exit only after pressure falls below a LOWER
/// threshold for a sustained period."
const WRITE_EMERGENCY_EXIT_L0_FRACTION: f64 = 0.45;
const WRITE_EMERGENCY_EXIT_DEBT_RATIO: f64 = 1.5;
const WRITE_EMERGENCY_EXIT_WINDOWS: u32 = 3;

/// BALANCED <-> READ_PROTECT uses the EWMA'd signal plus its own streak
/// requirement in both directions.
const READ_PROTECT_ENTER_TABLES_PROBED: f64 = 1.6;
const READ_PROTECT_ENTER_WINDOWS: u32 = 3;
const READ_PROTECT_EXIT_TABLES_PROBED: f64 = 1.15;
const READ_PROTECT_EXIT_WINDOWS: u32 = 4;

/// Prefix lengths considered per level batch candidate: 1, 2, 3, 4 —
/// exactly the task's own `[A] [A,B] [A,B,C] [A,B,C,D]` example. A
/// fixed small cap, not a search: linear in this constant, never
/// combinatorial.
const MAX_ADAPTIVE_BATCH: usize = 4;

/// Cumulative, monotonic bookkeeping plus the FSM's smoothed signal and
/// hysteresis counters. One instance lives on `Kiban` for the whole
/// engine lifetime (mirrors `MaintenanceStats`' "raw counters, never
/// reset" convention).
pub(crate) struct GovernorState {
    mode: GovernorMode,
    tables_probed_ewma: f64,
    enter_read_protect_streak: u32,
    exit_read_protect_streak: u32,
    exit_emergency_streak: u32,
    mode_switches: u64,
    time_in_mode: [std::time::Duration; 3],
    last_transition: std::time::Instant,
    // `observe_engine`'s windowing baseline (cumulative counters as of
    // the last call).
    last_probed: u64,
    last_resolved: u64,
    // Reporting-only accumulators (section 9/final report).
    batch_sizes_chosen: Vec<usize>,
    trivial_moves_performed: u64,
    trivial_move_bytes_avoided: u64,
    predicted_actual_abs_pct_error_sum: f64,
    predicted_actual_samples: u64,
}

impl GovernorState {
    pub(crate) fn new() -> Self {
        GovernorState {
            mode: GovernorMode::Balanced,
            tables_probed_ewma: 0.0,
            enter_read_protect_streak: 0,
            exit_read_protect_streak: 0,
            exit_emergency_streak: 0,
            mode_switches: 0,
            time_in_mode: [std::time::Duration::ZERO; 3],
            last_transition: std::time::Instant::now(),
            last_probed: 0,
            last_resolved: 0,
            batch_sizes_chosen: Vec::new(),
            trivial_moves_performed: 0,
            trivial_move_bytes_avoided: 0,
            predicted_actual_abs_pct_error_sum: 0.0,
            predicted_actual_samples: 0,
        }
    }

    pub(crate) fn mode(&self) -> GovernorMode {
        self.mode
    }

    /// The pure FSM step: given one pressure sample, updates the EWMA
    /// and hysteresis counters and returns the (possibly just-changed)
    /// mode. No I/O, no wall-clock dependence in the *decision* itself
    /// (only bookkeeping for reporting touches `Instant`) — fully
    /// deterministic given a sequence of samples, which is what makes
    /// the FSM tests below possible without a real engine.
    pub(crate) fn observe(&mut self, sample: PressureSample) -> GovernorMode {
        self.tables_probed_ewma = EWMA_ALPHA * sample.tables_probed_per_get
            + (1.0 - EWMA_ALPHA) * self.tables_probed_ewma;

        let l0_fraction = if sample.l0_write_stall_trigger > 0 {
            sample.l0_count as f64 / sample.l0_write_stall_trigger as f64
        } else {
            0.0
        };
        let emergency_now = l0_fraction >= WRITE_EMERGENCY_ENTER_L0_FRACTION
            || sample.max_level_debt_ratio >= WRITE_EMERGENCY_ENTER_DEBT_RATIO;
        let recovered = l0_fraction <= WRITE_EMERGENCY_EXIT_L0_FRACTION
            && sample.max_level_debt_ratio <= WRITE_EMERGENCY_EXIT_DEBT_RATIO;

        let new_mode = match self.mode {
            GovernorMode::WriteEmergency => {
                if recovered {
                    self.exit_emergency_streak += 1;
                } else {
                    self.exit_emergency_streak = 0;
                }
                if self.exit_emergency_streak >= WRITE_EMERGENCY_EXIT_WINDOWS {
                    GovernorMode::Balanced
                } else {
                    GovernorMode::WriteEmergency
                }
            }
            GovernorMode::Balanced if emergency_now => GovernorMode::WriteEmergency,
            GovernorMode::Balanced => {
                if self.tables_probed_ewma >= READ_PROTECT_ENTER_TABLES_PROBED {
                    self.enter_read_protect_streak += 1;
                } else {
                    self.enter_read_protect_streak = 0;
                }
                if self.enter_read_protect_streak >= READ_PROTECT_ENTER_WINDOWS {
                    GovernorMode::ReadProtect
                } else {
                    GovernorMode::Balanced
                }
            }
            GovernorMode::ReadProtect if emergency_now => GovernorMode::WriteEmergency,
            GovernorMode::ReadProtect => {
                if self.tables_probed_ewma <= READ_PROTECT_EXIT_TABLES_PROBED {
                    self.exit_read_protect_streak += 1;
                } else {
                    self.exit_read_protect_streak = 0;
                }
                if self.exit_read_protect_streak >= READ_PROTECT_EXIT_WINDOWS {
                    GovernorMode::Balanced
                } else {
                    GovernorMode::ReadProtect
                }
            }
        };

        self.record_transition(new_mode);
        self.mode = new_mode;
        new_mode
    }

    fn record_transition(&mut self, new_mode: GovernorMode) {
        let now = std::time::Instant::now();
        self.time_in_mode[self.mode.idx()] += now.duration_since(self.last_transition);
        self.last_transition = now;
        if new_mode != self.mode {
            self.mode_switches += 1;
            self.enter_read_protect_streak = 0;
            self.exit_read_protect_streak = 0;
            self.exit_emergency_streak = 0;
        }
    }

    /// Non-pure adapter: derives `tables_probed_per_get` as a windowed
    /// average since the last call from `ReadAmpCounters`' cumulative
    /// snapshot, then delegates to `observe`. This is the only method
    /// `crate::db` calls in production; `observe` itself stays testable
    /// in isolation.
    pub(crate) fn observe_engine(
        &mut self,
        l0_count: usize,
        l0_write_stall_trigger: usize,
        max_level_debt_ratio: f64,
        read_amp_now: ReadAmpStats,
    ) -> GovernorMode {
        let probed_now = read_amp_now.l0_tables_probed + read_amp_now.leveled_tables_probed;
        let resolved_sst_now = read_amp_now
            .gets_total
            .saturating_sub(read_amp_now.resolved_in_memtable)
            .saturating_sub(read_amp_now.resolved_in_immutable);
        let d_probed = probed_now.saturating_sub(self.last_probed);
        let d_resolved = resolved_sst_now.saturating_sub(self.last_resolved);
        let tables_probed_per_get = if d_resolved > 0 {
            d_probed as f64 / d_resolved as f64
        } else {
            // No new SST-path gets this window: hold the EWMA steady
            // rather than feeding it a fabricated 0.0 (which would read
            // as "read pressure vanished" when really there just were
            // no reads to measure).
            self.tables_probed_ewma
        };
        self.last_probed = probed_now;
        self.last_resolved = resolved_sst_now;
        self.observe(PressureSample {
            l0_count,
            l0_write_stall_trigger,
            max_level_debt_ratio,
            tables_probed_per_get,
        })
    }

    pub(crate) fn record_candidate_chosen(&mut self, kind: CandidateKind) {
        if let CandidateKind::LevelBatch { batch_len, .. } = kind {
            self.batch_sizes_chosen.push(batch_len);
        }
    }

    pub(crate) fn record_trivial_move(&mut self, bytes_avoided: u64) {
        self.trivial_moves_performed += 1;
        self.trivial_move_bytes_avoided = self
            .trivial_move_bytes_avoided
            .saturating_add(bytes_avoided);
    }

    pub(crate) fn record_outcome(&mut self, predicted_bytes: u64, actual_bytes: u64) {
        let denom = predicted_bytes.max(1) as f64;
        let err = (actual_bytes as f64 - predicted_bytes as f64).abs() / denom;
        self.predicted_actual_abs_pct_error_sum += err;
        self.predicted_actual_samples += 1;
    }

    pub(crate) fn snapshot(&self) -> GovernorStats {
        let mut time_in_mode = self.time_in_mode;
        time_in_mode[self.mode.idx()] += self.last_transition.elapsed();
        let avg_batch_size = if self.batch_sizes_chosen.is_empty() {
            0.0
        } else {
            self.batch_sizes_chosen.iter().sum::<usize>() as f64
                / self.batch_sizes_chosen.len() as f64
        };
        let mean_abs_pct_error = if self.predicted_actual_samples == 0 {
            0.0
        } else {
            self.predicted_actual_abs_pct_error_sum / self.predicted_actual_samples as f64
        };
        GovernorStats {
            mode: self.mode,
            mode_switches: self.mode_switches,
            time_balanced_ms: time_in_mode[GovernorMode::Balanced.idx()].as_millis() as u64,
            time_read_protect_ms: time_in_mode[GovernorMode::ReadProtect.idx()].as_millis() as u64,
            time_write_emergency_ms: time_in_mode[GovernorMode::WriteEmergency.idx()].as_millis()
                as u64,
            avg_batch_size,
            trivial_moves_performed: self.trivial_moves_performed,
            trivial_move_bytes_avoided: self.trivial_move_bytes_avoided,
            predicted_actual_samples: self.predicted_actual_samples,
            predicted_actual_mean_abs_pct_error: mean_abs_pct_error,
        }
    }
}

/// Point-in-time facts about the governor, for `KibanStats::governor`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GovernorStats {
    pub mode: GovernorMode,
    pub mode_switches: u64,
    pub time_balanced_ms: u64,
    pub time_read_protect_ms: u64,
    pub time_write_emergency_ms: u64,
    pub avg_batch_size: f64,
    pub trivial_moves_performed: u64,
    pub trivial_move_bytes_avoided: u64,
    pub predicted_actual_samples: u64,
    pub predicted_actual_mean_abs_pct_error: f64,
}

// ---- candidate generation / estimation / scoring ---------------------

/// Plain, owned mirror of `db::TableEntry` — deliberately dumber than
/// the real thing (no open `SstTable`, no `Arc`) so this whole section
/// can be built and tested without a live engine.
#[derive(Debug, Clone)]
pub(crate) struct TableInfo {
    pub number: u64,
    pub level: u32,
    pub size: u64,
    pub first_key: Vec<u8>,
    pub last_key: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateKind {
    L0,
    LevelBatch { level: u32, batch_len: usize },
    TrivialMove { level: u32 },
}

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub kind: CandidateKind,
    /// Table numbers this candidate touches. For `L0`: every L0 table
    /// plus overlapping L1 tables. For `LevelBatch`: the contiguous
    /// key-ordered run plus overlapping L+1 tables. For `TrivialMove`:
    /// exactly one number.
    pub input_numbers: Vec<u64>,
    pub output_level: u32,
}

/// What a candidate costs and buys, using only information already
/// available at PLAN time (table sizes and ranges) — never a fake-
/// precision estimate of post-merge dedup savings, which BUILD alone
/// can know. `estimated_rewrite_bytes` is a deliberate upper bound
/// (input + overlapping-destination bytes, nothing dropped), not a
/// prediction of the exact output size.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CandidateEstimate {
    /// Kept even though nothing currently reads it back out (beyond
    /// computing `estimated_rewrite_bytes`/`debt_relief_bytes` below):
    /// it's the honest, inspectable breakdown behind those derived
    /// numbers, useful for debugging a scoring surprise.
    #[allow(dead_code)]
    pub input_bytes: u64,
    #[allow(dead_code)]
    pub overlap_bytes: u64,
    pub estimated_rewrite_bytes: u64,
    /// Bytes this candidate removes from the pressured source (L0 or
    /// the over-budget level) — the thing actually relieving debt.
    pub debt_relief_bytes: u64,
    pub l0_files_removed: usize,
    /// File-number distance of the oldest input from the next number to
    /// be allocated — starvation proxy, same idea as `score_level_candidate`.
    pub age: u64,
}

fn overlapping<'a>(
    tables: &'a [TableInfo],
    level: u32,
    lo: &[u8],
    hi: &[u8],
) -> Vec<&'a TableInfo> {
    tables
        .iter()
        .filter(|t| t.level == level && t.first_key.as_slice() <= hi && t.last_key.as_slice() >= lo)
        .collect()
}

/// Enumerates every viable job for the current table topology. Bounded,
/// not combinatorial: at most one L0 candidate, up to `MAX_ADAPTIVE_BATCH`
/// batch-length candidates per over-budget level, and at most one
/// trivial-move candidate per over-budget level (the largest zero-
/// overlap table there — see module docs on why only the largest, and
/// why gated on the same "level is over budget" condition as a real
/// batch: an unconditionally-eligible trivial move would have the
/// worker moving files around on a perfectly healthy database forever).
pub(crate) fn generate_candidates(
    tables: &[TableInfo],
    l0_compaction_trigger: usize,
    over_budget_levels: &[(u32, u64)],
    next_file_number: u64,
) -> Vec<Candidate> {
    let mut out = Vec::new();

    let l0_count = tables.iter().filter(|t| t.level == 0).count();
    if l0_count >= l0_compaction_trigger && l0_count > 0 {
        let mut numbers: Vec<u64> = tables
            .iter()
            .filter(|t| t.level == 0)
            .map(|t| t.number)
            .collect();
        let lo = tables
            .iter()
            .filter(|t| t.level == 0)
            .map(|t| t.first_key.clone())
            .min()
            .expect("l0 nonempty");
        let hi = tables
            .iter()
            .filter(|t| t.level == 0)
            .map(|t| t.last_key.clone())
            .max()
            .expect("l0 nonempty");
        numbers.extend(overlapping(tables, 1, &lo, &hi).iter().map(|t| t.number));
        out.push(Candidate {
            kind: CandidateKind::L0,
            input_numbers: numbers,
            output_level: 1,
        });
    }

    for &(level, _budget) in over_budget_levels {
        let mut by_key: Vec<&TableInfo> = tables.iter().filter(|t| t.level == level).collect();
        if by_key.is_empty() {
            continue;
        }
        by_key.sort_by(|a, b| a.first_key.cmp(&b.first_key));
        let seed_pos = by_key
            .iter()
            .enumerate()
            .min_by_key(|(_, t)| t.number)
            .map(|(pos, _)| pos)
            .expect("by_key nonempty");

        for batch_len in 1..=MAX_ADAPTIVE_BATCH.min(by_key.len() - seed_pos) {
            let batch = &by_key[seed_pos..seed_pos + batch_len];
            let lo = batch
                .iter()
                .map(|t| t.first_key.clone())
                .min()
                .expect("batch nonempty");
            let hi = batch
                .iter()
                .map(|t| t.last_key.clone())
                .max()
                .expect("batch nonempty");
            let mut numbers: Vec<u64> = batch.iter().map(|t| t.number).collect();
            numbers.extend(
                overlapping(tables, level + 1, &lo, &hi)
                    .iter()
                    .map(|t| t.number),
            );
            out.push(Candidate {
                kind: CandidateKind::LevelBatch { level, batch_len },
                input_numbers: numbers,
                output_level: level + 1,
            });
        }

        // Trivial-move candidate: the largest table at this level with
        // zero overlap against level+1, ties broken by lowest number
        // for determinism. `min_by_key`/`max_by_key` scans, cheap
        // (bounded by this level's table count).
        let best_free = by_key
            .iter()
            .filter(|t| overlapping(tables, level + 1, &t.first_key, &t.last_key).is_empty())
            .max_by_key(|t| (t.size, std::cmp::Reverse(t.number)));
        if let Some(t) = best_free {
            out.push(Candidate {
                kind: CandidateKind::TrivialMove { level },
                input_numbers: vec![t.number],
                output_level: level + 1,
            });
        }
        let _ = next_file_number; // age is computed per-candidate in estimate_candidate
    }

    out
}

pub(crate) fn estimate_candidate(
    tables: &[TableInfo],
    candidate: &Candidate,
    next_file_number: u64,
) -> CandidateEstimate {
    let by_number = |n: u64| tables.iter().find(|t| t.number == n);
    let source_level = match candidate.kind {
        CandidateKind::L0 => 0,
        CandidateKind::LevelBatch { level, .. } => level,
        CandidateKind::TrivialMove { level } => level,
    };
    let input_bytes: u64 = candidate
        .input_numbers
        .iter()
        .filter_map(|&n| by_number(n))
        .filter(|t| t.level == source_level)
        .map(|t| t.size)
        .sum();
    let overlap_bytes: u64 = candidate
        .input_numbers
        .iter()
        .filter_map(|&n| by_number(n))
        .filter(|t| t.level != source_level)
        .map(|t| t.size)
        .sum();
    let l0_files_removed = match candidate.kind {
        CandidateKind::L0 => candidate
            .input_numbers
            .iter()
            .filter_map(|&n| by_number(n))
            .filter(|t| t.level == 0)
            .count(),
        _ => 0,
    };
    let oldest_number = candidate
        .input_numbers
        .iter()
        .filter_map(|&n| by_number(n))
        .filter(|t| t.level == source_level)
        .map(|t| t.number)
        .min()
        .unwrap_or(next_file_number);
    let age = next_file_number.saturating_sub(oldest_number);

    let estimated_rewrite_bytes = match candidate.kind {
        // A trivial move performs no I/O beyond reopening the same
        // file's footer/index — not a rewrite at all. Charged at 0 so
        // scoring reflects reality, not a merge cost it will never pay.
        CandidateKind::TrivialMove { .. } => 0,
        _ => input_bytes + overlap_bytes,
    };
    CandidateEstimate {
        input_bytes,
        overlap_bytes,
        estimated_rewrite_bytes,
        debt_relief_bytes: input_bytes,
        l0_files_removed,
        age,
    }
}

/// Mode-dependent objective (section 5). Trivial moves are handled as a
/// deliberate special case rather than folded into the general formula:
/// they cost (approximately) nothing, so in every mode except
/// `WriteEmergency` they should simply always win against any real
/// merge competing for the same level's debt relief. In
/// `WriteEmergency`, restoring L0 headroom specifically is what
/// prevents a write stall — a free level-3-to-4 move that does nothing
/// for L0 should not preempt an L0 drain.
fn score(
    mode: GovernorMode,
    kind: CandidateKind,
    est: &CandidateEstimate,
    l0_write_stall_trigger: usize,
) -> f64 {
    if matches!(kind, CandidateKind::TrivialMove { .. }) {
        return match mode {
            GovernorMode::WriteEmergency => 0.5,
            // Free, but does nothing for the foreground read path (it
            // never touches L0) — stays well below an L0-relieving
            // candidate's score band (1_000.0+ below) so READ_PROTECT
            // never prefers a deep-level freebie over actually shrinking
            // the probe chain, while still comfortably beating a real,
            // costly merge competing for the same non-L0 debt.
            GovernorMode::ReadProtect => 50.0,
            GovernorMode::Balanced => 1_000.0,
        };
    }
    let rewrite = est.estimated_rewrite_bytes.max(1) as f64;
    match mode {
        // Strongly penalize unnecessary rewrite bytes: score is debt
        // relieved per byte physically rewritten (amortization
        // efficiency), the same quantity `compaction_batch_size`
        // tuning was chasing manually — now chosen per-decision instead
        // of fixed globally. A small age term prevents perpetual
        // starvation of a level that's only slightly over budget but
        // has been waiting a long time, mirroring `score_level_candidate`.
        GovernorMode::Balanced => {
            est.debt_relief_bytes as f64 / rewrite + (est.age as f64).ln_1p() * 0.01
        }
        // What directly shortens the foreground probe chain is removing
        // L0 files — nothing else does, since level>=1 lookups already
        // cost at most one probe regardless of how over-budget that
        // level is. So any L0-relieving candidate outscores every
        // non-L0 candidate categorically (a large fixed base, not an
        // additive term that a sufficiently tiny non-L0 rewrite could
        // still out-divide); among L0 candidates a cheaper one is
        // (mildly) preferred. Non-L0 candidates still score — never
        // zero, so they aren't starved forever once L0 is healthy again
        // and nothing better is competing — just heavily discounted.
        GovernorMode::ReadProtect => {
            if est.l0_files_removed > 0 {
                1_000.0 + est.l0_files_removed as f64 - rewrite * 1e-6
            } else {
                est.debt_relief_bytes as f64 / rewrite / 100.0
            }
        }
        // Survival: maximize L0 relief achieved, amplification mostly
        // ignored (a small sqrt-scaled cost term only breaks ties
        // between equally-relieving candidates, never dominates).
        GovernorMode::WriteEmergency => {
            let l0_bias = if est.l0_files_removed > 0 { 10.0 } else { 1.0 };
            let stall_headroom = l0_write_stall_trigger.max(1) as f64;
            (est.debt_relief_bytes as f64 / stall_headroom) * l0_bias - rewrite.sqrt() * 0.001
        }
    }
}

/// Picks the highest-scoring candidate for the current mode, or `None`
/// if there is nothing to do.
pub(crate) fn pick_best(
    mode: GovernorMode,
    candidates: &[(Candidate, CandidateEstimate)],
    l0_write_stall_trigger: usize,
) -> Option<&(Candidate, CandidateEstimate)> {
    candidates
        .iter()
        .enumerate()
        .map(|(i, (c, e))| (i, score(mode, c.kind, e, l0_write_stall_trigger)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| &candidates[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(number: u64, level: u32, size: u64, lo: &[u8], hi: &[u8]) -> TableInfo {
        TableInfo {
            number,
            level,
            size,
            first_key: lo.to_vec(),
            last_key: hi.to_vec(),
        }
    }

    fn sample(
        l0_count: usize,
        stall_trigger: usize,
        debt_ratio: f64,
        tables_probed: f64,
    ) -> PressureSample {
        PressureSample {
            l0_count,
            l0_write_stall_trigger: stall_trigger,
            max_level_debt_ratio: debt_ratio,
            tables_probed_per_get: tables_probed,
        }
    }

    // ---- FSM transition tests (section 10) ----

    #[test]
    fn balanced_to_read_protect_needs_a_sustained_streak() {
        let mut g = GovernorState::new();
        assert_eq!(g.mode(), GovernorMode::Balanced);
        // One noisy high sample: not enough by itself.
        assert_eq!(g.observe(sample(1, 100, 0.0, 5.0)), GovernorMode::Balanced);
        for _ in 0..10 {
            g.observe(sample(1, 100, 0.0, 5.0));
        }
        assert_eq!(g.mode(), GovernorMode::ReadProtect);
    }

    #[test]
    fn read_protect_to_balanced_needs_sustained_low_pressure() {
        let mut g = GovernorState::new();
        for _ in 0..10 {
            g.observe(sample(1, 100, 0.0, 5.0));
        }
        assert_eq!(g.mode(), GovernorMode::ReadProtect);
        // One good sample shouldn't flip it back immediately.
        g.observe(sample(1, 100, 0.0, 0.0));
        assert_eq!(g.mode(), GovernorMode::ReadProtect);
        for _ in 0..10 {
            g.observe(sample(1, 100, 0.0, 0.0));
        }
        assert_eq!(g.mode(), GovernorMode::Balanced);
    }

    #[test]
    fn balanced_to_write_emergency_is_immediate_not_smoothed() {
        let mut g = GovernorState::new();
        // A single sample right at the danger threshold must trip
        // emergency mode on the spot — no waiting for a streak, unlike
        // read pressure.
        assert_eq!(
            g.observe(sample(80, 100, 0.0, 0.0)),
            GovernorMode::WriteEmergency
        );
    }

    #[test]
    fn read_protect_to_write_emergency_is_immediate() {
        let mut g = GovernorState::new();
        for _ in 0..10 {
            g.observe(sample(1, 100, 0.0, 5.0));
        }
        assert_eq!(g.mode(), GovernorMode::ReadProtect);
        assert_eq!(
            g.observe(sample(80, 100, 0.0, 5.0)),
            GovernorMode::WriteEmergency
        );
    }

    #[test]
    fn write_emergency_exits_only_after_recovery_hysteresis() {
        let mut g = GovernorState::new();
        g.observe(sample(80, 100, 0.0, 0.0));
        assert_eq!(g.mode(), GovernorMode::WriteEmergency);
        // Recovered readings, but fewer than the required streak.
        g.observe(sample(1, 100, 0.0, 0.0));
        g.observe(sample(1, 100, 0.0, 0.0));
        assert_eq!(g.mode(), GovernorMode::WriteEmergency);
        // One more completes the streak.
        g.observe(sample(1, 100, 0.0, 0.0));
        assert_eq!(g.mode(), GovernorMode::Balanced);
    }

    #[test]
    fn write_emergency_recovery_streak_resets_on_a_bad_sample() {
        let mut g = GovernorState::new();
        g.observe(sample(80, 100, 0.0, 0.0));
        g.observe(sample(1, 100, 0.0, 0.0));
        g.observe(sample(1, 100, 0.0, 0.0));
        // Pressure spikes again right before the streak would complete.
        g.observe(sample(80, 100, 0.0, 0.0));
        assert_eq!(g.mode(), GovernorMode::WriteEmergency);
        g.observe(sample(1, 100, 0.0, 0.0));
        g.observe(sample(1, 100, 0.0, 0.0));
        assert_eq!(
            g.mode(),
            GovernorMode::WriteEmergency,
            "streak should have reset, not carried over"
        );
        g.observe(sample(1, 100, 0.0, 0.0));
        assert_eq!(g.mode(), GovernorMode::Balanced);
    }

    #[test]
    fn no_flapping_right_at_the_read_pressure_boundary() {
        // A signal that oscillates exactly at the enter threshold
        // should never actually flip mode back and forth — the EWMA
        // smooths single-sample noise and the streak counters add a
        // further debounce.
        let mut g = GovernorState::new();
        let mut switches_seen = 0u64;
        let mut last = g.mode();
        for i in 0..40 {
            let v = if i % 2 == 0 { 1.6 } else { 1.4 };
            let m = g.observe(sample(1, 100, 0.0, v));
            if m != last {
                switches_seen += 1;
            }
            last = m;
        }
        assert!(
            switches_seen <= 1,
            "expected at most one settle transition oscillating at the boundary, saw {switches_seen}"
        );
    }

    #[test]
    fn emergency_mode_does_not_permanently_suppress_deeper_levels() {
        // WRITE_EMERGENCY still scores level candidates (just biased);
        // a deep-level candidate with no L0 relief still gets a
        // nonzero, comparable score rather than being excluded outright.
        let tables = vec![t(1, 2, 1000, b"a", b"m"), t(2, 3, 100, b"z", b"zz")];
        let est = estimate_candidate(
            &tables,
            &Candidate {
                kind: CandidateKind::LevelBatch {
                    level: 2,
                    batch_len: 1,
                },
                input_numbers: vec![1],
                output_level: 3,
            },
            10,
        );
        let s = score(
            GovernorMode::WriteEmergency,
            CandidateKind::LevelBatch {
                level: 2,
                batch_len: 1,
            },
            &est,
            100,
        );
        assert!(
            s.is_finite() && s != 0.0,
            "deep-level candidate must still score, not be excluded"
        );
    }

    // ---- candidate generation / batching / trivial-move tests ----

    #[test]
    fn adaptive_batch_picker_prefers_the_cheaper_useful_batch() {
        // [A] rewrite 40 (input 40, no overlap)
        // [A,B] rewrite 90 (input 90)
        // [A,B,C] rewrite 150 (input 150)
        // [A,B,C,D] rewrite 400 (a huge D dominates the union range,
        // pulling in a large L+1 overlap) — the task's own example
        // shape: growth suddenly gets much worse.
        let tables = vec![
            t(1, 1, 40, b"a", b"b"),
            t(2, 1, 50, b"c", b"d"),
            t(3, 1, 60, b"e", b"f"),
            t(4, 1, 220, b"g", b"z"), // wide, drags in overlap below
            t(5, 2, 180, b"g", b"z"), // only overlaps D
        ];
        let candidates = generate_candidates(&tables, 100, &[(1, 100)], 10);
        let batches: Vec<_> = candidates
            .iter()
            .filter(|c| matches!(c.kind, CandidateKind::LevelBatch { .. }))
            .collect();
        assert_eq!(batches.len(), 4, "expected all four prefix lengths");

        let scored: Vec<(usize, f64)> = batches
            .iter()
            .map(|c| {
                let batch_len = match c.kind {
                    CandidateKind::LevelBatch { batch_len, .. } => batch_len,
                    _ => unreachable!(),
                };
                let est = estimate_candidate(&tables, c, 10);
                (batch_len, score(GovernorMode::Balanced, c.kind, &est, 1000))
            })
            .collect();
        let best = scored.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert_eq!(
            best.0, 3,
            "expected [A,B,C] to win, got batch_len={}",
            best.0
        );
    }

    #[test]
    fn trivial_move_chosen_over_rewrite_when_valid() {
        // Table 1 at level 1 has zero overlap with level 2 — a free
        // trivial move. Table 2 at level 1 overlaps level 2's table 3,
        // forcing a real rewrite. Both are over-budget-level candidates
        // competing in BALANCED mode; the trivial move must win.
        let tables = vec![
            t(1, 1, 500, b"a", b"b"), // no L2 overlap: free move
            t(2, 1, 500, b"y", b"z"),
            t(3, 2, 500, b"y", b"z"), // overlaps table 2 only
        ];
        let candidates = generate_candidates(&tables, 100, &[(1, 100)], 10);
        let trivial = candidates
            .iter()
            .find(|c| matches!(c.kind, CandidateKind::TrivialMove { .. }))
            .expect("a trivial move candidate must be generated for table 1");
        assert_eq!(trivial.input_numbers, vec![1]);

        let scored: Vec<(&Candidate, f64)> = candidates
            .iter()
            .map(|c| {
                let est = estimate_candidate(&tables, c, 10);
                (c, score(GovernorMode::Balanced, c.kind, &est, 1000))
            })
            .collect();
        let best = scored.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(
            matches!(best.0.kind, CandidateKind::TrivialMove { .. }),
            "expected the trivial move to win"
        );
    }

    #[test]
    fn trivial_move_not_generated_when_every_table_overlaps() {
        let tables = vec![t(1, 1, 500, b"a", b"m"), t(2, 2, 500, b"a", b"m")];
        let candidates = generate_candidates(&tables, 100, &[(1, 100)], 10);
        assert!(
            !candidates
                .iter()
                .any(|c| matches!(c.kind, CandidateKind::TrivialMove { .. })),
            "no zero-overlap table exists; no trivial move should be offered"
        );
    }

    #[test]
    fn read_protect_prefers_l0_relief_even_if_cheaper_batch_available() {
        let tables = vec![
            t(1, 0, 50, b"a", b"z"),
            t(2, 0, 50, b"a", b"z"),
            t(3, 0, 50, b"a", b"z"),
            t(10, 1, 5, b"y", b"z"), // tiny, over budget, cheap to compact but irrelevant to L0
        ];
        let candidates = generate_candidates(&tables, 3, &[(1, 1)], 10);
        let scored: Vec<(&Candidate, f64)> = candidates
            .iter()
            .map(|c| {
                let est = estimate_candidate(&tables, c, 10);
                (c, score(GovernorMode::ReadProtect, c.kind, &est, 1000))
            })
            .collect();
        let best = scored.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(
            matches!(best.0.kind, CandidateKind::L0),
            "READ_PROTECT should favor L0 relief"
        );
    }
}
