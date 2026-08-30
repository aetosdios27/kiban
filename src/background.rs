//! One background compaction worker per [`crate::db::SharedKiban`].
//!
//! Compaction is split PLAN/BUILD/COMMIT (`docs/design/compaction.md`,
//! phase 11.4): PLAN and COMMIT run under the engine lock and are
//! cheap; BUILD — the merge and sstable construction — runs here, on
//! this worker thread, outside the lock, so foreground reads and
//! writes are never blocked behind it. There is exactly one worker per
//! engine: compaction jobs cannot race with each other, and MANIFEST
//! publication stays single-writer.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::db::Kiban;

/// A background maintenance failure. Sticky: nothing was waiting on the
/// call that failed, so the failure must be surfaced some other way
/// (11.4 background error handling) — this is that other way.
#[derive(Debug, Clone)]
pub struct MaintenanceError(pub(crate) String);

impl std::fmt::Display for MaintenanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "background compaction failed: {}", self.0)
    }
}

impl std::error::Error for MaintenanceError {}

/// Raw counters for the background maintenance worker (phase 11.7,
/// extended 11.8) — facts only, no health verdicts. `compaction_running`
/// mirrors the worker's own busy flag: true for the whole PLAN/BUILD/
/// COMMIT cycle it is currently working through — a pending flush job
/// or a compaction job, since 11.8 has the one worker do both (flush
/// always first — see `run_pending_maintenance`), not narrowed to just
/// BUILD or to compaction specifically.
#[derive(Debug, Clone, Copy)]
pub struct MaintenanceStats {
    pub compaction_running: bool,
    pub compactions_completed: u64,
    pub compactions_failed: u64,
    pub compaction_input_bytes: u64,
    pub compaction_output_bytes: u64,
    pub waiting_writers: usize,
    pub write_stalls: u64,
    pub flushes_completed: u64,
    pub flushes_failed: u64,
}

struct Signal {
    /// There may be more compaction to do; the worker should look.
    wake: bool,
    /// The worker should exit at its next opportunity.
    stop: bool,
    /// True while a compaction job is actually running (for
    /// `wait_settled` in tests).
    busy: bool,
    /// Sticky: once a job fails, the worker stops attempting more
    /// compaction until the process is reopened (never retry forever).
    error: Option<MaintenanceError>,
    /// Bumped whenever a stalled writer (11.5 backpressure) needs to
    /// recheck its condition: a compaction commit succeeds, background
    /// maintenance fails, or the worker shuts down. A writer captures
    /// this value while still holding the engine lock that any commit
    /// would also need, then waits for it to change — never for a bare
    /// notify — so a commit landing between "L0 is too high" and
    /// "start waiting" can never be missed.
    progress_epoch: u64,
    /// Count of `SharedKiban` callers currently parked in a backpressure
    /// wait (11.5) — real, condvar-observable state a test can block on
    /// instead of inferring blocking from elapsed time.
    waiting_writers: usize,
    /// Phase 11.7 raw counters, all cumulative and monotonic
    /// (`saturating_add`, never reset).
    compactions_completed: u64,
    /// Bumped only for an actual failed compaction job (a BUILD error,
    /// a COMMIT error, or a caught worker panic — see `record_error`).
    /// Deliberately NOT bumped by `Maintenance::error`'s lazy dead-
    /// thread detection: that discovers an already-dead worker, which
    /// is a different kind of failure than a job that ran and failed.
    compactions_failed: u64,
    compaction_input_bytes: u64,
    compaction_output_bytes: u64,
    /// One per `SharedKiban` mutation call that genuinely had to wait
    /// for L0 room, immutable-slot room, or both, at least once — one
    /// blocked call, one stall, however many wake/recheck cycles it
    /// took (mirrors the file-cache `waits` counting rule).
    write_stalls: u64,
    /// Phase 11.8 counters, same conventions as the compaction ones.
    flushes_completed: u64,
    flushes_failed: u64,
}

