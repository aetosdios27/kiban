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
//!
//! 11.17-round-3.1: maintenance *pacing* (how aggressively the
//! background worker paces itself between jobs) is deliberately
//! decoupled from the mode FSM above. Modes still decide *what* to
//! compact and gate correctness-relevant behavior; a separate
//! continuous `pressure` value (`GovernorState::pressure`, `[0.0,
//! 1.0]`) decides *how hard* to pace, via `pacing_delay`. This exists
//! because the mode FSM's own hysteresis (needed so BALANCED/
//! READ_PROTECT don't flap) means a mode can stay "stale" for a few
//! PLAN cycles after the pressure that justified it has actually
//! subsided — harmless for candidate selection, but a real problem for
//! pacing: a phase-changing benchmark caught `WRITE_EMERGENCY`'s flat,
//! mode-keyed pacing multiplier producing a multi-millisecond PUT p999
//! spike exactly in that stale window. `pressure` reacts every PLAN
//! call, not every mode transition, so pacing decays as smoothly as
//! the underlying signals do instead of jumping the instant (delayed)
//! mode flips.

pub(crate) use pressure::pacing_delay;

mod pressure {
    use std::time::Duration;

    /// Continuous map from blended pressure to a maintenance pacing
    /// delay. `READ_PROTECT`'s ceiling is higher (more cautious at low
    /// pressure — foreground reads are already under sustained load);
    /// every mode shares the same floor, because a real emergency must
    /// be reachable regardless of which mode the (separately, more
    /// slowly) hysteresis-gated FSM currently reports. All three
    /// duration constants are unchanged from the discrete, mode-keyed
    /// version this replaces (20us BALANCED, 40us READ_PROTECT, 5us
    /// WRITE_EMERGENCY post-fix) — reused as the endpoints of a
    /// continuous curve instead of three fixed, independently-selected
    /// values.
    pub(crate) fn pacing_delay(mode: super::GovernorMode, pressure: f64) -> Duration {
        const MAX_DELAY: Duration = Duration::from_micros(20);
        const READ_PROTECT_MAX_DELAY: Duration = Duration::from_micros(40);
        const MIN_DELAY: Duration = Duration::from_micros(5);
        let ceiling = if mode == super::GovernorMode::ReadProtect {
            READ_PROTECT_MAX_DELAY
        } else {
            MAX_DELAY
        };
        let p = pressure.clamp(0.0, 1.0);
        let ceiling_nanos = ceiling.as_nanos() as f64;
        let floor_nanos = MIN_DELAY.as_nanos() as f64;
        let nanos = ceiling_nanos - p * (ceiling_nanos - floor_nanos);
        Duration::from_nanos(nanos.max(floor_nanos) as u64)
    }
}

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

/// 11.17-round-3.2: the causal fix. `GovernorState::pressure` (the
/// scalar from round 3.1) is still exactly what it was — a blend used
/// only for maintenance *pacing*. But blending L0 danger and level debt
/// into one number is precisely why candidate *selection* kept
/// mis-firing: a badly over-budget deep level and a calm L0 produce the
/// same high scalar, so a scorer that only sees the scalar (or the
/// coarser `WriteEmergency` mode it drives) can't tell "clear L0 now"
/// apart from "pay down that level now" — and the old scoring
/// hard-coded a bias toward L0 regardless of which one was actually
/// true. `PressureVector` keeps the components separate all the way
/// into candidate scoring, so each candidate can be judged by how much
/// of the pressure that's ACTUALLY present it would relieve.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PressureVector {
    /// Instantaneous (unsmoothed) L0-vs-stall-trigger fraction,
    /// normalized so 1.0 is exactly the FSM's own WRITE_EMERGENCY entry
    /// point — same signal as the pacing emergency floor, on purpose:
    /// L0 is the only thing this engine's write-stall backpressure
    /// actually gates on, so "is L0 dangerous" must never wait on a
    /// smoothing window here either.
    pub l0: f64,
    /// EWMA of level-debt-ratio and backlog-breadth (deliberately
    /// smoothed, unlike `l0`: a deep level being over budget is real
    /// but not stall-imminent, so there's no requirement to react to a
    /// single noisy sample).
    pub deep_debt: f64,
    /// EWMA'd tables-probed-per-get, normalized against the same
    /// threshold that drives READ_PROTECT entry.
    pub read: f64,
}

impl PressureVector {
    pub fn dominant(&self) -> PressureSource {
        let peak = self.l0.max(self.deep_debt).max(self.read);
        if peak <= 0.05 {
            PressureSource::None
        } else if self.l0 >= peak {
            PressureSource::L0
        } else if self.deep_debt >= peak {
            PressureSource::DeepDebt
        } else {
            PressureSource::Read
        }
    }
}

/// Which component of a `PressureVector` is currently dominant —
/// attribution only, not itself a scoring input (`dominant()` is
/// derived from the vector, never the other way around).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PressureSource {
    #[default]
    None,
    L0,
    DeepDebt,
    Read,
}

/// Public mirror of `CandidateKind`, for observability
/// (`SchedulingDecision`) without widening `CandidateKind` itself past
/// `pub(crate)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChosenKind {
    L0,
    LevelBatch { level: u32, batch_len: usize },
    TrivialMove { level: u32 },
}

impl From<CandidateKind> for ChosenKind {
    fn from(kind: CandidateKind) -> Self {
        match kind {
            CandidateKind::L0 => ChosenKind::L0,
            CandidateKind::LevelBatch { level, batch_len } => {
                ChosenKind::LevelBatch { level, batch_len }
            }
            CandidateKind::TrivialMove { level } => ChosenKind::TrivialMove { level },
        }
    }
}

/// One scheduling decision, recorded every PLAN call (section 3): what
/// pressure looked like, how many candidates were on the table, and
/// which one (if any) won and why. A run of these, read in order, is
/// meant to be legible as a story — "deep debt high -> deep-level job
/// chosen -> next sample's deep debt lower" — without needing to
/// correlate against anything else.
#[derive(Debug, Clone, Copy)]
pub struct SchedulingDecision {
    pub pressure: PressureVector,
    pub dominant: PressureSource,
    /// 11.17-round-3.3: which of `pick_best_cause_aware`'s three tiers
    /// produced `chosen` — the direct, observable proof of whether the
    /// focus mechanism is actually preventing level-thrashing.
    /// `FocusContinued` appearing several times in a row for the same
    /// level (visible via `chosen`) is what "the fix is working" looks
    /// like in this trace.
    pub tier: SelectionTier,
    pub candidates_considered: usize,
    pub chosen: Option<ChosenKind>,
    pub predicted_rewrite_bytes: u64,
    pub l0_relief: f64,
    pub debt_relief: f64,
    pub read_relief: f64,
}

