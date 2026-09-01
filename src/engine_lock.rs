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

unsafe impl<T: Send> Send for ShardedRwLock<T> {}
unsafe impl<T: Send + Sync> Sync for ShardedRwLock<T> {}

pub(crate) struct ReadGuard<'a, T> {
    _shard: RwLockReadGuard<'a, ()>,
    value: *const T,
    _marker: PhantomData<&'a T>,
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: `_shard` remains live for this guard's lifetime.
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
        // SAFETY: every shard write guard remains live.
        unsafe { &*self.value }
    }
}
impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: every shard write guard remains live and writers serialize.
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
        let shard = READER_SHARD.with(|slot| *slot);
        loop {
            if self.writer_pending.load(Ordering::Acquire) {
                std::thread::yield_now();
                continue;
            }
            let guard = self.shards[shard].read().map_err(|_| ())?;
            if !self.writer_pending.load(Ordering::Acquire) {
                // SAFETY: `guard` prevents any writer from holding every shard.
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
        // SAFETY: all shard write guards exclude every read and write guard.
        Ok(WriteGuard {
            _intent: intent,
            _serial: serial,
            _shards: shards,
            value: self.value.get(),
            _marker: PhantomData,
        })
    }
}