/// Owns the worker thread and the signal used to wake, stop, and query
/// it. One `Maintenance` per `SharedKiban` engine (shared by every
/// clone of that handle).
pub(crate) struct Maintenance {
    /// Count of live `SharedKiban` handles sharing this worker; the
    /// last one to drop stops and joins the thread (no Arc cycle: the
    /// worker holds strong refs to the engine and to this struct, but
    /// nothing holds a strong ref back to `SharedKiban` itself, so
    /// shutdown is driven by this explicit count, not by refcount
    /// polling).
    handles: AtomicUsize,
    state: Mutex<Signal>,
    condvar: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    test: TestHooks,
}

impl Maintenance {
    pub(crate) fn spawn(engine: Arc<Mutex<Kiban>>) -> Arc<Maintenance> {
        let m = Arc::new(Maintenance {
            handles: AtomicUsize::new(1),
            state: Mutex::new(Signal {
                wake: false,
                stop: false,
                busy: false,
                error: None,
                progress_epoch: 0,
                waiting_writers: 0,
                compactions_completed: 0,
                compactions_failed: 0,
                compaction_input_bytes: 0,
                compaction_output_bytes: 0,
                write_stalls: 0,
                flushes_completed: 0,
                flushes_failed: 0,
            }),
            condvar: Condvar::new(),
            thread: Mutex::new(None),
            #[cfg(test)]
            test: TestHooks::default(),
        });
        let worker = m.clone();
        let handle = std::thread::spawn(move || worker_loop(engine, worker));
        *m.thread.lock().unwrap() = Some(handle);
        m
    }

    /// Registers one more `SharedKiban` handle sharing this worker.
    pub(crate) fn add_handle(&self) {
        self.handles.fetch_add(1, Ordering::AcqRel);
    }

