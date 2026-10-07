use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

// Loom models the gate's real protocol (below, `loom_tests`) by swapping
// these primitives for loom-modeled equivalents; the non-loom build is
// byte-identical std code.
// AtomicUsize is only reached through `mod tests`/`mod loom_tests` here
// (`use super::*`), never by non-test lib code under this branch — the
// loom build's own shard counter is dropped (see below).
#[cfg(feature = "loom")]
#[allow(unused_imports)]
use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(feature = "loom")]
use loom::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(feature = "loom")]
use loom::thread::yield_now;
#[cfg(not(feature = "loom"))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(feature = "loom"))]
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(not(feature = "loom"))]
use std::thread::yield_now;

// Shard pick is per-thread round-robin over the process; under loom the
// global ticket counter is dropped (loom atomics are not const-constructible)
// and every model thread lands on shard 0, which only tightens the explored
// contention — shard choice is load balancing, never a safety property.
pub(crate) const ENGINE_READ_SHARDS: usize = 8;

#[cfg(not(feature = "loom"))]
static NEXT_SHARD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    #[cfg(not(feature = "loom"))]
    static READER_SHARD: usize = NEXT_SHARD.fetch_add(1, Ordering::Relaxed) % ENGINE_READ_SHARDS;
    #[cfg(feature = "loom")]
    static READER_SHARD: usize = const { 0 };
}

/// One engine value guarded by eight independent reader gates.
///
/// SAFETY: a read guard owns one shard read lock before exposing `&T`. A
/// write guard owns `writer_serial` and every shard write lock, acquired in
/// ascending order, before exposing `&mut T`. Thus readers may coexist, but
/// no reader can coexist with a writer and writers cannot coexist. Writer
/// intent affects progress only; these shard locks alone establish aliasing.
pub(crate) struct ShardedRwLock<T> {
    shards: [RwLock<()>; ENGINE_READ_SHARDS],
    writer_serial: Mutex<()>,
    writer_pending: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: moving the lock moves its owned T and synchronization state
// together, so no thread-affinity or aliasing assumption is broken by a
// cross-thread move. T: Send is required because a value written on one
// thread may be dropped (or handed to a reader) on another.
unsafe impl<T: Send> Send for ShardedRwLock<T> {}
// SAFETY: `&ShardedRwLock<T>` can hand out concurrent `&T` to any number of
// reader threads via `read()`, so T must be Sync. It can also hand exclusive
// `&mut T` access to a writer thread that need not be the thread that
// created the value or holds the outer `&ShardedRwLock<T>`, so T must also
// be Send. Both bounds are required, not just sufficient: dropping either
// would let this impl manufacture cross-thread `&T`/`&mut T` for a type that
// does not itself promise that is sound.
unsafe impl<T: Send + Sync> Sync for ShardedRwLock<T> {}

pub(crate) struct ReadGuard<'a, T> {
    _shard: RwLockReadGuard<'a, ()>,
    value: *const T,
    _marker: PhantomData<&'a T>,
}
impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: this guard owns a live read lock on its shard, and a
        // WriteGuard cannot exist without holding a write lock on every
        // shard, so no writer can hold `&mut T` while this `&T` is live.
        // The returned reference borrows `self`, so PhantomData<&'a T> ties
        // its lifetime to this guard: it cannot outlive the shard lock that
        // makes it sound, and `value` (a raw pointer, not a reference) is
        // never itself exposed by any method.
        unsafe { &*self.value }
    }
}

struct WriterIntent<'a>(&'a AtomicBool);
impl Drop for WriterIntent<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// WriteGuard's fields are dropped in declaration order (Rust drops struct
// fields top-to-bottom), so `_intent` releases writer_pending BEFORE
// `_serial` and `_shards` release their real locks. That is intentional and
// safe: writer_pending is a liveness hint only, never a safety mechanism.
// A thread that observes writer_pending == false during this window still
// must acquire the real, still-held shard/serial locks to make progress,
// and will correctly block until this guard finishes dropping. No aliasing
// is possible from the flag flipping early; see the SAFETY notes on
// Deref/DerefMut below for what actually establishes exclusion.
pub(crate) struct WriteGuard<'a, T> {
    _intent: WriterIntent<'a>,
    _serial: MutexGuard<'a, ()>,
    _shards: [RwLockWriteGuard<'a, ()>; ENGINE_READ_SHARDS],
    value: *mut T,
    _marker: PhantomData<&'a mut T>,
}
impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: this guard holds a write lock on every shard plus
        // `writer_serial`, all still live while `self` is borrowed, so no
        // reader can hold a shard read lock and no other writer can hold
        // `writer_serial` concurrently. PhantomData<&'a mut T> ties any
        // reference derived here to this guard's borrow.
        unsafe { &*self.value }
    }
}
impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: same exclusivity argument as Deref above, and `&mut self`
        // here additionally guarantees no other reference to this guard
        // (and thus no other derived `&T`/`&mut T`) exists concurrently.
        unsafe { &mut *self.value }
    }
}

