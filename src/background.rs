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
        if dead && s.error.is_none() && !s.stop {
            s.error = Some(MaintenanceError(
                "maintenance worker thread exited unexpectedly".to_string(),
            ));
        }
        s.error.clone()
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
            run_pending_compactions(&engine, &m);
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

/// Runs every compaction job the engine currently needs, in
/// `Kiban::maybe_compact`'s own priority order (L0 first, then a level
/// cascade) — the exact same decision function the synchronous path
/// uses, just with BUILD moved off the lock.
fn run_pending_compactions(engine: &Arc<Mutex<Kiban>>, m: &Arc<Maintenance>) {
    let mut cascade_level = 1u32;
    loop {
        let plan = {
            let Ok(mut guard) = engine.lock() else { return };
            guard.plan_next_compaction(&mut cascade_level)
        };
        let Some(plan) = plan else { return };

        #[cfg(test)]
        m.test.before_build();

        match plan.build() {
            Ok(outputs) => {
                let Ok(mut guard) = engine.lock() else { return };
                if let Err(e) = guard.commit_compaction(plan, outputs) {
                    record_error(m, e.to_string());
                    return;
                }
            }
            Err(e) => {
                record_error(m, e.to_string());
                return;
            }
        }
    }
}

fn record_error(m: &Arc<Maintenance>, msg: String) {
    m.state.lock().unwrap().error = Some(MaintenanceError(msg));
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
    before_build: Checkpoint,
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
}

#[cfg(test)]
impl Maintenance {
    /// Freezes the worker the next time it reaches the point after PLAN
    /// and before BUILD starts.
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