    /// Releases one handle. Stops and joins the worker iff this was the
    /// last live handle (race-free: `fetch_sub` returns the pre-
    /// decrement count, so exactly one caller ever observes 1).
    pub(crate) fn drop_handle(&self) {
        if self.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shutdown();
        }
    }

    fn shutdown(&self) {
        {
            let mut s = self.state.lock().unwrap();
            s.stop = true;
            s.wake = true;
            s.progress_epoch += 1;
        }
        self.condvar.notify_all();
        if let Some(h) = self.thread.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    /// Tells the worker there may be more compaction to do.
    pub(crate) fn wake(&self) {
        {
            self.state.lock().unwrap().wake = true;
        }
        self.condvar.notify_all();
    }

    /// The most recent background failure, if any. Also detects a
    /// worker thread that exited without being told to (a dead
    /// maintenance thread nobody asked to die is itself a failure).
    pub(crate) fn error(&self) -> Option<MaintenanceError> {
        let dead = match self.thread.lock().unwrap().as_ref() {
            Some(h) => h.is_finished(),
            None => false, // already joined via a clean shutdown
        };
        let mut s = self.state.lock().unwrap();
        let became_dead = dead && s.error.is_none() && !s.stop;
        if became_dead {
            s.error = Some(MaintenanceError(
                "maintenance worker thread exited unexpectedly".to_string(),
            ));
            s.progress_epoch += 1;
        }
        let result = s.error.clone();
        drop(s);
        if became_dead {
            // A stalled writer may be parked on exactly this transition;
            // nothing else would ever wake it (11.5).
            self.condvar.notify_all();
        }
        result
    }

    /// Current progress epoch (11.5 backpressure). Must be read while
    /// still holding the engine lock, immediately after observing L0
    /// too high — see `SharedKiban::wait_for_write_room` for why that
    /// ordering is what makes the wait below race-free.
    pub(crate) fn progress_epoch(&self) -> u64 {
        self.state.lock().unwrap().progress_epoch
    }

    /// Blocks until the progress epoch has moved past `since`, or
    /// maintenance has failed, or the worker is stopping. Never a bare
    /// sleep or poll: the check and the wait share one lock, so a
    /// commit that bumps the epoch between a caller's read of it and
    /// this call can never be missed.
    pub(crate) fn wait_for_progress(&self, since: u64) {
        let mut s = self.state.lock().unwrap();
        while s.progress_epoch == since && s.error.is_none() && !s.stop {
            s = self.condvar.wait(s).unwrap();
        }
    }

    /// Whether the worker has been told to stop. A writer that somehow
    /// wakes with nothing else changed (no epoch bump, no error) but
    /// finds this true must give up rather than loop forever waiting
    /// for a worker that is going away.
    pub(crate) fn is_stopped(&self) -> bool {
        self.state.lock().unwrap().stop
    }

    /// Marks entry into / exit from a backpressure wait (11.5). Paired
    /// calls around `wait_for_progress` in
    /// `SharedKiban::wait_for_write_room`.
    pub(crate) fn writer_started_waiting(&self) {
        let mut s = self.state.lock().unwrap();
        s.waiting_writers += 1;
        drop(s);
        self.condvar.notify_all();
    }

    pub(crate) fn writer_stopped_waiting(&self) {
        self.state.lock().unwrap().waiting_writers -= 1;
    }

    /// How many `SharedKiban` callers are, right now, genuinely parked
    /// waiting for L0 room — not inferred from elapsed time. The same
    /// fact is also part of `stats()` (11.7), which reads the field
    /// directly under its own already-held lock rather than calling
    /// this (a `Mutex` is not reentrant).
    #[cfg(test)]
    pub(crate) fn waiting_writers(&self) -> usize {
        self.state.lock().unwrap().waiting_writers
    }

    /// Records that one `SharedKiban` mutation call genuinely had to
    /// wait for L0 room — call exactly once per call that blocks, not
    /// once per wake/recheck cycle within it (11.7).
    pub(crate) fn record_write_stall(&self) {
        let mut s = self.state.lock().unwrap();
        s.write_stalls = s.write_stalls.saturating_add(1);
    }

    /// A cheap, lock-once read of every maintenance counter (11.7). No
    /// I/O, no effect on any counter it reads.
    pub(crate) fn stats(&self) -> MaintenanceStats {
        let s = self.state.lock().unwrap();
        MaintenanceStats {
            compaction_running: s.busy,
            compactions_completed: s.compactions_completed,
            compactions_failed: s.compactions_failed,
            compaction_input_bytes: s.compaction_input_bytes,
            compaction_output_bytes: s.compaction_output_bytes,
            waiting_writers: s.waiting_writers,
            write_stalls: s.write_stalls,
            flushes_completed: s.flushes_completed,
            flushes_failed: s.flushes_failed,
        }
    }

    /// Blocks until at least one writer is genuinely parked in a
    /// backpressure wait — for deterministic tests, instead of
    /// inferring blocking from elapsed time.
    #[cfg(test)]
    pub(crate) fn wait_until_writer_waiting(&self) {
        let mut s = self.state.lock().unwrap();
        while s.waiting_writers == 0 {
            s = self.condvar.wait(s).unwrap();
        }
    }
}