impl<T> ShardedRwLock<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            shards: std::array::from_fn(|_| RwLock::new(())),
            writer_serial: Mutex::new(()),
            writer_pending: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
    pub(crate) fn read(&self) -> Result<ReadGuard<'_, T>, ()> {
        self.read_from_shard(READER_SHARD.with(|slot| *slot))
    }

    fn read_from_shard(&self, shard: usize) -> Result<ReadGuard<'_, T>, ()> {
        loop {
            if self.writer_pending.load(Ordering::Acquire) {
                yield_now();
                continue;
            }
            let guard = self.shards[shard].read().map_err(|_| ())?;
            if !self.writer_pending.load(Ordering::Acquire) {
                // SAFETY: this shard guard excludes writers, which require every shard.
                return Ok(ReadGuard {
                    _shard: guard,
                    value: self.value.get(),
                    _marker: PhantomData,
                });
            }
            drop(guard);
            yield_now();
        }
    }

    #[cfg(test)]
    fn writer_pending(&self) -> bool {
        self.writer_pending.load(Ordering::Acquire)
    }

    /// Poisons exactly one shard's RwLock, for tests exercising partial-
    /// acquisition failure. Does not go through `write()`, so it does not
    /// touch `writer_serial` or `writer_pending`.
    #[cfg(test)]
    fn poison_shard(&self, idx: usize) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.shards[idx].write().unwrap();
            panic!("test-induced shard poison");
        }));
    }

    pub(crate) fn write(&self) -> Result<WriteGuard<'_, T>, ()> {
        self.write_with_hook(|_shard_index| {})
    }

    /// Same acquisition protocol as `write()`, with a hook invoked after
    /// each shard's write lock is acquired (its argument is the shard
    /// index just acquired). `write()` calls this with a no-op closure, so
    /// production behavior is unchanged; tests use the hook to observe or
    /// pause mid-acquisition without duplicating the acquisition loop.
    fn write_with_hook(&self, mut after_shard: impl FnMut(usize)) -> Result<WriteGuard<'_, T>, ()> {
        let serial = self.writer_serial.lock().map_err(|_| ())?;
        self.writer_pending.store(true, Ordering::Release);
        let intent = WriterIntent(&self.writer_pending);
        let mut guards = Vec::with_capacity(ENGINE_READ_SHARDS);
        for (i, shard) in self.shards.iter().enumerate() {
            guards.push(shard.write().map_err(|_| ())?);
            after_shard(i);
        }
        let shards: [RwLockWriteGuard<'_, ()>; ENGINE_READ_SHARDS] =
            guards.try_into().map_err(|_| ())?;
        // SAFETY: every shard write guard excludes all reads and every other writer.
        Ok(WriteGuard {
            _intent: intent,
            _serial: serial,
            _shards: shards,
            value: self.value.get(),
            _marker: PhantomData,
        })
    }
}

