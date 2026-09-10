//! Phase 11.17 read-concurrency shootout prototype: a gate-free published
//! read state for foreground point lookups.
//!
//! Controlled architectural experiment, feature `readview`. It adds a
//! second GET path alongside the current [`ShardedRwLock`](crate::engine_lock::ShardedRwLock)
//! read path; production behavior is untouched.
//!
//! # The published model
//!
//! ```text
//! Kiban writer state (still behind the write gate)
//!        |
//!        +-- current memtable     -> Arc<RwLock<Memtable>>    (slot 1)
//!        +-- frozen memtable      -> Option<Arc<RwLock<Memtable>>> (slot 2)
//!        +-- published Version    -> StdArc<Version>          (slot 3)
//!        |
//!        v
//!   AtomicArc<ReadView>  -- one SeqCst pointer swap per structural commit
//!        |                  (freeze, flush commit, compaction commit)
//!        v
//!   reader: load_full() -> stable Arc clone -> probe, no engine gate
//! ```
//!
//! # Visibility invariants
//!
//! * **PUT visibility.** A put appends to the memtable under the
//!   memtable's write lock (inside the write gate) and bumps
//!   `last_sequence`. A GET loads the view atomically and then probes
//!   the memtable under its read lock. If a put's append completed
//!   before a GET's pointer load, the GET sees it; if the put landed
//!   after, the GET does not. Same ordering shape as the locked path,
//!   where the gate acquire is the linearization point.
//! * **Memtable rotation.** Freeze swaps the triple in one atomic
//!   store: the old memtable becomes the frozen slot, a fresh empty
//!   memtable becomes the current one. A reader always sees the old
//!   or the new triple — never "neither".
//! * **Flush.** `commit_flush` clears the frozen slot and swaps the
//!   version (its SST now carries the data) in one gate hold, then
//!   publishes. A reader on the old triple still holds the frozen
//!   memtable's `Arc`, so the data never disappears for it.
//! * **Compaction.** `commit_compaction` swaps the version; the old
//!   `Arc<Version>` (and every `Arc<TableEntry>` in it) stays alive in
//!   the graveyard and in every reader's clone, so old SST files are
//!   never deleted while a reader still references them
//!   (`reclaim_obsolete`'s `strong_count == 1` check).
//! * **Tombstones / snapshots.** Tombstones are entries like any
//!   other; a reader's view-bound filtering uses the triple's
//!   `last_sequence` captured at publication. `SharedSnapshot`
//!   semantics are untouched by this module.

// Scaffold only (see the module doc above): nothing in the crate wires
// this up to a real GET path yet, so a plain `--features readview` lib
// build has no caller for any of it — expected, not a bug.
#![allow(dead_code)]

use std::mem::ManuallyDrop;
use std::sync::Arc as StdArc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::db::Version;

/// A point-in-time immutable read triple, published atomically.
pub(crate) struct ReadView {
    pub(crate) memtable: std::sync::Arc<RwLock<crate::memtable::Memtable>>,
    pub(crate) immutable: Option<std::sync::Arc<RwLock<crate::memtable::Memtable>>>,
    pub(crate) version: std::sync::Arc<Version>,
    pub(crate) last_sequence: u64,
}

/// One hand-rolled atomic `Arc` slot (ArcSwap-shaped, minimal).
///
/// SAFETY: the slot owns one strong reference to the current value.
/// `load` does a SeqCst pointer load and then increments the strong
/// count, which is safe only because `store` never frees the value it
/// replaces: the swapped-out `Arc` moves into a graveyard held for the
/// slot's whole lifetime. A reader between the load and its increment
/// can therefore never touch freed memory. This bounds the prototype's
/// growth at one graveyard entry per structural commit; retiring them
/// would need epoch/hazard-style reclamation — deliberately out of
/// scope for this phase (see the 11.17 rules).
pub(crate) struct AtomicArc<T> {
    ptr: AtomicPtr<T>,
    graveyard: Mutex<Vec<std::sync::Arc<T>>>,
}

// SAFETY: pointer and graveyard move (and share) as one unit; T: Send + Sync
// are required for the same reasons as ShardedRwLock's impls.
unsafe impl<T: Send + Sync> Send for AtomicArc<T> {}
unsafe impl<T: Send + Sync> Sync for AtomicArc<T> {}

impl<T> AtomicArc<T> {
    pub(crate) fn new(value: StdArc<T>) -> Self {
        AtomicArc {
            ptr: AtomicPtr::new(StdArc::into_raw(value) as *mut T),
            graveyard: Mutex::new(Vec::new()),
        }
    }

    /// Stable handle: pointer load + refcount increment as one unit. The
    /// SeqCst store in [`AtomicArc::store`] publishes the pointer before
    /// any reclamation of the replaced value, and the replaced value is
    /// never freed (graveyard), so a reader between the load and the
    /// increment can never dereference freed memory.
    pub(crate) fn load_full(&self) -> std::sync::Arc<T> {
        let ptr = self.ptr.load(Ordering::SeqCst);
        assert!(!ptr.is_null());
        // SAFETY: ptr came from `Arc::into_raw` in `new`/`store`; the
        // slot still owns one reference to it (it is only moved to the
        // graveyard, never freed), so the allocation is live here. The
        // temp guard is dropped with its count intact, then a fresh
        // clone is handed to the caller.
        let temp = ManuallyDrop::new(unsafe { StdArc::from_raw(ptr) });
        std::sync::Arc::clone(&temp)
    }

    /// Publishes `value`; the replaced value moves to the graveyard and is
    /// never freed for the process lifetime (see the SAFETY note).
    pub(crate) fn store(&self, value: StdArc<T>) {
        let new_raw = std::sync::Arc::into_raw(value) as *mut T;
        let old = self.ptr.swap(new_raw, Ordering::SeqCst);
        // SAFETY: old came from `Arc::into_raw` in a previous `store`; the
        // slot's implicit reference transfers to this call, which moves
        // the value to the graveyard WITHOUT decrementing, so a reader
        // between the load and its own increment can never free it.
        let retired = unsafe { StdArc::from_raw(old) };
        self.graveyard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(retired);
    }
}
