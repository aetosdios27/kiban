use std::sync::atomic::{AtomicUsize, Ordering};

use std::cell::UnsafeCell;
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
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for ShardedRwLock<T> {}
unsafe impl<T: Send + Sync> Sync for ShardedRwLock<T> {}

pub(crate) struct ReadGuard<'a, T> {
    _shard: RwLockReadGuard<'a, ()>,
    value: &'a T,
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}

pub(crate) struct WriteGuard<'a, T> {
    _serial: MutexGuard<'a, ()>,
    _shards: [RwLockWriteGuard<'a, ()>; ENGINE_READ_SHARDS],
    value: &'a mut T,
}

impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}
impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<T> ShardedRwLock<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            shards: std::array::from_fn(|_| RwLock::new(())),
            writer_serial: Mutex::new(()),
            value: UnsafeCell::new(value),
        }
    }
    pub(crate) fn read(&self) -> Result<ReadGuard<'_, T>, ()> {
        let shard = READER_SHARD.with(|slot| *slot);
        let guard = self.shards[shard].read().map_err(|_| ())?;
        // SAFETY: `guard` prevents any writer from holding every shard.
        let value = unsafe { &*self.value.get() };
        Ok(ReadGuard {
            _shard: guard,
            value,
        })
    }
    pub(crate) fn write(&self) -> Result<WriteGuard<'_, T>, ()> {
        let serial = self.writer_serial.lock().map_err(|_| ())?;
        let mut guards = Vec::with_capacity(ENGINE_READ_SHARDS);
        for shard in &self.shards {
            guards.push(shard.write().map_err(|_| ())?);
        }
        let shards: [RwLockWriteGuard<'_, ()>; ENGINE_READ_SHARDS] =
            guards.try_into().map_err(|_| ())?;
        // SAFETY: all shard write guards exclude every read and write guard.
        let value = unsafe { &mut *self.value.get() };
        Ok(WriteGuard {
            _serial: serial,
            _shards: shards,
            value,
        })
    }
}