#[cfg(test)]
fn current_shard() -> usize {
    READER_SHARD.with(|slot| *slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn readers_and_writers_preserve_value() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let start = Arc::new(Barrier::new(9));
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let lock = lock.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..1000 {
                        let _ = *lock.read().unwrap();
                    }
                })
            })
            .collect();
        start.wait();
        for _ in 0..100 {
            *lock.write().unwrap() += 1;
        }
        for reader in readers {
            reader.join().unwrap();
        }
        assert_eq!(*lock.read().unwrap(), 100);
    }

    #[test]
    fn writer_panic_poison_is_reported() {
        let lock = ShardedRwLock::new(0usize);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lock.write().unwrap();
            panic!();
        }));
        assert!(lock.write().is_err());
    }

    #[test]
    fn different_and_same_shard_readers_coexist() {
        let lock = Arc::new(ShardedRwLock::new(7usize));
        for (a, b) in [(0, 1), (0, 0)] {
            let start = Arc::new(Barrier::new(3));
            let release = Arc::new(Barrier::new(3));
            let first = {
                let lock = lock.clone();
                let start = start.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    let guard = lock.read_from_shard(a).unwrap();
                    start.wait();
                    release.wait();
                    *guard
                })
            };
            let second = {
                let lock = lock.clone();
                let start = start.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    let guard = lock.read_from_shard(b).unwrap();
                    start.wait();
                    release.wait();
                    *guard
                })
            };
            start.wait();
            release.wait();
            assert_eq!(first.join().unwrap(), 7);
            assert_eq!(second.join().unwrap(), 7);
        }
    }

    #[test]
    fn reader_panic_does_not_poison() {
        let lock = ShardedRwLock::new(0usize);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lock.read().unwrap();
            panic!();
        }));
        assert!(lock.read().is_ok());
        assert!(lock.write().is_ok());
    }

    #[test]
    fn held_reader_excludes_writer_mutation() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let held = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let reader = {
            let lock = lock.clone();
            let held = held.clone();
            let release = release.clone();
            std::thread::spawn(move || {
                let _guard = lock.read_from_shard(3).unwrap();
                held.wait();
                release.wait();
            })
        };
        held.wait();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let writer = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                let mut guard = lock.write().unwrap();
                entered_tx.send(()).unwrap();
                *guard = 1;
            })
        };
        while !lock.writer_pending() {
            std::thread::yield_now();
        }
        assert!(entered_rx.try_recv().is_err());
        release.wait();
        entered_rx.recv().unwrap();
        reader.join().unwrap();
        writer.join().unwrap();
        assert_eq!(*lock.read().unwrap(), 1);
    }

    #[test]
    fn writer_intent_clears_on_success() {
        let lock = ShardedRwLock::new(0usize);
        drop(lock.write().unwrap());
        assert!(!lock.writer_pending());
    }

    /// TEST 1: a writer must be excluded by EVERY held reader shard, not
    /// just the first one released. Three readers hold three distinct
    /// shards; the writer must remain shut out until all three release, in
    /// order, proven by an mpsc channel rather than timing.
    #[test]
    fn multiple_held_reader_shards_block_writer() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let held_shards = [1usize, 3, 6];
        let held = Arc::new(Barrier::new(held_shards.len() + 1));
        let releases: Vec<Arc<Barrier>> = held_shards
            .iter()
            .map(|_| Arc::new(Barrier::new(2)))
            .collect();

        let readers: Vec<_> = held_shards
            .iter()
            .zip(releases.iter())
            .map(|(&shard, release)| {
                let lock = lock.clone();
                let held = held.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    let _guard = lock.read_from_shard(shard).unwrap();
                    held.wait();
                    release.wait();
                })
            })
            .collect();
        held.wait();

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let writer = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                let mut guard = lock.write().unwrap();
                entered_tx.send(()).unwrap();
                *guard = 42;
            })
        };
        while !lock.writer_pending() {
            std::thread::yield_now();
        }
        assert!(entered_rx.try_recv().is_err());

        releases[0].wait();
        assert!(
            entered_rx.try_recv().is_err(),
            "writer entered after only 1 of 3 held reader shards released"
        );
        releases[1].wait();
        assert!(
            entered_rx.try_recv().is_err(),
            "writer entered after only 2 of 3 held reader shards released"
        );
        releases[2].wait();

        entered_rx.recv().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }
        writer.join().unwrap();
        assert_eq!(*lock.read().unwrap(), 42);
    }

    /// TEST 2: two writers must serialize. Writer A is proven to still hold
    /// its guard (via a Barrier neither side can pass alone) while writer B
    /// is proven to have started its own acquisition attempt, then the
    /// max-concurrent-writers counter must never exceed 1.
    #[test]
    fn two_writers_serialize_deterministically() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let current_writers = Arc::new(AtomicUsize::new(0));
        let max_writers = Arc::new(AtomicUsize::new(0));
        let release_a = Arc::new(Barrier::new(2));
        let (entered_a_tx, entered_a_rx) = std::sync::mpsc::channel();
        let (entered_b_tx, entered_b_rx) = std::sync::mpsc::channel();
        let b_attempting = Arc::new(AtomicBool::new(false));

        let writer_a = {
            let lock = lock.clone();
            let current_writers = current_writers.clone();
            let max_writers = max_writers.clone();
            let release_a = release_a.clone();
            std::thread::spawn(move || {
                let mut guard = lock.write().unwrap();
                let n = current_writers.fetch_add(1, Ordering::SeqCst) + 1;
                max_writers.fetch_max(n, Ordering::SeqCst);
                *guard += 1;
                entered_a_tx.send(()).unwrap();
                release_a.wait();
                current_writers.fetch_sub(1, Ordering::SeqCst);
            })
        };
        entered_a_rx.recv().unwrap();

        let writer_b = {
            let lock = lock.clone();
            let current_writers = current_writers.clone();
            let max_writers = max_writers.clone();
            let b_attempting = b_attempting.clone();
            std::thread::spawn(move || {
                b_attempting.store(true, Ordering::Release);
                let mut guard = lock.write().unwrap();
                let n = current_writers.fetch_add(1, Ordering::SeqCst) + 1;
                max_writers.fetch_max(n, Ordering::SeqCst);
                *guard += 1;
                entered_b_tx.send(()).unwrap();
                current_writers.fetch_sub(1, Ordering::SeqCst);
            })
        };
        while !b_attempting.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        // Writer A's guard is provably still alive: A is blocked on
        // release_a, which requires this thread's wait() too, and that
        // hasn't happened yet. So B's write() call is really still blocked
        // by the live writer_serial/shard locks, not by scheduling luck.
        assert!(
            entered_b_rx.try_recv().is_err(),
            "writer B entered while writer A still held its guard"
        );

        release_a.wait();
        writer_a.join().unwrap();
        entered_b_rx.recv().unwrap();
        writer_b.join().unwrap();

        assert_eq!(max_writers.load(Ordering::SeqCst), 1);
        assert_eq!(*lock.read().unwrap(), 2);
    }

    /// TEST 3: pause a writer after acquiring a prefix of shards (0..=3)
    /// but before the rest. At that checkpoint, prove write() has not
    /// returned (no WriteGuard exists, so no `&mut T` can exist anywhere
    /// in the program by the type system alone), that an already-acquired
    /// shard is genuinely exclusively locked, and that a not-yet-acquired
    /// shard is genuinely still free.
    #[test]
    fn partial_acquisition_exposes_no_mutable_access() {
        let lock = Arc::new(ShardedRwLock::new(41usize));
        let reached = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let (done_tx, done_rx) = std::sync::mpsc::channel();

        let writer = {
            let lock = lock.clone();
            let reached = reached.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                let mut guard = lock
                    .write_with_hook(|i| {
                        if i == 3 {
                            reached.wait();
                            resume.wait();
                        }
                    })
                    .unwrap();
                *guard = 99;
                done_tx.send(()).unwrap();
            })
        };

        reached.wait();
        assert!(
            done_rx.try_recv().is_err(),
            "write() returned before the acquisition checkpoint resumed"
        );
        // Shards 0..=3 are genuinely held exclusively right now.
        assert!(
            lock.shards[1].try_read().is_err(),
            "shard 1 should still be write-locked mid-acquisition"
        );
        // Shard 6 has not been reached yet and must be genuinely free.
        assert!(
            lock.shards[6].try_read().is_ok(),
            "shard 6 should still be free before the writer reaches it"
        );

        resume.wait();
        writer.join().unwrap();
        done_rx.recv().unwrap();
        assert_eq!(*lock.read().unwrap(), 99);
    }

    /// TEST 4: while a writer is pending (blocked behind an old reader),
    /// new readers must never remain admitted before the writer finishes.
    /// Each new reader checks a `writer_done` flag at the instant it is
    /// actually admitted, which is a hard ordering proof rather than a
    /// timing one: no reader's `read_from_shard` loop can return while
    /// writer_pending stays true, and writer_done is set before the
    /// WriteGuard is dropped (which is what clears writer_pending).
    #[test]
    fn pending_writer_repels_new_readers() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let held = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let old_shard = 4;
        let old_reader = {
            let lock = lock.clone();
            let held = held.clone();
            let release = release.clone();
            std::thread::spawn(move || {
                let _guard = lock.read_from_shard(old_shard).unwrap();
                held.wait();
                release.wait();
            })
        };
        held.wait();

        let writer_done = Arc::new(AtomicBool::new(false));
        let writer = {
            let lock = lock.clone();
            let writer_done = writer_done.clone();
            std::thread::spawn(move || {
                let mut guard = lock.write().unwrap();
                *guard = 7;
                writer_done.store(true, Ordering::Release);
            })
        };
        while !lock.writer_pending() {
            std::thread::yield_now();
        }

        let violations = Arc::new(AtomicUsize::new(0));
        let new_readers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                let writer_done = writer_done.clone();
                let violations = violations.clone();
                std::thread::spawn(move || {
                    let _guard = lock.read().unwrap();
                    if !writer_done.load(Ordering::Acquire) {
                        violations.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        release.wait();
        old_reader.join().unwrap();
        writer.join().unwrap();
        for reader in new_readers {
            reader.join().unwrap();
        }
        assert_eq!(
            violations.load(Ordering::Relaxed),
            0,
            "a new reader was admitted before the pending writer finished"
        );
        assert_eq!(*lock.read().unwrap(), 7);
    }

    /// TEST 5: a non-panic acquisition failure (a poisoned later shard)
    /// must still clear writer_pending, release writer_serial, and leave
    /// unrelated unpoisoned shards usable. Poisoning is real (via a real
    /// panic on that one shard's RwLock), not simulated.
    #[test]
    fn writer_intent_clears_after_acquisition_failure() {
        let lock = ShardedRwLock::new(0usize);
        lock.poison_shard(2);

        assert!(lock.write().is_err());
        assert!(!lock.writer_pending());
        assert!(
            lock.writer_serial.try_lock().is_ok(),
            "writer_serial must not remain held after a failed acquisition"
        );
        assert!(
            lock.read_from_shard(5).is_ok(),
            "an unrelated, unpoisoned shard must still admit readers"
        );
    }

    /// TEST 6: a panic mid-acquisition (after writer_pending is set, before
    /// the WriteGuard is fully built) must still clear writer_pending via
    /// unwind-driven drops. Lock poisoning from the unwind is expected and
    /// not asserted on; only the flag's liveness is.
    #[test]
    fn writer_intent_clears_on_unwind_during_acquisition() {
        let lock = ShardedRwLock::new(0usize);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = lock.write_with_hook(|i| {
                if i == 3 {
                    panic!("test-induced unwind mid-acquisition");
                }
            });
        }));
        assert!(result.is_err());
        assert!(!lock.writer_pending());
    }

    /// TEST 7: poisoning a LATE shard (not shard 0) exercises real partial
    /// acquisition: earlier shards succeed for real before the failure.
    #[test]
    fn late_shard_poison_cleans_up_partial_acquisition() {
        let lock = ShardedRwLock::new(0usize);
        lock.poison_shard(6);

        assert!(lock.write().is_err());
        assert!(!lock.writer_pending());
        assert!(
            lock.writer_serial.try_lock().is_ok(),
            "writer_serial must not remain held after a late-shard poison failure"
        );
        for shard in [0usize, 1, 2, 3, 4, 5, 7] {
            assert!(
                lock.read_from_shard(shard).is_ok(),
                "shard {shard} should have released after the failed acquisition"
            );
        }
        // Same poisoned-gate policy as a full writer panic: permanently Err.
        assert!(lock.write().is_err());
    }

    /// TEST 8: the real production thread-local shard assignment spreads
    /// fresh threads across shards. Does not assume thread 0 == shard 0,
    /// since other tests in this binary may already have consumed tickets.
    #[test]
    fn thread_shard_distribution_stays_in_bounds_and_spreads() {
        let shards: Vec<usize> = (0..ENGINE_READ_SHARDS)
            .map(|_| std::thread::spawn(current_shard).join().unwrap())
            .collect();
        for &s in &shards {
            assert!(s < ENGINE_READ_SHARDS, "shard {s} out of bounds");
        }
        let distinct: std::collections::HashSet<usize> = shards.iter().copied().collect();
        assert!(
            distinct.len() > 1,
            "{} fresh threads all landed on the same shard: {shards:?}",
            ENGINE_READ_SHARDS
        );
    }

    /// TEST 9: writer progress under sustained reader pressure (the mirror
    /// of the existing ignored reader-progress-under-write-pressure
    /// diagnostic). Kept deterministic: the proof is the writer's call
    /// actually returning with the correct value, not elapsed time. The
    /// timeout exists only so a real liveness bug fails the test instead of
    /// hanging the suite.
    #[test]
    fn writer_makes_progress_under_sustained_reader_pressure() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let stop = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = *lock.read().unwrap();
                    }
                })
            })
            .collect();

        // Not a correctness dependency, just improves the odds the writer
        // actually meets live contention instead of an idle lock.
        for _ in 0..1000 {
            std::thread::yield_now();
        }

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        {
            let lock = lock.clone();
            std::thread::spawn(move || {
                let mut guard = lock.write().unwrap();
                *guard = 99;
                let _ = done_tx.send(());
            });
        }
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect(
                "writer failed to make progress under sustained reader pressure \
                 within the watchdog window",
            );

        stop.store(true, Ordering::Relaxed);
        for reader in readers {
            reader.join().unwrap();
        }
        assert_eq!(*lock.read().unwrap(), 99);
    }

    /// TEST 10: a HELD WRITE guard excludes fresh readers as well as fresh
    /// writers. The writer's guard is provably alive (it sits on a Barrier
    /// this thread must also pass), every contender announces its attempt
    /// before acquiring, and only after the writer's release do any of them
    /// enter.
    #[test]
    fn held_writer_excludes_readers_and_writers() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let attempting = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();

        let writer = {
            let lock = lock.clone();
            let held = held.clone();
            let release = release.clone();
            std::thread::spawn(move || {
                let mut guard = lock.write().unwrap();
                *guard = 5;
                held.wait();
                release.wait();
            })
        };
        held.wait();

        // Two readers and two writers, each signalling "attempting" before
        // its acquisition call so the exclusion window is a proof, not a
        // race on thread startup.
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let lock = lock.clone();
                let attempting = attempting.clone();
                let entered_tx = entered_tx.clone();
                std::thread::spawn(move || {
                    attempting.fetch_add(1, Ordering::SeqCst);
                    let guard = lock.read().unwrap();
                    entered_tx.send(()).unwrap();
                    drop(guard);
                })
            })
            .collect();
        let writers: Vec<_> = (0..2)
            .map(|_| {
                let lock = lock.clone();
                let attempting = attempting.clone();
                let entered_tx = entered_tx.clone();
                std::thread::spawn(move || {
                    attempting.fetch_add(1, Ordering::SeqCst);
                    let mut guard = lock.write().unwrap();
                    entered_tx.send(()).unwrap();
                    *guard += 1;
                })
            })
            .collect();

        while attempting.load(Ordering::SeqCst) < 4 {
            std::thread::yield_now();
        }
        assert!(
            entered_rx.try_recv().is_err(),
            "a contender entered the gate while the writer's guard was provably held"
        );

        release.wait();
        writer.join().unwrap();
        for _ in 0..4 {
            entered_rx.recv().unwrap();
        }
        for reader in readers {
            reader.join().unwrap();
        }
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(
            *lock.read().unwrap(),
            7,
            "5 was not preserved; expected the two queued writers' increments on top"
        );
    }

    /// TEST 11: three readers on distinct shards hold the gate at the same
    /// instant (Barrier-proven: all three guards are alive simultaneously)
    /// and all read the same value.
    #[test]
    fn three_readers_hold_gate_simultaneously() {
        let lock = Arc::new(ShardedRwLock::new(3usize));
        let held = Arc::new(Barrier::new(4));
        let release = Arc::new(Barrier::new(4));
        let readers: Vec<_> = [1usize, 4, 6]
            .map(|shard| {
                let lock = lock.clone();
                let held = held.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    let guard = lock.read_from_shard(shard).unwrap();
                    assert_eq!(*guard, 3);
                    held.wait();
                    release.wait();
                })
            })
            .into_iter()
            .collect();
        held.wait();
        release.wait();
        for reader in readers {
            reader.join().unwrap();
        }
        assert!(lock.write().is_ok());
    }

    /// TEST 12: three writers queue while two reader threads stream reads.
    /// At most one writer may hold the gate at a time; every queued writer
    /// eventually enters (join proves it); after the queue drains the gate
    /// is fully released and admits readers.
    #[test]
    fn three_queued_writers_progress_under_reader_stream() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        let writers_in = Arc::new(AtomicUsize::new(0));
        let max_writers = Arc::new(AtomicUsize::new(0));

        let start = Arc::new(Barrier::new(5));
        let reader_streams: Vec<_> = (0..2)
            .map(|_| {
                let lock = lock.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..250 {
                        let _ = *lock.read().unwrap();
                    }
                })
            })
            .collect();
        let writers: Vec<_> = (0..3)
            .map(|_| {
                let lock = lock.clone();
                let writers_in = writers_in.clone();
                let max_writers = max_writers.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    let mut guard = lock.write().unwrap();
                    let n = writers_in.fetch_add(1, Ordering::SeqCst) + 1;
                    max_writers.fetch_max(n, Ordering::SeqCst);
                    *guard += 1;
                    writers_in.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();

        for reader in reader_streams {
            reader.join().unwrap();
        }
        for writer in writers {
            writer.join().unwrap();
        }

        assert_eq!(max_writers.load(Ordering::SeqCst), 1);
        assert!(
            !lock.writer_pending(),
            "writer_pending stayed set after the writer queue drained"
        );
        assert_eq!(*lock.read().unwrap(), 3);
    }

    /// TEST 13: after each of a stream of writers finishes, fresh readers are
    /// immediately admissible and `writer_pending` is never observed stuck
    /// between writers.
    #[test]
    fn readers_admit_after_every_writer_flag_never_stuck() {
        let lock = Arc::new(ShardedRwLock::new(0usize));
        for i in 0..8 {
            let mut guard = lock.write().unwrap();
            *guard = i;
            drop(guard);
            assert!(
                !lock.writer_pending(),
                "writer_pending stuck after writer {i} released"
            );
            assert_eq!(
                *lock.read().unwrap(),
                i,
                "reader admitted right after writer {i}"
            );
        }

        let held = Arc::new(Barrier::new(9));
        let release = Arc::new(Barrier::new(9));
        let readers: Vec<_> = (0..8)
            .map(|shard| {
                let lock = lock.clone();
                let held = held.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    let guard = lock.read_from_shard(shard % ENGINE_READ_SHARDS).unwrap();
                    assert_eq!(*guard, 7);
                    held.wait();
                    release.wait();
                })
            })
            .collect();
        held.wait();
        release.wait();
        for reader in readers {
            reader.join().unwrap();
        }
        assert!(!lock.writer_pending());
        assert_eq!(*lock.read().unwrap(), 7);
    }

    /// TEST 14 (ignored, heavier): mixed stress with live exclusion
    /// invariants. Readers count admissions and assert no writer was inside
    /// the gate with them; writers assert no reader and no second writer was
    /// inside with them. Run with
    /// `cargo test --release -- --ignored --nocapture mixed_stress`.
    #[test]
    #[ignore]
    fn mixed_stress_exclusion_invariants_hold() {
        let lock = Arc::new(ShardedRwLock::new(0u64));
        let readers_in = Arc::new(AtomicUsize::new(0));
        let writers_in = Arc::new(AtomicUsize::new(0));
        let violations = Arc::new(AtomicUsize::new(0));

        let readers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                let readers_in = readers_in.clone();
                let writers_in = writers_in.clone();
                let violations = violations.clone();
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        let guard = lock.read().unwrap();
                        readers_in.fetch_add(1, Ordering::Relaxed);
                        if writers_in.load(Ordering::Relaxed) != 0 {
                            violations.fetch_add(1, Ordering::Relaxed);
                        }
                        readers_in.fetch_sub(1, Ordering::Relaxed);
                        drop(guard);
                    }
                })
            })
            .collect();
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                let readers_in = readers_in.clone();
                let writers_in = writers_in.clone();
                let violations = violations.clone();
                std::thread::spawn(move || {
                    for _ in 0..500 {
                        let mut guard = lock.write().unwrap();
                        let n = writers_in.fetch_add(1, Ordering::Relaxed) + 1;
                        if readers_in.load(Ordering::Relaxed) != 0 || n != 1 {
                            violations.fetch_add(1, Ordering::Relaxed);
                        }
                        *guard += 1;
                        writers_in.fetch_sub(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        for reader in readers {
            reader.join().unwrap();
        }
        for writer in writers {
            writer.join().unwrap();
        }

        assert_eq!(
            violations.load(Ordering::Relaxed),
            0,
            "exclusion violated: reader(s) and writer(s) held the gate together"
        );
        assert!(!lock.writer_pending());
        assert_eq!(*lock.read().unwrap(), 2_000);
    }

    /// Investigates whether writer priority (readers yield while
    /// `writer_pending` is set) starves readers under sustained write
    /// pressure. Ignored by default since it's a timing measurement, not a
    /// pass/fail check: run with `cargo test --release -- --ignored
    /// --nocapture reader_starvation_under_sustained_writes`.
    #[test]
    #[ignore]
    fn reader_starvation_under_sustained_writes() {
        use std::time::{Duration, Instant};

        let lock = Arc::new(ShardedRwLock::new(0u64));
        let stop = Arc::new(AtomicBool::new(false));
        let run_for = Duration::from_millis(500);

        let writers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut count = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        *lock.write().unwrap() += 1;
                        count += 1;
                    }
                    count
                })
            })
            .collect();

        let readers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut count = 0u64;
                    let mut max_gap = Duration::ZERO;
                    let mut last = Instant::now();
                    while !stop.load(Ordering::Relaxed) {
                        let _ = *lock.read().unwrap();
                        let now = Instant::now();
                        max_gap = max_gap.max(now.duration_since(last));
                        last = now;
                        count += 1;
                    }
                    (count, max_gap)
                })
            })
            .collect();

        std::thread::sleep(run_for);
        stop.store(true, Ordering::Relaxed);

        let write_counts: Vec<u64> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        let read_results: Vec<(u64, Duration)> =
            readers.into_iter().map(|r| r.join().unwrap()).collect();

        let total_writes: u64 = write_counts.iter().sum();
        let total_reads: u64 = read_results.iter().map(|(c, _)| c).sum();
        let worst_gap = read_results.iter().map(|(_, g)| *g).max().unwrap();

        eprintln!(
            "writes={total_writes} ({:.0}/s), reads={total_reads} ({:.0}/s), worst reader gap={worst_gap:?}",
            total_writes as f64 / run_for.as_secs_f64(),
            total_reads as f64 / run_for.as_secs_f64(),
        );

        assert!(
            total_reads > 0,
            "readers made zero progress under write pressure"
        );
    }
}

