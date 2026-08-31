//! Byte-budgeted cache of verified sstable blocks.
//!
//! Keys are `(file number, block offset)`. Hits use a shared lock, clone
//! immutable `Arc`-backed blocks, and set a relaxed CLOCK reference bit.
//! Insertion and second-chance eviction own the write lock; capacity is a
//! hard global byte bound.

use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

#[derive(Debug)]
pub struct BlockCache {
    inner: RwLock<Inner>,
    capacity: usize,
    /// Observational counters; they do not publish correctness state, so
    /// relaxed atomics avoid turning successful hits into write-lock work.
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

/// Raw counters for a [`BlockCache`] (phase 11.7) — facts only, no
/// derived rates or verdicts. A caller wanting a hit rate computes
/// `hits / (hits + misses)` itself.
#[derive(Debug, Clone, Copy)]
pub struct BlockCacheStats {
    pub capacity_bytes: usize,
    pub resident_bytes: usize,
    pub resident_entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
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
}
