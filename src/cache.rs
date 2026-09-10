//! Byte-budgeted cache of verified sstable blocks.
//!
//! Keys are `(file number, block offset)`. Hits use a shared lock, clone
//! immutable `Arc`-backed blocks, and set a relaxed CLOCK reference bit.
//! Insertion and second-chance eviction own the write lock; capacity is a
//! hard global byte bound.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};

/// Parsed layout of a verified data block, relative to its raw bytes.
#[derive(Debug, Clone, Copy)]
pub struct BlockMeta {
    pub entries_end: usize,
    pub restart_start: usize,
    pub num_restarts: usize,
}

#[derive(Debug, Clone)]
pub struct CachedBlock {
    pub data: ArcBlock,
    pub meta: BlockMeta,
}

pub type ArcBlock = std::sync::Arc<[u8]>;

type CacheKey = (u64, u64);

#[derive(Debug)]
struct Entry {
    block: CachedBlock,
    /// Eviction-policy metadata only. Relaxed is sufficient: map
    /// membership and block ownership remain protected by `inner`.
    referenced: AtomicBool,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<CacheKey, Entry>,
    clock: VecDeque<CacheKey>,
    bytes: usize,
}

/// State of one physical load in flight for a key, shared by every
/// caller that arrives while it is running.
#[derive(Debug)]
enum Outcome {
    /// The leader is still running `load`.
    Pending,
    /// The leader finished (successfully or not) and published a result.
    Done(Result<CachedBlock, String>),
    /// The leader's `load` panicked before publishing anything — never
    /// set by the leader's own code, only by `RemoveOnDrop::drop` when
    /// it finds the slot still `Pending` during an unwind. A waiter
    /// that observes this must retry as a fresh leader, not wait
    /// forever: nothing will ever move this slot out of `Abandoned`.
    Abandoned,
}

/// One physical load in flight for a key. `outcome` starts `Pending`;
/// the leader (the caller that created this slot) resolves it and
/// notifies exactly once, whether it succeeds, returns an error, or
/// panics (via `RemoveOnDrop` in `get_or_load`, which fires on unwind
/// too) — so a waiter's wait loop always has a real state to act on
/// and can never block forever.
#[derive(Debug)]
struct InFlight {
    outcome: Mutex<Outcome>,
    condvar: Condvar,
}

#[derive(Debug)]
pub struct BlockCache {
    inner: RwLock<Inner>,
    capacity: usize,
    /// Observational counters; they do not publish correctness state, so
    /// relaxed atomics avoid turning successful hits into write-lock work.
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    /// Single-flight state for `get_or_load` (11.17-F): one entry per
    /// key with a physical load currently running. Misses that don't
    /// collide with a concurrent load for the same key never touch
    /// this map's lock at all beyond the one check-and-maybe-insert.
    in_flight: Mutex<HashMap<CacheKey, Arc<InFlight>>>,
    /// Count of `get_or_load` calls that found a load for their key
    /// already in flight and reused its result instead of issuing a
    /// second physical read — the duplicate I/O `get_or_load` exists to
    /// avoid.
    coalesced: AtomicU64,
}

/// Raw counters for a [`BlockCache`] (phase 11.7, extended 11.17-F) —
/// facts only, no derived rates or verdicts. A caller wanting a hit
/// rate computes `hits / (hits + misses)` itself.
#[derive(Debug, Clone, Copy)]
pub struct BlockCacheStats {
    pub capacity_bytes: usize,
    pub resident_bytes: usize,
    pub resident_entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Concurrent misses on the same key that waited for another
    /// caller's already-running load instead of duplicating it.
    pub coalesced: u64,
}