/// Exhaustive interleaving checks over the gate's synchronization core.
///
/// The unbounded reader yield-spin is not expressible as a bounded loom
/// search (loom itself rejects spin loops: "exceeded maximum number of
/// branches"), so the models below run a self-contained copy of the
/// protocol core — identical state, acquisition order, admission
/// linearization, and guard drop order — with the spin replaced by bounded
/// give-up semantics, which is equivalent for every safety property
/// (spinning only retries what a give-up would retry later).
///
/// Deliberately not modeled here: poisoning (loom locks never poison),
/// unwinding (loom does not model panics), and liveness (writer progress
/// under pressure). All three stay covered by the std unit tests above,
/// which exercise the production bytes directly.
#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use loom::sync::Arc;
    use loom::thread;

    /// Shard count for the loom model only — deliberately NOT
    /// `ENGINE_READ_SHARDS` (8). Shard count is load-balancing, never a
    /// safety property (see the module doc above and `current_shard`'s
    /// test), so shrinking it does not weaken anything the tests below
    /// check: 2 shards is still enough to exercise "a writer needs
    /// every shard" against "two readers on two different shards", and
    /// a full 8-way acquisition loop is the same code shape as a 2-way
    /// one. What 8 shards do cost is loom's search: each additional
    /// `RwLock` roughly multiplies the interleavings loom must
    /// enumerate for a writer's sequential acquire loop, and 8 shards
    /// across 2-3 threads was combinatorially intractable in practice
    /// (still running after 40+ CPU-minutes with no result). Even at 2
    /// shards, a fully exhaustive (unbounded) search of the two
    /// multi-thread models below did not finish in 20+ CPU-minutes
    /// either — the thread-interleaving space, not just the shard
    /// count, is the dominant cost for 3 concurrent threads each doing
    /// several lock operations. See `bounded_model` below for how
    /// these tests actually run.
    const MODEL_SHARDS: usize = 2;

    /// Runs `f` under a loom search bounded to `PREEMPTION_BOUND`
    /// context switches instead of `loom::model`'s fully exhaustive
    /// (unbounded) search. Even after shrinking `MODEL_SHARDS` to 2 (see
    /// above), unbounded exhaustive search of
    /// `loom_single_writer_excludes_concurrent_readers` (3 threads) and
    /// `loom_two_writers_never_share_the_gate` (2 threads, but each
    /// needing both shards) did not complete in 20+ CPU-minutes — the
    /// interleaving space from thread count alone is already large
    /// enough to dominate. A preemption bound is loom's own documented
    /// answer to exactly this: in practice almost every real
    /// concurrency bug is reachable within a small number of context
    /// switches, and bounding the search trades a small, well-understood
    /// gap in formal completeness for the search actually finishing.
    /// `PREEMPTION_BOUND = 3` reproduces the guard-lifetime bug this
    /// module's fix addresses well within budget (loom found it with
    /// the ORIGINAL unbounded search before this change existed) and
    /// completes in ~2 seconds; raise it (or unset it via a bespoke run
    /// with `loom::model` directly) for a deeper, slower search when
    /// specifically investigating this protocol.
    fn bounded_model<F>(f: F)
    where
        F: Fn() + Sync + Send + 'static,
    {
        const PREEMPTION_BOUND: usize = 3;
        let mut builder = loom::model::Builder::new();
        builder.preemption_bound = Some(PREEMPTION_BOUND);
        builder.check(f);
    }

    /// Model of the `ShardedRwLock` protocol core: same fields, same reader
    /// double-check admission, same writer serial → intent → ascending
    /// shard acquisition, same guard drop order (intent → serial → shards).
    /// MUST be kept in sync with `ShardedRwLock` above (modulo shard count,
    /// see `MODEL_SHARDS`); it exists so loom can exhaustively explore the
    /// protocol without the unbounded spin.
    struct Gate {
        shards: [RwLock<()>; MODEL_SHARDS],
        writer_serial: Mutex<()>,
        writer_pending: AtomicBool,
        value: UnsafeCell<u64>,
    }

    // SAFETY: mirror of the `ShardedRwLock` impls — shard read guards gate
    // shared access, the writer's serial + full shard set gate exclusive
    // access, and the model moves its owned state as one unit.
    unsafe impl Send for Gate {}
    unsafe impl Sync for Gate {}

    struct GateRead<'a> {
        _shard: RwLockReadGuard<'a, ()>,
        value: *const u64,
    }
    impl Deref for GateRead<'_> {
        type Target = u64;
        fn deref(&self) -> &u64 {
            // SAFETY: same argument as `ReadGuard::deref` — this guard owns a
            // shard read lock and no `GateWrite` can exist without every shard.
            unsafe { &*self.value }
        }
    }

    struct GateIntent<'a>(&'a AtomicBool);
    impl Drop for GateIntent<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }

    struct GateWrite<'a> {
        _intent: GateIntent<'a>,
        _serial: MutexGuard<'a, ()>,
        _shards: [RwLockWriteGuard<'a, ()>; MODEL_SHARDS],
        value: *mut u64,
    }
    impl Deref for GateWrite<'_> {
        type Target = u64;
        fn deref(&self) -> &u64 {
            // SAFETY: same argument as `WriteGuard::deref` — serial plus every
            // shard write lock are still live while `self` is borrowed.
            unsafe { &*self.value }
        }
    }
    impl DerefMut for GateWrite<'_> {
        fn deref_mut(&mut self) -> &mut u64 {
            // SAFETY: same argument as `WriteGuard::deref_mut` — serial plus
            // every shard write lock are still live while `self` is borrowed.
            unsafe { &mut *self.value }
        }
    }

    impl Gate {
        fn new() -> Self {
            Self {
                shards: std::array::from_fn(|_| RwLock::new(())),
                writer_serial: Mutex::new(()),
                writer_pending: AtomicBool::new(false),
                value: UnsafeCell::new(0),
            }
        }

        /// Reader admission: linearized at the second `writer_pending`
        /// check, exactly as in `read_from_shard`. A give-up (`None`) is
        /// what one spin iteration with an unmet shard lock would do.
        fn read(&self, shard: usize) -> Option<GateRead<'_>> {
            if self.writer_pending.load(Ordering::Acquire) {
                return None;
            }
            let guard = self.shards[shard].read().map_err(|_| ()).ok()?;
            if !self.writer_pending.load(Ordering::Acquire) {
                return Some(GateRead {
                    _shard: guard,
                    value: self.value.get(),
                });
            }
            None
        }

        fn write(&self) -> Option<GateWrite<'_>> {
            let serial = self.writer_serial.lock().map_err(|_| ()).ok()?;
            self.writer_pending.store(true, Ordering::Release);
            let intent = GateIntent(&self.writer_pending);
            let mut guards = Vec::with_capacity(MODEL_SHARDS);
            for shard in &self.shards {
                guards.push(shard.write().map_err(|_| ()).ok()?);
            }
            let shards: [RwLockWriteGuard<'_, ()>; MODEL_SHARDS] = guards.try_into().ok()?;
            Some(GateWrite {
                _intent: intent,
                _serial: serial,
                _shards: shards,
                value: self.value.get(),
            })
        }

        fn pending(&self) -> bool {
            self.writer_pending.load(Ordering::Acquire)
        }
    }

    /// Invariant harness: each critical-section entry increments its side's
    /// counter BEFORE checking the other side's, so if reader and writer (or
    /// two writers) could ever coexist, at least one check would observe the
    /// overlap and the assert fires — and loom explores every interleaving.
    #[test]
    fn loom_single_writer_excludes_concurrent_readers() {
        bounded_model(|| {
            let gate = Arc::new(Gate::new());
            let readers_in = Arc::new(AtomicUsize::new(0));
            let writers_in = Arc::new(AtomicUsize::new(0));
            let mut joins = Vec::new();
            for shard in 0..2usize {
                let (gate, readers_in, writers_in) =
                    (gate.clone(), readers_in.clone(), writers_in.clone());
                joins.push(thread::spawn(move || {
                    if let Some(guard) = gate.read(shard) {
                        readers_in.fetch_add(1, Ordering::Relaxed);
                        assert!(
                            writers_in.load(Ordering::Relaxed) == 0,
                            "reader and writer held the gate together"
                        );
                        readers_in.fetch_sub(1, Ordering::Relaxed);
                        drop(guard);
                    }
                }));
            }
            joins.push(thread::spawn({
                let (gate, readers_in, writers_in) =
                    (gate.clone(), readers_in.clone(), writers_in.clone());
                move || {
                    // Bind the guard (not `.is_some()` on the temporary):
                    // an unbound `if gate.write().is_some() { .. }` drops
                    // the `Option<GateWrite>` at the end of the condition,
                    // before the body runs, releasing every shard lock
                    // ahead of the counter check below and making the
                    // assertion race for real. Binding keeps the guard
                    // alive for the whole body, matching the real
                    // exclusion this test exists to prove.
                    if let Some(_guard) = gate.write() {
                        writers_in.fetch_add(1, Ordering::Relaxed);
                        assert!(
                            readers_in.load(Ordering::Relaxed) == 0,
                            "reader and writer held the gate together"
                        );
                        writers_in.fetch_sub(1, Ordering::Relaxed);
                    }
                }
            }));
            for join in joins {
                join.join().unwrap();
            }
            assert!(!gate.pending(), "writer intent lost or stuck");
        });
    }

    /// Two writers serialized by `writer_serial`: never both inside, intent
    /// never lost, and both eventually enter (join proves it).
    #[test]
    fn loom_two_writers_never_share_the_gate() {
        bounded_model(|| {
            let gate = Arc::new(Gate::new());
            let writers_in = Arc::new(AtomicUsize::new(0));
            let mut joins = Vec::new();
            for _ in 0..2 {
                let (gate, writers_in) = (gate.clone(), writers_in.clone());
                joins.push(thread::spawn(move || {
                    // Same bound-guard requirement as above: an unbound
                    // `.is_some()` releases the guard before this body
                    // runs, defeating the very exclusion under test.
                    if let Some(_guard) = gate.write() {
                        let n = writers_in.fetch_add(1, Ordering::Relaxed) + 1;
                        assert!(n == 1, "two writers held the gate together");
                        writers_in.fetch_sub(1, Ordering::Relaxed);
                    }
                }));
            }
            for join in joins {
                join.join().unwrap();
            }
            assert!(!gate.pending(), "writer intent lost or stuck");
        });
    }

    /// Value handoff: a reader that observes the writer's completion must
    /// observe the written value (the intent flag's Release store pairs
    /// with the reader's Acquire load), and after all threads drain, the
    /// final read guard sees the write through the shard-lock handoff.
    #[test]
    fn loom_write_handoff_is_ordered_and_visible() {
        bounded_model(|| {
            let gate = Arc::new(Gate::new());
            let writer_done = Arc::new(AtomicBool::new(false));
            let reader = {
                let (gate, writer_done) = (gate.clone(), writer_done.clone());
                thread::spawn(move || {
                    if let Some(guard) = gate.read(0)
                        && writer_done.load(Ordering::Acquire)
                    {
                        assert_eq!(*guard, 1, "reader observed completion but not the write");
                    }
                })
            };
            let writer = {
                let (gate, writer_done) = (gate.clone(), writer_done.clone());
                thread::spawn(move || {
                    if let Some(mut guard) = gate.write() {
                        *guard = 1;
                        writer_done.store(true, Ordering::Release);
                    }
                })
            };
            reader.join().unwrap();
            writer.join().unwrap();
            assert!(writer_done.load(Ordering::Acquire));
            assert!(!gate.pending(), "writer intent lost or stuck");
            assert_eq!(
                *gate
                    .read(1)
                    .unwrap_or_else(|| panic!("gate refused the final read")),
                1
            );
        });
    }
}
