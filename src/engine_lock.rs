use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(crate) const ENGINE_READ_SHARDS: usize = 8;
static NEXT_SHARD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static READER_SHARD: usize = NEXT_SHARD.fetch_add(1, Ordering::Relaxed) % ENGINE_READ_SHARDS;
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

// SAFETY: moving the lock moves its owned T and synchronization state together.
unsafe impl<T: Send> Send for ShardedRwLock<T> {}
// SAFETY: shard locks enforce shared/exclusive access; shared readers need T: Sync.
unsafe impl<T: Send + Sync> Sync for ShardedRwLock<T> {}

pub(crate) struct ReadGuard<'a, T> {
    _shard: RwLockReadGuard<'a, ()>,
    value: *const T,
    _marker: PhantomData<&'a T>,
}
impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.value }
    }
}

struct WriterIntent<'a>(&'a AtomicBool);
impl Drop for WriterIntent<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

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
        unsafe { &*self.value }
    }
}
impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
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
                std::thread::yield_now();
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
            std::thread::yield_now();
        }
    }

    #[cfg(test)]
    fn writer_pending(&self) -> bool {
        self.writer_pending.load(Ordering::Acquire)
    }
    pub(crate) fn write(&self) -> Result<WriteGuard<'_, T>, ()> {
        let serial = self.writer_serial.lock().map_err(|_| ())?;
        self.writer_pending.store(true, Ordering::Release);
        let intent = WriterIntent(&self.writer_pending);
        let mut guards = Vec::with_capacity(ENGINE_READ_SHARDS);
        for shard in &self.shards {
            guards.push(shard.write().map_err(|_| ())?);
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

        assert!(total_reads > 0, "readers made zero progress under write pressure");
    }
}