impl BlockCache {
    pub fn new(capacity_bytes: usize) -> BlockCache {
        BlockCache {
            inner: RwLock::new(Inner {
                map: HashMap::new(),
                clock: VecDeque::new(),
                bytes: 0,
            }),
            capacity: capacity_bytes,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            in_flight: Mutex::new(HashMap::new()),
            coalesced: AtomicU64::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn resident_bytes(&self) -> usize {
        self.inner.read().expect("block cache lock poisoned").bytes
    }

    pub fn resident_entries(&self) -> usize {
        self.inner
            .read()
            .expect("block cache lock poisoned")
            .map
            .len()
    }

    /// Side-effect-free cache observation. The read lock only protects
    /// membership and resident byte accounting; counters are relaxed
    /// observations and do not change cache policy.
    pub fn stats(&self) -> BlockCacheStats {
        let inner = self.inner.read().expect("block cache lock poisoned");
        BlockCacheStats {
            capacity_bytes: self.capacity,
            resident_bytes: inner.bytes,
            resident_entries: inner.map.len(),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
        }
    }

    /// A cache hit takes only a shared lock. The relaxed reference-bit
    /// store affects future CLOCK eviction only; it never publishes block
    /// contents or map membership.
    pub fn get(&self, key: &CacheKey) -> Option<CachedBlock> {
        let inner = self.inner.read().expect("block cache lock poisoned");
        let Some(entry) = inner.map.get(key) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let block = entry.block.clone();
        entry.referenced.store(true, Ordering::Relaxed);
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(block)
    }

    /// Inserts a verified block. Oversized blocks bypass admission without
    /// disturbing resident entries. CLOCK work is deliberately insertion
    /// work, never hit-path work.
    pub fn insert(&self, key: CacheKey, block: CachedBlock) {
        let bytes = block.data.len();
        if bytes > self.capacity {
            return;
        }
        let mut inner = self.inner.write().expect("block cache lock poisoned");
        if let Some(entry) = inner.map.get(&key) {
            entry.referenced.store(true, Ordering::Relaxed);
            return;
        }
        while inner.bytes + bytes > self.capacity {
            let victim = inner
                .clock
                .pop_front()
                .expect("clock tracks every resident entry");
            let referenced = inner
                .map
                .get(&victim)
                .expect("clock key must be resident")
                .referenced
                .swap(false, Ordering::Relaxed);
            if referenced {
                inner.clock.push_back(victim);
                continue;
            }
            let removed = inner
                .map
                .remove(&victim)
                .expect("clock key must be resident");
            inner.bytes -= removed.block.data.len();
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
        inner.bytes += bytes;
        inner.clock.push_back(key);
        inner.map.insert(
            key,
            Entry {
                block,
                referenced: AtomicBool::new(true),
            },
        );
        debug_assert!(inner.bytes <= self.capacity);
    }

    /// Loads `key` via `load` on a miss, coalescing concurrent misses
    /// for the SAME key into one physical load (11.17-F): the first
    /// caller to miss becomes the leader and runs `load`; any other
    /// caller that misses on the same key while the leader is still
    /// working waits for the leader's result — cached or not — instead
    /// of repeating a possibly disk-bound read. A hit (the common case)
    /// never touches the single-flight machinery at all.
    ///
    /// `load`'s error is stringified for waiters (`E` need not be
    /// `Clone`): this mirrors how callers already format table-read
    /// errors into their own error type, so nothing is lost in
    /// practice. If the leader panics, `RemoveOnDrop` below still runs
    /// during unwind and marks the slot `Abandoned` before notifying —
    /// waiters wake, see they'll never get a result from this slot, and
    /// retry as a fresh leader rather than waiting on a condvar nothing
    /// will ever signal again.
    pub fn get_or_load<E, F>(&self, key: CacheKey, load: F) -> Result<CachedBlock, String>
    where
        F: FnOnce() -> Result<CachedBlock, E>,
        E: std::fmt::Display,
    {
        loop {
            if let Some(hit) = self.get(&key) {
                return Ok(hit);
            }

            enum Role {
                Leader(Arc<InFlight>),
                Waiter(Arc<InFlight>),
            }
            let role = {
                let mut in_flight = self
                    .in_flight
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Some(slot) = in_flight.get(&key) {
                    Role::Waiter(slot.clone())
                } else {
                    let slot = Arc::new(InFlight {
                        outcome: Mutex::new(Outcome::Pending),
                        condvar: Condvar::new(),
                    });
                    in_flight.insert(key, slot.clone());
                    Role::Leader(slot)
                }
            };

            match role {
                Role::Waiter(slot) => {
                    self.coalesced.fetch_add(1, Ordering::Relaxed);
                    let mut outcome = slot.outcome.lock().unwrap_or_else(PoisonError::into_inner);
                    loop {
                        match &*outcome {
                            Outcome::Pending => {
                                outcome = slot
                                    .condvar
                                    .wait(outcome)
                                    .unwrap_or_else(PoisonError::into_inner);
                            }
                            Outcome::Done(result) => return result.clone(),
                            Outcome::Abandoned => break,
                        }
                    }
                    // The leader panicked before publishing: its
                    // RemoveOnDrop guard already dropped the map entry.
                    // Retry from the top — either the cache now has it
                    // (another retrying waiter already won leadership
                    // and finished) or we become the new leader.
                    continue;
                }
                Role::Leader(slot) => {
                    struct RemoveOnDrop<'a> {
                        cache: &'a BlockCache,
                        key: CacheKey,
                        slot: &'a InFlight,
                    }
                    impl Drop for RemoveOnDrop<'_> {
                        fn drop(&mut self) {
                            self.cache
                                .in_flight
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&self.key);
                            // On the normal path `outcome` is already
                            // `Done` by the time this guard drops. On a
                            // panic it is still `Pending` — mark it
                            // `Abandoned` so waiters know to stop
                            // waiting on this slot instead of blocking
                            // on a notify that will never come again.
                            let mut outcome = self
                                .slot
                                .outcome
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner);
                            if matches!(*outcome, Outcome::Pending) {
                                *outcome = Outcome::Abandoned;
                            }
                            drop(outcome);
                            self.slot.condvar.notify_all();
                        }
                    }
                    let _remove = RemoveOnDrop {
                        cache: self,
                        key,
                        slot: &slot,
                    };
                    let result = load().map_err(|e| e.to_string());
                    if let Ok(block) = &result {
                        self.insert(key, block.clone());
                    }
                    *slot.outcome.lock().unwrap_or_else(PoisonError::into_inner) =
                        Outcome::Done(result.clone());
                    return result;
                }
            }
        }
    }

    #[cfg(test)]
    fn in_flight_len(&self) -> usize {
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    #[cfg(test)]
    fn clock_entries(&self) -> usize {
        self.inner
            .read()
            .expect("block cache lock poisoned")
            .clock
            .len()
    }

    #[cfg(test)]
    fn clear_reference_bits(&self) {
        let inner = self.inner.read().expect("block cache lock poisoned");
        for entry in inner.map.values() {
            entry.referenced.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn block(size: usize, fill: u8) -> CachedBlock {
        CachedBlock {
            data: vec![fill; size].into(),
            meta: BlockMeta {
                entries_end: size,
                restart_start: 0,
                num_restarts: 1,
            },
        }
    }

    #[test]
    fn basic_hit_and_miss_track_exactly() {
        let cache = BlockCache::new(300);
        cache.insert((1, 0), block(100, 1));
        let hit = cache.get(&(1, 0)).unwrap();
        assert_eq!(hit.data.as_ref(), &[1; 100]);
        assert!(cache.get(&(9, 9)).is_none());
        let stats = cache.stats();
        assert_eq!((stats.hits, stats.misses, stats.evictions), (1, 1, 0));
        assert_eq!(stats.resident_bytes, 100);
    }

    #[test]
    fn clock_gives_touched_entry_second_chance() {
        let cache = BlockCache::new(300);
        for i in 1..=3 {
            cache.insert((i, 0), block(100, i as u8));
        }
        cache.clear_reference_bits();
        cache.get(&(1, 0)).unwrap();
        cache.insert((4, 0), block(100, 4));
        assert!(cache.get(&(1, 0)).is_some());
        assert!(cache.get(&(2, 0)).is_none());
        assert!(cache.get(&(4, 0)).is_some());
    }

    #[test]
    fn referenced_entry_is_eventually_evictable() {
        let cache = BlockCache::new(200);
        cache.insert((1, 0), block(100, 1));
        cache.insert((2, 0), block(100, 2));
        cache.get(&(1, 0)).unwrap();
        cache.insert((3, 0), block(100, 3));
        cache.insert((4, 0), block(100, 4));
        assert!(cache.get(&(1, 0)).is_none());
    }

    #[test]
    fn capacity_and_oversize_bypass_hold_under_pressure() {
        let cache = BlockCache::new(257);
        cache.insert((0, 0), block(100, 0));
        let before = cache.stats();
        cache.insert((9, 0), block(300, 9));
        assert_eq!(cache.stats().evictions, before.evictions);
        for i in 1..50 {
            cache.insert((i, 0), block(37 + (i as usize % 11), i as u8));
            assert!(cache.resident_bytes() <= 257);
        }
    }

    #[test]
    fn duplicate_insert_has_one_clock_slot() {
        let cache = BlockCache::new(300);
        cache.insert((1, 0), block(100, 1));
        cache.insert((1, 0), block(150, 2));
        assert_eq!(cache.resident_entries(), 1);
        assert_eq!(cache.clock_entries(), 1);
        assert_eq!(cache.resident_bytes(), 100);
    }

    #[test]
    fn evicted_entry_clone_remains_usable() {
        let cache = BlockCache::new(100);
        cache.insert((1, 0), block(100, 7));
        let live = cache.get(&(1, 0)).unwrap();
        cache.insert((2, 0), block(100, 8));
        assert!(cache.get(&(1, 0)).is_none());
        assert_eq!(live.data.as_ref(), &[7; 100]);
    }

    #[test]
    fn concurrent_same_key_hits_are_correct() {
        let cache = Arc::new(BlockCache::new(100));
        cache.insert((1, 0), block(100, 3));
        let threads = 8;
        let rounds = 5_000;
        let start = Arc::new(Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let cache = cache.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..rounds {
                        assert_eq!(cache.get(&(1, 0)).unwrap().data[0], 3);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(cache.stats().hits, (threads * rounds) as u64);
        assert_eq!(cache.resident_bytes(), 100);
    }

    #[test]
    fn concurrent_scattered_hits_and_eviction_stay_consistent() {
        let cache = Arc::new(BlockCache::new(400));
        for i in 0..4 {
            cache.insert((i, 0), block(100, i as u8));
        }
        let start = Arc::new(Barrier::new(5));
        let mut handles = Vec::new();
        for thread in 0..4 {
            let cache = cache.clone();
            let start = start.clone();
            handles.push(std::thread::spawn(move || {
                start.wait();
                for i in 0..2_000 {
                    let _ = cache.get(&((i + thread) as u64 % 4, 0));
                }
            }));
        }
        let writer = cache.clone();
        let writer_start = start.clone();
        handles.push(std::thread::spawn(move || {
            writer_start.wait();
            for i in 4..200 {
                writer.insert((i, 0), block(100, i as u8));
                assert!(writer.resident_bytes() <= 400);
            }
        }));
        for handle in handles {
            handle.join().unwrap();
        }
        assert!(cache.resident_bytes() <= 400);
        assert_eq!(cache.clock_entries(), cache.resident_entries());
    }

    #[test]
    fn same_key_insert_race_keeps_one_entry() {
        let cache = Arc::new(BlockCache::new(300));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                std::thread::spawn(move || cache.insert((1, 0), block(100, 1)))
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(cache.resident_entries(), 1);
        assert_eq!(cache.clock_entries(), 1);
    }

    /// 11.17-F: N concurrent misses on the SAME key must trigger exactly
    /// one physical load — every caller still gets the right block.
    #[test]
    fn concurrent_misses_on_same_key_coalesce_to_one_load() {
        let cache = Arc::new(BlockCache::new(1024));
        let load_calls = Arc::new(AtomicU64::new(0));
        let readers_ready = Arc::new(Barrier::new(9));
        let release = Arc::new(Barrier::new(9));
        // Every thread races into `get_or_load` for the same key; the
        // barriers just maximize the odds they actually overlap instead
        // of proving anything themselves — the assertion below is what
        // actually proves single-flight, not timing.
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let load_calls = load_calls.clone();
                let readers_ready = readers_ready.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    readers_ready.wait();
                    release.wait();
                    cache
                        .get_or_load((1, 0), || {
                            load_calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            Ok::<CachedBlock, String>(block(64, 7))
                        })
                        .unwrap()
                })
            })
            .collect();
        readers_ready.wait();
        release.wait();
        for h in handles {
            assert_eq!(h.join().unwrap().data.as_ref(), &[7u8; 64]);
        }
        assert_eq!(
            load_calls.load(Ordering::SeqCst),
            1,
            "8 concurrent misses on the same key issued more than one physical load"
        );
        assert!(cache.stats().coalesced >= 1);
        assert_eq!(
            cache.in_flight_len(),
            0,
            "in-flight slot must be cleaned up"
        );
    }

    /// Non-colliding misses (distinct keys) must not serialize on each
    /// other or on the single-flight map beyond its own brief lock.
    #[test]
    fn non_colliding_misses_do_not_coalesce_or_block_each_other() {
        let cache = Arc::new(BlockCache::new(4096));
        let load_calls = Arc::new(AtomicU64::new(0));
        let handles: Vec<_> = (0..8u64)
            .map(|i| {
                let cache = cache.clone();
                let load_calls = load_calls.clone();
                std::thread::spawn(move || {
                    cache
                        .get_or_load((1, i), || {
                            load_calls.fetch_add(1, Ordering::SeqCst);
                            Ok::<CachedBlock, String>(block(64, i as u8))
                        })
                        .unwrap()
                })
            })
            .collect();
        for (i, h) in handles.into_iter().enumerate() {
            assert_eq!(h.join().unwrap().data.as_ref(), &[i as u8; 64]);
        }
        assert_eq!(load_calls.load(Ordering::SeqCst), 8);
        assert_eq!(cache.stats().coalesced, 0);
        assert_eq!(cache.in_flight_len(), 0);
    }

    /// A leader whose `load` panics must not strand its waiters forever:
    /// they wake, see no published result, and retry as fresh leaders
    /// until one of them actually succeeds.
    #[test]
    fn panicking_leader_does_not_strand_waiters() {
        let cache = Arc::new(BlockCache::new(1024));
        let attempt = Arc::new(AtomicU64::new(0));
        let ready = Arc::new(Barrier::new(5));
        let release = Arc::new(Barrier::new(5));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let cache = cache.clone();
                let attempt = attempt.clone();
                let ready = ready.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    ready.wait();
                    release.wait();
                    cache.get_or_load((1, 0), || {
                        // The first attempt across all racing threads
                        // panics (simulating a leader that dies mid-load);
                        // every later attempt (a retrying waiter or a
                        // fresh miss) succeeds.
                        if attempt.fetch_add(1, Ordering::SeqCst) == 0 {
                            panic!("simulated leader failure");
                        }
                        Ok::<CachedBlock, String>(block(64, 9))
                    })
                })
            })
            .collect();
        ready.wait();
        release.wait();
        let mut ok = 0;
        let mut panicked = 0;
        for h in handles {
            match h.join() {
                Ok(Ok(b)) => {
                    assert_eq!(b.data.as_ref(), &[9u8; 64]);
                    ok += 1;
                }
                Err(_) => panicked += 1,
                Ok(Err(e)) => panic!("unexpected load error: {e}"),
            }
        }
        assert!(ok >= 1, "no caller ever got a successful result");
        assert!(panicked >= 1, "the simulated panic never actually fired");
        assert_eq!(
            cache.in_flight_len(),
            0,
            "in-flight slot must not leak after a panic"
        );
    }
}