/// See `SchedulingDecision::tier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionTier {
    #[default]
    None,
    L0,
    FocusContinued,
    FocusReselected,
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
    /// Number of levels >= 1 currently over their byte budget — how
    /// *wide* the compaction backlog is, a different question from
    /// `max_level_debt_ratio`'s "how deep is the worst one." Feeds only
    /// the smoothed pressure signal, not the instant emergency floor
    /// (breadth alone was never part of the FSM's own emergency
    /// criteria, so it shouldn't gate the pacing floor either).
    pub backlog_levels: usize,
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

/// Weight of the read-amplification contribution in the blended
/// pressure signal that feeds `GovernorState::pressure` (structural
/// signals — L0 and level debt — get the rest). Read amplification is
/// corroborating evidence of a bad backlog, not the primary driver: a
/// weight well under half keeps it from dominating the signal that
/// controls pacing.
const PRESSURE_READAMP_WEIGHT: f64 = 0.3;
/// Number of simultaneously over-budget levels treated as "fully"
/// contributing backlog-breadth pressure. Three over-budget levels at
/// once is already an unusual, clearly-bad topology for this engine's
/// leveled layout; more doesn't need to push the signal harder.
const PRESSURE_BACKLOG_NORMALIZATION: f64 = 3.0;

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
    /// EWMA of the blended structural + read-amp pressure signal
    /// (11.17-round-3.1) — decays gradually across successive
    /// `observe` calls rather than stepping when the (separately
    /// hysteresis-gated) mode changes. See `pressure`.
    pressure_ewma: f64,
    /// `pressure_ewma` combined with the current call's *unsmoothed*
    /// emergency floor — what `pressure()` actually returns. Stored
    /// (rather than recomputed) because `observe`'s return value is the
    /// mode, not this.
    last_pressure: f64,
    /// EWMA of level-debt/backlog-breadth alone (11.17-round-3.2) —
    /// kept separate from `pressure_ewma` (which blends it with read
    /// amplification for the pacing scalar) because candidate scoring
    /// needs to know deep-debt pressure specifically, not folded into
    /// anything else. See `PressureVector`.
    debt_ewma: f64,
    /// The current pressure vector, updated every `observe` call —
    /// what candidate scoring actually reads. See `pressure_vector`.
    pressure_vector: PressureVector,
    /// Bounded trace of recent scheduling decisions (section 3),
    /// oldest dropped first. Exists purely for observability — nothing
    /// in the engine reads this back to make a decision.
    decisions: std::collections::VecDeque<SchedulingDecision>,
    /// 11.17-round-3.3: which deep level candidate selection is
    /// currently committed to, and for how many consecutive jobs — see
    /// `pick_best_cause_aware`. Unlike `decisions`, this genuinely IS
    /// read back to make the next decision.
    focus: FocusState,
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
            pressure_ewma: 0.0,
            last_pressure: 0.0,
            debt_ewma: 0.0,
            pressure_vector: PressureVector::default(),
            decisions: std::collections::VecDeque::new(),
            focus: FocusState::default(),
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

    /// Continuous maintenance-aggressiveness signal in `[0.0, 1.0]`,
    /// updated on every `observe`/`observe_engine` call — see the
    /// module-level 11.17-round-3.1 docs and `pacing_delay`. Independent
    /// of `mode`'s own hysteresis: this is what actually decays
    /// smoothly.
    pub(crate) fn pressure(&self) -> f64 {
        self.last_pressure
    }

    /// The current pressure vector (11.17-round-3.2) — what candidate
    /// scoring reads. See `PressureVector`.
    pub(crate) fn pressure_vector(&self) -> PressureVector {
        self.pressure_vector
    }

    /// 11.17-round-3.3: which deep level `pick_best_cause_aware` is
    /// currently committed to, if any — read by `crate::db` before
    /// calling it and written back with `set_focus` after.
    pub(crate) fn focus(&self) -> FocusState {
        self.focus
    }

    pub(crate) fn set_focus(&mut self, focus: FocusState) {
        self.focus = focus;
    }

    /// Appends one scheduling decision to the bounded trace (section
    /// 3), dropping the oldest if at capacity.
    pub(crate) fn record_decision(&mut self, decision: SchedulingDecision) {
        const MAX_DECISIONS_KEPT: usize = 4096;
        if self.decisions.len() >= MAX_DECISIONS_KEPT {
            self.decisions.pop_front();
        }
        self.decisions.push_back(decision);
    }

    /// Snapshot of the recorded scheduling-decision trace, oldest
    /// first — for a caller (a benchmark, say) that wants to show the
    /// actual pressure-source -> chosen-job -> resulting-pressure story
    /// for a window of interest.
    pub(crate) fn recent_decisions(&self) -> Vec<SchedulingDecision> {
        self.decisions.iter().copied().collect()
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

        // 11.17-round-3.1: continuous pressure, computed every call
        // regardless of mode transitions. `pressure_ewma` blends L0
        // fraction, level debt, backlog breadth, and read amplification
        // (all normalized so 1.0 lands roughly where the FSM's own
        // WRITE_EMERGENCY/READ_PROTECT entry criteria do) and decays
        // gradually across calls — this is what lets pacing taper off
        // smoothly after a busy stretch instead of stepping the moment
        // the (separately hysteresis-gated) mode itself flips back.
        //
        // `emergency_floor`, by contrast, is UNSMOOTHED and deliberately
        // narrower than the blend: L0 fraction ONLY, never level debt.
        // L0 count is what this engine's write-stall backpressure
        // actually gates on (`l0_write_stall_trigger`) — a real
        // approach to that ceiling must force maximum maintenance on
        // this exact call, no smoothing window (requirement 1). Level
        // debt has no such direct backpressure consequence: a deep
        // level can legitimately sit several times over its (often
        // small, at low levels) byte budget for a while under a
        // sustained new-data phase without threatening a stall the way
        // high L0 does. Two full six-phase runs caught exactly this
        // conflation when debt was still part of the floor: a
        // persistently over-budget deep level pinned pressure at 1.0
        // for tens of seconds even while L0 itself sat at 1-2 (far
        // below any real danger), because candidate selection
        // (unchanged, out of scope this round) prioritizes L0 relief
        // over deep-level relief in WRITE_EMERGENCY — so the debt
        // component could stay maxed far longer than L0 ever does,
        // producing a governor-only PUT p99 tail with no correctness
        // justification. Level debt still drives the smoothed blend
        // below (it should still push pacing more aggressive), it just
        // no longer gets to hold the INSTANT floor open on its own.
        let l0_component = (l0_fraction / WRITE_EMERGENCY_ENTER_L0_FRACTION).min(1.0);
        let debt_component =
            (sample.max_level_debt_ratio / WRITE_EMERGENCY_ENTER_DEBT_RATIO).min(1.0);
        let backlog_component =
            (sample.backlog_levels as f64 / PRESSURE_BACKLOG_NORMALIZATION).min(1.0);
        let readamp_component =
            (self.tables_probed_ewma / READ_PROTECT_ENTER_TABLES_PROBED).min(1.0);
        let structural = l0_component.max(debt_component).max(backlog_component);
        let instant = structural * (1.0 - PRESSURE_READAMP_WEIGHT)
            + readamp_component * PRESSURE_READAMP_WEIGHT;
        self.pressure_ewma = EWMA_ALPHA * instant + (1.0 - EWMA_ALPHA) * self.pressure_ewma;
        let emergency_floor = l0_component;
        self.last_pressure = self.pressure_ewma.max(emergency_floor).clamp(0.0, 1.0);

        // 11.17-round-3.2: the deep-debt component gets its OWN EWMA,
        // separate from the blended `pressure_ewma` above — candidate
        // scoring needs "how much deep-debt pressure exists right now"
        // as its own number, not folded together with read/backlog
        // signals the way the pacing scalar folds them. `l0` in the
        // vector stays unsmoothed (same value as `emergency_floor`)
        // for the same reason the pacing floor does: L0 danger must
        // never wait on a smoothing window.
        let debt_instant = debt_component.max(backlog_component);
        self.debt_ewma = EWMA_ALPHA * debt_instant + (1.0 - EWMA_ALPHA) * self.debt_ewma;
        self.pressure_vector = PressureVector {
            l0: l0_component,
            deep_debt: self.debt_ewma,
            read: readamp_component,
        };

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
        backlog_levels: usize,
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
            backlog_levels,
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
            pressure: self.last_pressure,
            pressure_vector: self.pressure_vector,
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
    /// Continuous pacing-aggressiveness signal at snapshot time (11.17-
    /// round-3.1) — see `GovernorState::pressure`. Sampled alongside
    /// `mode` so a caller (a benchmark, say) can plot whether actuation
    /// actually decays smoothly across a workload transition instead of
    /// stepping with the mode.
    pub pressure: f64,
    /// 11.17-round-3.2: the pressure vector candidate scoring actually
    /// reads, at snapshot time — see `GovernorState::pressure_vector`.
    /// Exposed here (rather than only via `governor_trace`) so a
    /// frequent poller (a benchmark's sampler thread, say) can plot the
    /// components over time cheaply.
    pub pressure_vector: PressureVector,
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
    /// Raw bytes this candidate removes from the pressured source (L0
    /// or the over-budget level) — the numerator behind `debt_relief`
    /// below, kept for inspectability now that scoring itself reads
    /// the normalized fraction instead.
    #[allow(dead_code)]
    pub debt_relief_bytes: u64,
    #[allow(dead_code)]
    pub l0_files_removed: usize,
    /// File-number distance of the oldest input from the next number to
    /// be allocated — starvation proxy, same idea as `score_level_candidate`.
    pub age: u64,
    /// 11.17-round-3.2: normalized (0.0 or 1.0) — this candidate either
    /// fully clears L0 (the `L0` kind always takes every current L0
    /// table) or has nothing to do with it. Paired with
    /// `PressureVector::l0` in `cause_aware_score`.
    pub l0_relief: f64,
    /// 11.17-round-3.2: fraction of the TARGET level's own over-budget
    /// excess (`level_bytes - budget`) this candidate would remove,
    /// capped at 1.0. Zero for the `L0` kind — an L0 merge's output
    /// lands AT the level it targets, so it grows that level rather
    /// than relieving it. Paired with `PressureVector::deep_debt`.
    pub debt_relief: f64,
    /// 11.17-round-3.2: normalized (0.0 or 1.0). Equal to `l0_relief` in
    /// this engine specifically: level >= 1 lookups already cost at
    /// most one probe regardless of how many tables live there (range
    /// disjointness), so only clearing L0 measurably shortens the
    /// foreground probe chain — a level>=1 merge has no such effect to
    /// claim credit for. Paired with `PressureVector::read`.
    pub read_relief: f64,
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

        // Trivial-move candidate: EVERY table at this level with zero
        // overlap against level+1 (up to MAX_ADAPTIVE_BATCH), not just
        // the largest one — batched into ONE job. 11.17-round-3.3
        // evidence (three independent full six-phase runs) found that
        // moving only one table per job, when a level often has several
        // simultaneously free-to-move tables, meant several separate
        // PLAN/BUILD/COMMIT cycles — several foreground-excluding
        // COMMITs in rapid succession — to do work that costs nothing
        // extra to combine into one. Each trivial move is independently
        // free (no I/O beyond a footer/index reopen), so there is no
        // batching-length tradeoff to explore the way real merges have
        // (more overlap, more rewrite cost) — take as many as exist, up
        // to the same cap real batches use, largest first for
        // determinism.
        let mut free: Vec<&TableInfo> = by_key
            .iter()
            .copied()
            .filter(|t| overlapping(tables, level + 1, &t.first_key, &t.last_key).is_empty())
            .collect();
        free.sort_by(|a, b| b.size.cmp(&a.size).then(a.number.cmp(&b.number)));
        free.truncate(MAX_ADAPTIVE_BATCH);
        if !free.is_empty() {
            out.push(Candidate {
                kind: CandidateKind::TrivialMove { level },
                input_numbers: free.iter().map(|t| t.number).collect(),
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
    over_budget_levels: &[(u32, u64)],
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

    // 11.17-round-3.2: cause-aware relief fields. `l0_relief`/
    // `read_relief` are simple booleans-as-floats (see their doc
    // comments on `CandidateEstimate`); `debt_relief` is a real
    // fraction of the target level's own over-budget excess, so a
    // candidate that only nibbles at a level far over budget scores
    // honestly lower than one that would actually clear it.
    let l0_relief = if matches!(candidate.kind, CandidateKind::L0) {
        1.0
    } else {
        0.0
    };
    let read_relief = l0_relief;
    let debt_relief = match candidate.kind {
        CandidateKind::L0 => 0.0,
        CandidateKind::LevelBatch { level, .. } | CandidateKind::TrivialMove { level } => {
            over_budget_levels
                .iter()
                .find(|(l, _)| *l == level)
                .map(|&(_, budget)| {
                    let level_bytes: u64 = tables
                        .iter()
                        .filter(|t| t.level == level)
                        .map(|t| t.size)
                        .sum();
                    let excess = level_bytes.saturating_sub(budget).max(1);
                    (input_bytes as f64 / excess as f64).min(1.0)
                })
                .unwrap_or(0.0)
        }
    };

    CandidateEstimate {
        input_bytes,
        overlap_bytes,
        estimated_rewrite_bytes,
        debt_relief_bytes: input_bytes,
        l0_files_removed,
        age,
        l0_relief,
        debt_relief,
        read_relief,
    }
}

/// Small bonus so a candidate that has been sitting neglected the
/// longest doesn't lose forever to ones with a marginally better
/// relief-per-cost ratio — same idea `score_level_candidate` used
/// before the governor existed, log-scaled so it nudges rather than
/// dominates.
const AGE_BONUS_WEIGHT: f64 = 0.01;

/// 11.17-round-3.2: cause-aware objective. Benefit is a dot product of
/// the CURRENT pressure vector against what this specific candidate
/// would actually relieve — a candidate only gets credit for the kind
/// of pressure that's really present, which is the entire fix: the old
/// mode-keyed `score` gave every L0-relieving candidate a fixed bonus
/// regardless of whether L0 pressure existed at all, so a badly
/// over-budget deep level (which the L0 bonus does nothing for) could
/// sit unaddressed while the scorer kept reaching for L0 work out of
/// habit. Cost is rewrite bytes, square-rooted rather than used
/// linearly: linear cost division makes ordering among same-order-of-
/// magnitude real candidates degenerate into "whichever touches fewer
/// bytes," even when a larger candidate provides meaningfully more
/// relief per byte — sqrt keeps that discrimination while still making
/// a near-zero-cost trivial move (rewrite floored at 1) dominate any
/// real merge by orders of magnitude, exactly as intended.
fn cause_aware_score(vector: PressureVector, est: &CandidateEstimate) -> f64 {
    let rewrite = est.estimated_rewrite_bytes.max(1) as f64;
    let benefit = vector.l0 * est.l0_relief
        + vector.deep_debt * est.debt_relief
        + vector.read * est.read_relief
        + (est.age as f64).ln_1p() * AGE_BONUS_WEIGHT;
    // Tried adding a fixed per-job byte overhead here (charging every
    // candidate a minimum cost so many-tiny-jobs stopped looking
    // categorically cheaper than fewer-larger-jobs) on the theory that
    // job COUNT, not just bytes, has a real fixed cost — a brief
    // foreground-excluding COMMIT per job. Real evidence (a full
    // six-phase re-run) rejected it: trivial-move and small-batch
    // selection share went UP, not down, and phase 3/5/6 PUT tail got
    // WORSE. Likely cause: at this engine's actual candidate byte
    // scale, a flat additive floor shifts small real candidates
    // relatively MORE than it shifts the already-near-zero trivial
    // moves, the opposite of the intended effect. Reverted rather than
    // kept on the strength of the reasoning alone — the round's own
    // rule against hand-tuning without evidence cuts both ways.
    benefit / rewrite.sqrt()
}

fn level_of(kind: CandidateKind) -> u32 {
    match kind {
        CandidateKind::L0 => 0,
        CandidateKind::LevelBatch { level, .. } => level,
        CandidateKind::TrivialMove { level } => level,
    }
}

/// Number of consecutive PLAN calls a focus level is allowed to keep
/// winning before selection is forced to re-evaluate from scratch —
/// same magnitude as `WRITE_EMERGENCY_EXIT_WINDOWS` elsewhere in this
/// file, reused rather than invented fresh. Exists only as an upper
/// bound so a level that's genuinely never going to resolve (should not
/// happen given real budgets, but this is a safety net, not the primary
/// mechanism) can't hold focus forever; in the overwhelmingly common
/// case focus clears naturally the moment the level drops under budget
/// (see `pick_best_cause_aware`'s doc comment).
const FOCUS_MAX_CONSECUTIVE_JOBS: u32 = 3;

/// Which level (if any) candidate selection is currently committed to,
/// and how many consecutive jobs have run against it. Threaded through
/// `pick_best_cause_aware` explicitly (not stored inside it) so the
/// function stays pure and testable; `crate::db` persists the returned
/// value back onto `GovernorState` between PLAN calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FocusState {
    pub current_focus: Option<u32>,
    pub streak: u32,
}

/// 11.17-round-3.3: the structural fix. A real decision trace (a
/// six-phase benchmark run) showed candidate selection re-litigating
/// "which of several over-budget levels is marginally best" on EVERY
/// PLAN call — L3, then L2, then L3, then L2-trivial-move, then L3,
/// then L4-trivial-move, then L3 again — because the previous design
/// had no memory: each call globally re-optimized `cause_aware_score`
/// from scratch. No reshaping of that score (three rounds tried:
/// mode-keyed bonuses, a pure pressure-vector dot product, a per-job
/// byte floor) could ever fix this, because target-switching is an
/// intrinsic property of "pick the global argmax fresh every time," not
/// a symptom the argmax's shape controls. The fix is a different
/// mechanism entirely: commit to one level, work it down across several
/// consecutive jobs, and only re-open the global question once that
/// level resolves (or a bounded streak expires as a safety net).
///
/// Three tiers, checked in order:
///
/// 1. L0 is UNCONDITIONAL, exactly like `FixedPriority`: if any L0
///    candidate exists (`generate_candidates` only emits one once
///    `l0_count >= l0_compaction_trigger`), it wins outright, no
///    scoring, no competition with deep-level work. This is the direct
///    fix for the round-3.2 max-L0 regression (5 -> 12): the previous
///    design let a high-scoring deep-level candidate occasionally
///    outbid L0 relief even while L0 had real candidates waiting,
///    which is exactly what let L0 grow past where `FixedPriority`
///    ever lets it. Reproducing `FixedPriority`'s own bound here,
///    structurally, is what actually gives the same guarantee back —
///    no vector-weight tuning was ever going to promise it.
/// 2. If a deep-level focus is already active (`focus.current_focus =
///    Some(level)`), stay on it: score only THAT level's candidates
///    and keep going, incrementing the streak. This naturally clears
///    itself the instant the level drops under budget — `candidates`
///    (generated only for `over_budget_levels`) simply stops
///    containing anything for that level, `pick_best_within_level`
///    returns `None`, and control falls through to tier 3 without any
///    extra bookkeeping. `FOCUS_MAX_CONSECUTIVE_JOBS` is the only
///    override, and only as a safety net.
/// 3. No active focus (or it just expired): re-score every deep-level
///    candidate globally via `cause_aware_score`, same as before, and
///    commit to whichever level wins as the new focus.
pub(crate) fn pick_best_cause_aware(
    vector: PressureVector,
    focus: FocusState,
    candidates: &[(Candidate, CandidateEstimate)],
) -> (
    Option<&(Candidate, CandidateEstimate)>,
    FocusState,
    SelectionTier,
) {
    // Tier 1: L0 is unconditional — no scoring, no focus bookkeeping.
    if let Some(idx) = candidates
        .iter()
        .position(|(c, _)| c.kind == CandidateKind::L0)
    {
        return (
            Some(&candidates[idx]),
            FocusState {
                current_focus: None,
                streak: 0,
            },
            SelectionTier::L0,
        );
    }

    // Tier 2: stay on the active focus level while it's still viable.
    if let Some(level) = focus.current_focus
        && focus.streak < FOCUS_MAX_CONSECUTIVE_JOBS
        && let Some(best) = pick_best_within_level(vector, level, candidates)
    {
        return (
            Some(best),
            FocusState {
                current_focus: Some(level),
                streak: focus.streak + 1,
            },
            SelectionTier::FocusContinued,
        );
    }

    // Tier 3: re-open the global question and commit to a new focus.
    let Some(new_level) = select_focus_level(vector, candidates) else {
        return (
            None,
            FocusState {
                current_focus: None,
                streak: 0,
            },
            SelectionTier::None,
        );
    };
    let best = pick_best_within_level(vector, new_level, candidates);
    (
        best,
        FocusState {
            current_focus: Some(new_level),
            streak: 1,
        },
        SelectionTier::FocusReselected,
    )
}

/// Tier 3 helper: which deep level scores best right now, globally,
/// across every over-budget level's candidates (L0 is never a
/// candidate here — tier 1 already handles it separately).
fn select_focus_level(
    vector: PressureVector,
    candidates: &[(Candidate, CandidateEstimate)],
) -> Option<u32> {
    candidates
        .iter()
        .filter(|(c, _)| c.kind != CandidateKind::L0)
        .map(|(c, e)| (level_of(c.kind), cause_aware_score(vector, e)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(level, _)| level)
}

/// Tier 2 helper: the best-scoring candidate restricted to one
/// specific level (adaptive batching still applies within it — this is
/// just the existing scoring narrowed to a subset of candidates, not a
/// different formula).
fn pick_best_within_level(
    vector: PressureVector,
    level: u32,
    candidates: &[(Candidate, CandidateEstimate)],
) -> Option<&(Candidate, CandidateEstimate)> {
    candidates
        .iter()
        .filter(|(c, _)| level_of(c.kind) == level)
        .max_by(|a, b| cause_aware_score(vector, &a.1).total_cmp(&cause_aware_score(vector, &b.1)))
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
            backlog_levels: 0,
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
        // Even with L0 pressure maxed (the old WRITE_EMERGENCY-style
        // condition), a deep-level candidate with real debt relief
        // still gets a nonzero, finite score from the deep_debt term —
        // it's just outranked, never excluded outright, by the L0
        // survival override when one actually applies.
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
            &[(2, 100)],
        );
        let vector = PressureVector {
            l0: 1.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let s = cause_aware_score(vector, &est);
        assert!(
            s.is_finite() && s != 0.0,
            "deep-level candidate must still score, not be excluded"
        );
    }

    // ---- candidate generation / batching / trivial-move tests ----

    #[test]
    fn adaptive_batch_picker_prefers_the_cheaper_useful_batch() {
        // Sized at realistic-SST scale (tens of KB), not toy byte
        // counts: `cause_aware_score`'s fixed per-job overhead
        // (JOB_OVERHEAD_BYTES) is calibrated against real rewrite
        // sizes, and swamps everything if the candidates themselves are
        // only tens of bytes.
        // [A] rewrite 40_000 (input 40_000, no overlap)
        // [A,B] rewrite 90_000 (input 90_000)
        // [A,B,C] rewrite 150_000 (input 150_000)
        // [A,B,C,D] rewrite 550_000 (a huge D dominates the union range,
        // pulling in a large L+1 overlap) — the task's own example
        // shape: growth suddenly gets much worse.
        let tables = vec![
            t(1, 1, 40_000, b"a", b"b"),
            t(2, 1, 50_000, b"c", b"d"),
            t(3, 1, 60_000, b"e", b"f"),
            t(4, 1, 220_000, b"g", b"z"), // wide, drags in overlap below
            t(5, 2, 180_000, b"g", b"z"), // only overlaps D
        ];
        let candidates = generate_candidates(&tables, 100, &[(1, 100_000)], 10);
        let batches: Vec<_> = candidates
            .iter()
            .filter(|c| matches!(c.kind, CandidateKind::LevelBatch { .. }))
            .collect();
        assert_eq!(batches.len(), 4, "expected all four prefix lengths");

        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let scored: Vec<(usize, f64)> = batches
            .iter()
            .map(|c| {
                let batch_len = match c.kind {
                    CandidateKind::LevelBatch { batch_len, .. } => batch_len,
                    _ => unreachable!(),
                };
                let est = estimate_candidate(&tables, c, 10, &[(1, 100_000)]);
                (batch_len, cause_aware_score(vector, &est))
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
        // competing for the same deep-debt pressure; the trivial move
        // must win on cost alone.
        // Realistic-SST scale, same reason as the batch-picker test
        // above: the fixed per-job overhead in `cause_aware_score` is
        // calibrated against real rewrite sizes.
        let tables = vec![
            t(1, 1, 500_000, b"a", b"b"), // no L2 overlap: free move
            t(2, 1, 500_000, b"y", b"z"),
            t(3, 2, 500_000, b"y", b"z"), // overlaps table 2 only
        ];
        let candidates = generate_candidates(&tables, 100, &[(1, 100_000)], 10);
        let trivial = candidates
            .iter()
            .find(|c| matches!(c.kind, CandidateKind::TrivialMove { .. }))
            .expect("a trivial move candidate must be generated for table 1");
        assert_eq!(trivial.input_numbers, vec![1]);

        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let scored: Vec<(&Candidate, f64)> = candidates
            .iter()
            .map(|c| {
                let est = estimate_candidate(&tables, c, 10, &[(1, 100_000)]);
                (c, cause_aware_score(vector, &est))
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
    fn read_pressure_with_calm_write_state_favors_the_read_relief_candidate() {
        let tables = vec![
            t(1, 0, 50, b"a", b"z"),
            t(2, 0, 50, b"a", b"z"),
            t(3, 0, 50, b"a", b"z"),
            t(10, 1, 5, b"y", b"z"), // tiny, over budget, cheap to compact but irrelevant to L0/reads
        ];
        let candidates = generate_candidates(&tables, 3, &[(1, 1)], 10);
        // read pressure high (what would have driven READ_PROTECT under
        // the old mode-based design); L0 and deep debt both calm.
        let vector = PressureVector {
            l0: 0.05,
            deep_debt: 0.05,
            read: 1.0,
        };
        let scored: Vec<(&Candidate, f64)> = candidates
            .iter()
            .map(|c| {
                let est = estimate_candidate(&tables, c, 10, &[(1, 1)]);
                (c, cause_aware_score(vector, &est))
            })
            .collect();
        let best = scored.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(
            matches!(best.0.kind, CandidateKind::L0),
            "high read pressure should favor the candidate with real read relief"
        );
    }

    // ---- continuous pressure / pacing tests (11.17-round-3.1) ----

    #[test]
    fn pacing_delay_ranges_from_ceiling_to_floor_monotonically() {
        let at_zero = pacing_delay(GovernorMode::Balanced, 0.0);
        let at_half = pacing_delay(GovernorMode::Balanced, 0.5);
        let at_one = pacing_delay(GovernorMode::Balanced, 1.0);
        assert_eq!(
            at_zero,
            std::time::Duration::from_micros(20),
            "pressure 0 must match the old flat BALANCED delay exactly"
        );
        assert_eq!(
            at_one,
            std::time::Duration::from_micros(5),
            "pressure 1 must reach the emergency floor"
        );
        assert!(
            at_half < at_zero && at_half > at_one,
            "delay must decrease monotonically with pressure, not step"
        );
    }

    #[test]
    fn read_protect_paces_more_than_balanced_at_the_same_low_pressure() {
        let balanced = pacing_delay(GovernorMode::Balanced, 0.1);
        let read_protect = pacing_delay(GovernorMode::ReadProtect, 0.1);
        assert!(
            read_protect > balanced,
            "READ_PROTECT should reduce foreground interference more than BALANCED at the same pressure"
        );
    }

    #[test]
    fn every_mode_reaches_the_same_emergency_floor_at_full_pressure() {
        // Whatever mode the (separately hysteresis-gated) FSM currently
        // reports, a real emergency must still be fully reachable —
        // pacing cannot be pinned to a cautious ceiling just because the
        // mode hasn't caught up yet.
        for mode in [
            GovernorMode::Balanced,
            GovernorMode::ReadProtect,
            GovernorMode::WriteEmergency,
        ] {
            assert_eq!(pacing_delay(mode, 1.0), std::time::Duration::from_micros(5));
        }
    }

    #[test]
    fn emergency_floor_overrides_smoothed_pressure_instantly() {
        let mut g = GovernorState::new();
        // Several calm samples: pressure_ewma should settle near 0.
        for _ in 0..10 {
            g.observe(sample(0, 100, 0.0, 0.0));
        }
        assert!(
            g.pressure() < 0.1,
            "pressure should be near zero after sustained calm, got {}",
            g.pressure()
        );
        // One sample right at the FSM's own emergency threshold: the
        // unsmoothed floor must force pressure to (near) 1.0 on THIS
        // call — no waiting for an EWMA to catch up.
        g.observe(sample(75, 100, 0.0, 0.0));
        assert!(
            g.pressure() > 0.95,
            "a single sample at the emergency threshold must force pressure to the floor immediately, got {}",
            g.pressure()
        );
    }

    #[test]
    fn pressure_decays_smoothly_not_instantly_after_a_spike_clears() {
        let mut g = GovernorState::new();
        // Drive pressure high via sustained real pressure (not just the
        // instantaneous floor, which itself doesn't persist) so there is
        // a real EWMA to decay.
        for _ in 0..10 {
            g.observe(sample(75, 100, 0.0, 0.0));
        }
        let peak = g.pressure();
        assert!(
            peak > 0.9,
            "expected sustained pressure near 1.0, got {peak}"
        );

        // Pressure now clears completely (L0 back to zero, no debt).
        let mut previous = peak;
        let mut saw_gradual_step = false;
        for _ in 0..15 {
            g.observe(sample(0, 100, 0.0, 0.0));
            let now = g.pressure();
            assert!(
                now <= previous + 1e-9,
                "pressure must not increase while the underlying signal stays clear"
            );
            if previous - now > 1e-9 && now > 0.05 {
                saw_gradual_step = true;
            }
            previous = now;
        }
        assert!(
            saw_gradual_step,
            "expected at least one intermediate, still-elevated sample during decay — pressure dropped in one step instead of decaying"
        );
        assert!(
            previous < 0.05,
            "pressure should eventually settle back near zero, ended at {previous}"
        );
    }

    #[test]
    fn backlog_breadth_and_read_amp_contribute_to_pressure_without_a_debt_ratio() {
        // No L0 pressure, no level debt — but several levels over
        // budget and elevated read amplification should still produce
        // some nonzero pressure (backlog breadth + read-amp
        // corroborating evidence), just not at the emergency floor.
        let mut g = GovernorState::new();
        for _ in 0..5 {
            g.observe(PressureSample {
                l0_count: 0,
                l0_write_stall_trigger: 100,
                max_level_debt_ratio: 0.0,
                tables_probed_per_get: 2.0,
                backlog_levels: 3,
            });
        }
        assert!(
            g.pressure() > 0.0,
            "backlog/read-amp signals should contribute nonzero pressure"
        );
        assert!(
            g.pressure() < 0.95,
            "this scenario is not a real emergency and must not hit the floor"
        );
    }

    #[test]
    fn sustained_level_debt_alone_does_not_pin_the_instant_floor() {
        // Reproduces the exact shape two full phase-changing runs
        // caught: L0 is calm (nowhere near the write-stall trigger) but
        // a deep level sits persistently, heavily over its own byte
        // budget — legitimate under a sustained new-data phase, and not
        // something that threatens a write stall the way high L0 does.
        // The instant floor must stay driven by L0 alone; debt still
        // pushes the smoothed pressure up, but gradually, not to the
        // floor on a single sample.
        let mut g = GovernorState::new();
        let high_debt = sample(0, 100, 10.0, 0.0); // 10x over budget, L0 empty
        let first = g.observe(high_debt);
        let _ = first;
        assert!(
            g.pressure() < 0.5,
            "one sample of pure level debt (no L0 pressure) must not spike pressure toward the floor, got {}",
            g.pressure()
        );
        // Even sustained, it must never reach the instant-floor
        // territory that L0 danger alone reaches in a single sample —
        // it can only push the smoothed component, capped by the same
        // structural blend as backlog/read-amp.
        for _ in 0..20 {
            g.observe(high_debt);
        }
        assert!(
            g.pressure() < 0.95,
            "sustained level debt alone (L0 still empty) must not reach the emergency floor, got {}",
            g.pressure()
        );
    }

    // ---- cause-aware candidate selection tests (11.17-round-3.2) ----
    //
    // These are the exact failure this round set out to fix: the old
    // mode-keyed scoring collapsed "L0 is calm but a deep level is
    // badly over budget" and "L0 itself is genuinely close to a write
    // stall" into the same generic WRITE_EMERGENCY treatment, so it
    // reached for L0-relieving work out of habit even when L0 wasn't
    // the actual problem. `PressureVector` keeps the two apart all the
    // way into scoring; these tests exercise both directions plus the
    // hard survival override and the age-based anti-starvation term.

    #[test]
    fn calm_l0_below_trigger_and_heavy_deep_debt_favors_the_deep_level_job() {
        // L0 is BELOW its compaction trigger (2 of 3 needed) — no L0
        // candidate exists at all — while a deep level is heavily over
        // budget. Tier 3 (global reselection) must pick it up.
        let tables = vec![
            t(1, 0, 100, b"a", b"b"),
            t(2, 0, 100, b"c", b"d"),
            t(10, 1, 900, b"m", b"n"), // one level 9x over its own budget
            t(20, 2, 10, b"m", b"n"),  // overlaps table 10: no free trivial move available
        ];
        let over_budget = [(1u32, 100u64)];
        let candidates = generate_candidates(&tables, 3, &over_budget, 20);
        assert!(
            !candidates.iter().any(|c| c.kind == CandidateKind::L0),
            "setup: L0 must be below its trigger, no L0 candidate"
        );
        assert!(
            candidates
                .iter()
                .any(|c| matches!(c.kind, CandidateKind::LevelBatch { level: 1, .. })),
            "setup: a deep-level candidate must exist"
        );

        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let estimated: Vec<_> = candidates
            .iter()
            .map(|c| (c.clone(), estimate_candidate(&tables, c, 20, &over_budget)))
            .collect();
        let (best, focus, tier) = pick_best_cause_aware(vector, FocusState::default(), &estimated);
        let best = best.expect("a candidate must be chosen");
        assert!(
            matches!(best.0.kind, CandidateKind::LevelBatch { level: 1, .. }),
            "expected the deep-level job to win when there is no L0 candidate, got {:?}",
            best.0.kind
        );
        assert_eq!(tier, SelectionTier::FocusReselected);
        assert_eq!(focus.current_focus, Some(1));
    }

    #[test]
    fn l0_is_unconditional_whenever_a_candidate_exists() {
        // 11.17-round-3.3: L0 is no longer a scored preference — it's a
        // tier-1 hard rule, exactly like FixedPriority. It must win even
        // when the pressure vector itself says deep debt is worse than
        // L0.
        let tables = vec![
            t(1, 0, 100, b"a", b"b"),
            t(2, 0, 100, b"c", b"d"),
            t(3, 0, 100, b"e", b"f"),
            t(10, 1, 900, b"m", b"n"),
        ];
        let over_budget = [(1u32, 100u64)];
        let candidates = generate_candidates(&tables, 3, &over_budget, 20);
        // deep_debt outweighs l0 in the vector itself — the OLD scoring
        // design would have let this outbid L0. Tier 1 must not care.
        let vector = PressureVector {
            l0: 0.05,
            deep_debt: 1.0,
            read: 0.0,
        };
        let estimated: Vec<_> = candidates
            .iter()
            .map(|c| (c.clone(), estimate_candidate(&tables, c, 20, &over_budget)))
            .collect();
        let (best, focus, tier) = pick_best_cause_aware(vector, FocusState::default(), &estimated);
        let best = best.expect("a candidate must be chosen");
        assert!(
            matches!(best.0.kind, CandidateKind::L0),
            "L0 must be unconditional whenever a candidate exists, got {:?}",
            best.0.kind
        );
        assert_eq!(tier, SelectionTier::L0);
        assert_eq!(
            focus.current_focus, None,
            "taking the L0 tier must clear any deep-level focus"
        );
    }

    #[test]
    fn no_l0_candidate_falls_through_to_focus_selection_normally() {
        let tables = vec![t(10, 1, 900, b"m", b"n")];
        let over_budget = [(1u32, 100u64)];
        let candidates = generate_candidates(&tables, 3, &over_budget, 20);
        assert!(!candidates.iter().any(|c| c.kind == CandidateKind::L0));
        let vector = PressureVector {
            l0: 1.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let estimated: Vec<_> = candidates
            .iter()
            .map(|c| (c.clone(), estimate_candidate(&tables, c, 20, &over_budget)))
            .collect();
        let (best, _, tier) = pick_best_cause_aware(vector, FocusState::default(), &estimated);
        assert!(
            best.is_some(),
            "no L0 candidate must fall through to focus selection, not return nothing"
        );
        assert_eq!(tier, SelectionTier::FocusReselected);
    }

    #[test]
    fn focus_persists_across_consecutive_calls_instead_of_switching_levels() {
        // Two over-budget levels, each with its own candidate, scored
        // so level 2 wins narrowly on the FIRST call. The old
        // (memoryless, global-reoptimize-every-call) design would
        // re-litigate this every time and could flip between them on
        // sample noise; the focus mechanism must stay on level 2 across
        // several calls without re-scoring level 3 at all.
        let tables = vec![
            t(1, 2, 500, b"a", b"b"),
            t(2, 3, 480, b"c", b"d"), // slightly cheaper/similar — a close competitor
        ];
        let over_budget = [(2u32, 100u64), (3u32, 100u64)];
        let candidates = generate_candidates(&tables, 100, &over_budget, 20);
        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let estimated: Vec<_> = candidates
            .iter()
            .map(|c| (c.clone(), estimate_candidate(&tables, c, 20, &over_budget)))
            .collect();

        let mut focus = FocusState::default();
        let (first, focus1, tier1) = pick_best_cause_aware(vector, focus, &estimated);
        let first_level = level_of(first.expect("a candidate must be chosen").0.kind);
        assert_eq!(tier1, SelectionTier::FocusReselected);
        focus = focus1;

        for _ in 0..(FOCUS_MAX_CONSECUTIVE_JOBS - 1) {
            let (chosen, next_focus, tier) = pick_best_cause_aware(vector, focus, &estimated);
            let level = level_of(chosen.expect("a candidate must be chosen").0.kind);
            assert_eq!(
                level, first_level,
                "focus must stay on the same level across consecutive calls"
            );
            assert_eq!(tier, SelectionTier::FocusContinued);
            focus = next_focus;
        }
    }

    #[test]
    fn focus_clears_and_reselects_once_the_focused_level_resolves() {
        let tables_busy = vec![t(1, 2, 500, b"a", b"b"), t(2, 3, 500, b"c", b"d")];
        let over_budget_busy = [(2u32, 100u64), (3u32, 100u64)];
        let candidates_busy = generate_candidates(&tables_busy, 100, &over_budget_busy, 20);
        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let estimated_busy: Vec<_> = candidates_busy
            .iter()
            .map(|c| {
                (
                    c.clone(),
                    estimate_candidate(&tables_busy, c, 20, &over_budget_busy),
                )
            })
            .collect();
        let (first, focus, tier) =
            pick_best_cause_aware(vector, FocusState::default(), &estimated_busy);
        let focused_level = level_of(first.expect("a candidate must be chosen").0.kind);
        assert_eq!(tier, SelectionTier::FocusReselected);

        // Now simulate that level having resolved: it's no longer over
        // budget, so it no longer generates any candidates — but the
        // OTHER level still is.
        let remaining_level = if focused_level == 2 { 3u32 } else { 2u32 };
        let over_budget_after = [(remaining_level, 100u64)];
        let candidates_after = generate_candidates(&tables_busy, 100, &over_budget_after, 20);
        let estimated_after: Vec<_> = candidates_after
            .iter()
            .map(|c| {
                (
                    c.clone(),
                    estimate_candidate(&tables_busy, c, 20, &over_budget_after),
                )
            })
            .collect();
        let (next, next_focus, tier) = pick_best_cause_aware(vector, focus, &estimated_after);
        let next_level = level_of(next.expect("a candidate must be chosen").0.kind);
        assert_eq!(
            next_level, remaining_level,
            "once the focused level resolves, selection must move to the level still over budget"
        );
        assert_eq!(tier, SelectionTier::FocusReselected);
        assert_eq!(next_focus.current_focus, Some(remaining_level));
    }

    #[test]
    fn focus_streak_cap_forces_a_reselection_even_if_still_over_budget() {
        // A single over-budget level, never resolving — the streak cap
        // is the only thing that would ever force a fresh tier-3 call
        // here. Confirms the cap actually fires (tier flips to
        // FocusReselected) rather than staying FocusContinued forever.
        let tables = vec![t(1, 2, 900, b"a", b"b")];
        let over_budget = [(2u32, 100u64)];
        let candidates = generate_candidates(&tables, 100, &over_budget, 20);
        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let estimated: Vec<_> = candidates
            .iter()
            .map(|c| (c.clone(), estimate_candidate(&tables, c, 20, &over_budget)))
            .collect();

        let mut focus = FocusState::default();
        let mut tiers = Vec::new();
        for _ in 0..(FOCUS_MAX_CONSECUTIVE_JOBS + 2) {
            let (_, next_focus, tier) = pick_best_cause_aware(vector, focus, &estimated);
            tiers.push(tier);
            focus = next_focus;
        }
        assert_eq!(tiers[0], SelectionTier::FocusReselected);
        for t in &tiers[1..FOCUS_MAX_CONSECUTIVE_JOBS as usize] {
            assert_eq!(*t, SelectionTier::FocusContinued);
        }
        assert_eq!(
            tiers[FOCUS_MAX_CONSECUTIVE_JOBS as usize],
            SelectionTier::FocusReselected,
            "the streak cap must force a reselection, not continue indefinitely"
        );
    }

    #[test]
    fn an_old_neglected_candidate_scores_higher_than_an_equally_relieving_young_one() {
        // Two single-table candidates with identical debt relief and
        // rewrite cost — the only difference is how long each has been
        // waiting. Without an age term, these would score identically
        // forever, and whichever loses a coin-flip-close tie could in
        // principle lose it every single time. The age bonus breaks
        // that tie in favor of the one that's been neglected longest.
        let est_young = CandidateEstimate {
            input_bytes: 100,
            overlap_bytes: 0,
            estimated_rewrite_bytes: 100,
            debt_relief_bytes: 100,
            l0_files_removed: 0,
            age: 1,
            l0_relief: 0.0,
            debt_relief: 0.5,
            read_relief: 0.0,
        };
        let mut est_old = est_young;
        est_old.age = 1_000_000;
        let vector = PressureVector {
            l0: 0.0,
            deep_debt: 1.0,
            read: 0.0,
        };
        let young_score = cause_aware_score(vector, &est_young);
        let old_score = cause_aware_score(vector, &est_old);
        assert!(
            old_score > young_score,
            "an old candidate must score higher than an otherwise-identical young one: young={young_score} old={old_score}"
        );
    }

    #[test]
    fn dominant_pressure_source_reflects_which_component_actually_changed() {
        // Section 3's attribution: a benchmark (or an operator) should
        // be able to watch `dominant()` move as the underlying vector
        // changes, without needing to know anything about which
        // candidate was chosen to cause that change.
        let before = PressureVector {
            l0: 0.1,
            deep_debt: 0.9,
            read: 0.2,
        };
        assert_eq!(before.dominant(), PressureSource::DeepDebt);

        // The deep-level job runs and actually pays down that debt;
        // read pressure (untouched by that job) is now the largest
        // remaining signal.
        let after = PressureVector {
            l0: 0.1,
            deep_debt: 0.15,
            read: 0.2,
        };
        assert_eq!(
            after.dominant(),
            PressureSource::Read,
            "once deep debt falls, attribution should move to whichever signal is now largest"
        );

        let calm = PressureVector {
            l0: 0.02,
            deep_debt: 0.03,
            read: 0.01,
        };
        assert_eq!(calm.dominant(), PressureSource::None);
    }
}