fn worker_loop(engine: Arc<Mutex<Kiban>>, m: Arc<Maintenance>) {
    loop {
        {
            let mut s = m.state.lock().unwrap();
            while !s.wake && !s.stop {
                s = m.condvar.wait(s).unwrap();
            }
            if s.stop {
                return;
            }
            s.wake = false;
            s.busy = true;
        }

        if m.state.lock().unwrap().error.is_none() {
            // A stalled writer (11.5) may be parked waiting for exactly
            // the progress this cycle would make. If this panics — a
            // bug, not an expected runtime condition — that must still
            // become a recorded, wake-triggering failure rather than a
            // silently vanished thread nobody notified.
            let engine = &engine;
            let m_ref = &m;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_pending_maintenance(engine, m_ref);
            }));
            if let Err(payload) = outcome {
                record_error(
                    &m,
                    format!("worker thread panicked: {}", panic_message(&payload)),
                );
            }
        }

        {
            let mut s = m.state.lock().unwrap();
            s.busy = false;
        }
        m.condvar.notify_all();

        if m.state.lock().unwrap().stop {
            return;
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Runs every job the engine currently needs — flush, then compaction
/// (11.8: memory pressure outranks maintenance debt) — with BUILD
/// moved off the lock for both. Compaction's own priority order within
/// itself (L0 first, then a level cascade) is unchanged, mirroring
/// `Kiban::maybe_compact`'s decision function exactly.
///
/// At most one immutable memtable can ever be pending (this phase's
/// own rule), so at most one flush job runs per pass through the outer
/// loop; `continue` after committing one so the loop rechecks — a
/// fresh freeze can land while this job's BUILD was running unlocked.
fn run_pending_maintenance(engine: &Arc<Mutex<Kiban>>, m: &Arc<Maintenance>) {
    let mut cascade_level = 1u32;
    loop {
        let flush_plan = {
            let Ok(mut guard) = engine.lock() else { return };
            guard.plan_flush()
        };
        if let Some(plan) = flush_plan {
            #[cfg(test)]
            m.test.before_flush_build();

            match plan.build() {
                Ok(output) => {
                    let committed = {
                        let Ok(mut guard) = engine.lock() else { return };
                        guard.commit_flush(plan, output)
                    };
                    match committed {
                        Ok(()) => record_flush_success(m),
                        Err(e) => {
                            record_flush_error(m, e.to_string());
                            return;
                        }
                    }
                }
                Err(e) => {
                    record_flush_error(m, e.to_string());
                    return;
                }
            }
            continue;
        }

        let plan = {
            let Ok(mut guard) = engine.lock() else { return };
            guard.plan_next_compaction(&mut cascade_level)
        };
        let Some(plan) = plan else { return };

        #[cfg(test)]
        m.test.before_build();

        match plan.build() {
            Ok(outputs) => {
                let committed = {
                    let Ok(mut guard) = engine.lock() else { return };
                    guard.commit_compaction(plan, outputs)
                };
                match committed {
                    // A successful commit is exactly the progress a
                    // stalled writer (11.5) is waiting to recheck
                    // against — wake it now, not after the whole
                    // cascade finishes.
                    Ok(outcome) => record_compaction_success(m, outcome),
                    Err(e) => {
                        record_error(m, e.to_string());
                        return;
                    }
                }
            }
            Err(e) => {
                record_error(m, e.to_string());
                return;
            }
        }
    }
}

fn record_flush_success(m: &Arc<Maintenance>) {
    {
        let mut s = m.state.lock().unwrap();
        s.flushes_completed = s.flushes_completed.saturating_add(1);
        s.progress_epoch += 1;
    }
    m.condvar.notify_all();
}

fn record_flush_error(m: &Arc<Maintenance>, msg: String) {
    {
        let mut s = m.state.lock().unwrap();
        s.error = Some(MaintenanceError(msg));
        s.flushes_failed = s.flushes_failed.saturating_add(1);
        s.progress_epoch += 1;
    }
    m.condvar.notify_all();
}

fn record_compaction_success(m: &Arc<Maintenance>, outcome: crate::db::CompactionOutcome) {
    {
        let mut s = m.state.lock().unwrap();
        s.compactions_completed = s.compactions_completed.saturating_add(1);
        s.compaction_input_bytes = s.compaction_input_bytes.saturating_add(outcome.input_bytes);
        s.compaction_output_bytes = s
            .compaction_output_bytes
            .saturating_add(outcome.output_bytes);
        s.progress_epoch += 1;
    }
    m.condvar.notify_all();
}

fn record_error(m: &Arc<Maintenance>, msg: String) {
    {
        let mut s = m.state.lock().unwrap();
        s.error = Some(MaintenanceError(msg));
        s.compactions_failed = s.compactions_failed.saturating_add(1);
        s.progress_epoch += 1;
    }
    m.condvar.notify_all();
}

// ---------------------------------------------------------- test hooks
//
// Deterministic control for the worker thread: freeze it at a known
// point (after PLAN, before BUILD) instead of sleeping, and run a
// one-shot closure ON the worker thread before it resumes — needed
// because fault injection (`sys::install_fault`) is thread-local, so a
// fault meant for the worker's own I/O must be installed from the
// worker's own thread.

#[cfg(test)]
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum CheckpointState {
    #[default]
    Idle,
    Armed,
    Reached,
}

#[cfg(test)]
#[derive(Default)]
struct Checkpoint {
    state: Mutex<CheckpointState>,
    condvar: Condvar,
}

#[cfg(test)]
impl Checkpoint {
    fn arm(&self) {
        *self.state.lock().unwrap() = CheckpointState::Armed;
    }

    /// Worker side: a no-op unless armed; if armed, announces it has
    /// reached this point and blocks until a test releases it.
    fn hit(&self) {
        let mut s = self.state.lock().unwrap();
        if *s != CheckpointState::Armed {
            return;
        }
        *s = CheckpointState::Reached;
        self.condvar.notify_all();
        while *s == CheckpointState::Reached {
            s = self.condvar.wait(s).unwrap();
        }
    }

    fn wait_reached(&self) {
        let mut s = self.state.lock().unwrap();
        while *s != CheckpointState::Reached {
            s = self.condvar.wait(s).unwrap();
        }
    }

    fn release(&self) {
        *self.state.lock().unwrap() = CheckpointState::Idle;
        self.condvar.notify_all();
    }
}

#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    /// Compaction's build point (11.4).
    before_build: Checkpoint,
    /// A pending flush's build point (11.8) — deliberately a separate
    /// checkpoint from compaction's: since 11.8, `SharedKiban::flush()`
    /// and auto-freeze both route their SST construction through this
    /// same worker, and a test arming *compaction's* checkpoint (e.g.
    /// to freeze the worker mid-cascade while seeding L0 tables via
    /// ordinary `flush()` calls) must not also freeze every flush
    /// those seed calls themselves depend on to ever return.
    before_flush_build: Checkpoint,
    pending: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[cfg(test)]
impl TestHooks {
    /// Blocks first (if armed), THEN runs any pending injected closure —
    /// so a test can wait until the worker is frozen here, install a
    /// fault via `inject_on_worker`, and release: the fault always lands
    /// right before BUILD, never before the freeze that would let the
    /// test miss the window to install it.
    fn before_build(&self) {
        self.before_build.hit();
        if let Some(f) = self.pending.lock().unwrap().take() {
            f();
        }
    }

    /// Same discipline as `before_build`, for a pending flush's BUILD.
    fn before_flush_build(&self) {
        self.before_flush_build.hit();
        if let Some(f) = self.pending.lock().unwrap().take() {
            f();
        }
    }
}

#[cfg(test)]
impl Maintenance {
    /// Freezes the worker the next time it reaches the point after PLAN
    /// and before a COMPACTION's BUILD starts.
    pub(crate) fn arm_before_build(&self) {
        self.test.before_build.arm();
    }

    /// Blocks until the worker has reached that point and is frozen
    /// there.
    pub(crate) fn wait_before_build_reached(&self) {
        self.test.before_build.wait_reached();
    }

    /// Lets a frozen worker continue into BUILD.
    pub(crate) fn release_before_build(&self) {
        self.test.before_build.release();
    }

    /// Freezes the worker the next time it reaches the point after PLAN
    /// and before a pending FLUSH's BUILD starts (11.8).
    pub(crate) fn arm_before_flush_build(&self) {
        self.test.before_flush_build.arm();
    }

    /// Blocks until the worker has reached that point and is frozen
    /// there.
    pub(crate) fn wait_before_flush_build_reached(&self) {
        self.test.before_flush_build.wait_reached();
    }

    /// Lets a frozen worker continue into a flush's BUILD.
    pub(crate) fn release_before_flush_build(&self) {
        self.test.before_flush_build.release();
    }

    /// Runs `f` on the worker thread itself, once, immediately before
    /// its next BUILD — e.g. `sys::install_fault(n)`, which is
    /// thread-local and must be set on the thread that will do the I/O.
    pub(crate) fn inject_on_worker(&self, f: impl FnOnce() + Send + 'static) {
        *self.test.pending.lock().unwrap() = Some(Box::new(f));
    }

    /// Blocks until the worker is idle and has no outstanding wake —
    /// i.e. it has settled, rather than sleeping a guessed duration.
    pub(crate) fn wait_settled(&self) {
        let mut s = self.state.lock().unwrap();
        while s.busy || s.wake {
            s = self.condvar.wait(s).unwrap();
        }
    }
}
