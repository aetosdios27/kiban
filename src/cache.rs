//! Byte-budgeted LRU cache of verified sstable blocks.
//!
//! Per `docs/design/block-cache.md` D2: keyed by (file number, block
//! offset), storing raw block bytes plus their parsed layout, CRC
//! verified once on insertion. Hits clone an `Arc`; eviction is strict
//! LRU under the cache mutex.

use std::collections::HashMap;
use std::sync::Mutex;

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

#[derive(Debug)]
struct Entry {
    block: CachedBlock,
    bytes: usize,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<(u64, u64), Entry>,
    order: std::collections::VecDeque<(u64, u64)>,
    bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
}

#[derive(Debug)]
pub struct BlockCache {
    inner: Mutex<Inner>,
    capacity: usize,
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
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: std::collections::VecDeque::new(),
                bytes: 0,
                hits: 0,
                misses: 0,
                evictions: 0,
            }),
            capacity: capacity_bytes,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn resident_bytes(&self) -> usize {
        self.inner.lock().unwrap().bytes
    }

    pub fn resident_entries(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }

    /// A cheap, lock-once read of every counter (phase 11.7). Reading
    /// stats never touches an entry, so it cannot itself cause a hit,
    /// miss, or eviction.
    pub fn stats(&self) -> BlockCacheStats {
        let inner = self.inner.lock().unwrap();
        BlockCacheStats {
            capacity_bytes: self.capacity,
            resident_bytes: inner.bytes,
            resident_entries: inner.map.len(),
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
        }
    }

    /// Promotes on hit.
    pub fn get(&self, key: &(u64, u64)) -> Option<CachedBlock> {
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.map.get_mut(key) else {
            inner.misses = inner.misses.saturating_add(1);
            return None;
        };
        let block = entry.block.clone();
        Self::touch(&mut inner.order, *key);
        inner.hits = inner.hits.saturating_add(1);
        Some(block)
    }

    /// Inserts a verified block. Blocks larger than the whole budget are
    /// not admitted (the caller already holds the data it needs) — not
    /// an eviction, since nothing resident was removed to make room.
    pub fn insert(&self, key: (u64, u64), block: CachedBlock) {
        let bytes = block.data.len();
        if bytes > self.capacity {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.map.contains_key(&key) {
            Self::touch(&mut inner.order, key);
            return;
        }
        while inner.bytes + bytes > self.capacity {
            let Some(victim) = inner.order.pop_front() else {
                break;
            };
            if let Some(removed) = inner.map.remove(&victim) {
                inner.bytes -= removed.bytes;
                inner.evictions = inner.evictions.saturating_add(1);
            }
        }
        inner.order.push_back(key);
        inner.bytes += bytes;
        inner.map.insert(key, Entry { block, bytes });
    }

    fn touch(order: &mut std::collections::VecDeque<(u64, u64)>, key: (u64, u64)) {
        if let Some(pos) = order.iter().position(|k| *k == key) {
            order.remove(pos);
        }
        order.push_back(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn block(size: usize) -> CachedBlock {
        CachedBlock {
            data: vec![0u8; size].into(),
            meta: BlockMeta {
                entries_end: size,
                restart_start: 0,
                num_restarts: 1,
            },
        }
    }

    #[test]
    fn hit_promotes_and_clones_arc() {
        let cache = BlockCache::new(1000);
        cache.insert((1, 0), block(100));
        let hit = cache.get(&(1, 0)).expect("hit");
        let hit2 = cache.get(&(1, 0)).expect("hit");
        assert_eq!(hit.data.len(), 100);
        assert!(Arc::ptr_eq(&hit.data, &hit2.data));
    }

    #[test]
    fn miss_is_none_and_unknown_keys_do_not_evict() {
        let cache = BlockCache::new(1000);
        cache.insert((1, 0), block(100));
        assert!(cache.get(&(9, 9)).is_none());
        assert_eq!(cache.resident_entries(), 1);
    }

    #[test]
    fn lru_evicts_least_recently_used_first() {
        let cache = BlockCache::new(300);
        cache.insert((1, 0), block(100));
        cache.insert((2, 0), block(100));
        cache.insert((3, 0), block(100));
        // touch (1,0): now LRU order is 2,1,3
        cache.get(&(1, 0)).expect("present");
        cache.insert((4, 0), block(100)); // must evict (2,0)
        assert!(cache.get(&(2, 0)).is_none());
        assert!(cache.get(&(1, 0)).is_some());
        assert!(cache.get(&(3, 0)).is_some());
        assert!(cache.get(&(4, 0)).is_some());
        assert!(cache.resident_bytes() <= 300);
    }

    #[test]
    fn byte_budget_is_respected_under_pressure() {
        let cache = BlockCache::new(1000);
        for i in 0..50u64 {
            cache.insert((i, 0), block(90));
        }
        assert!(
            cache.resident_bytes() <= 1000,
            "resident {} exceeded budget",
            cache.resident_bytes()
        );
        // newest insert always survives
        assert!(cache.get(&(49, 0)).is_some());
    }

    #[test]
    fn oversize_blocks_bypass_the_cache() {
        let cache = BlockCache::new(500);
        cache.insert((1, 0), block(400));
        cache.insert((2, 0), block(4000)); // larger than budget
        assert!(cache.get(&(2, 0)).is_none());
        // the resident block survived the bypass attempt
        assert!(cache.get(&(1, 0)).is_some());
    }

    #[test]
    fn reinsert_of_live_key_does_not_duplicate_or_evict() {
        let cache = BlockCache::new(300);
        cache.insert((1, 0), block(100));
        cache.insert((1, 0), block(150));
        assert_eq!(cache.resident_entries(), 1);
        assert_eq!(cache.resident_bytes(), 100); // original retained
    }

    /// Phase 11.7, Test 3: exact hit/miss/eviction counts, not fuzzy
    /// assertions. A miss, then a hit on the same key, then a forced
    /// eviction under a too-small budget.
    #[test]
    fn counters_track_hits_misses_and_evictions_exactly() {
        let cache = BlockCache::new(150);
        assert!(cache.get(&(1, 0)).is_none()); // miss: nothing resident yet
        cache.insert((1, 0), block(100));
        assert!(cache.get(&(1, 0)).is_some()); // hit
        cache.insert((2, 0), block(100)); // must evict (1,0) to fit

        let s = cache.stats();
        assert_eq!(s.misses, 1);
        assert_eq!(s.hits, 1);
        assert_eq!(s.evictions, 1);
        assert_eq!(s.capacity_bytes, 150);
        assert_eq!(s.resident_entries, 1);

        // reading stats itself must not move any counter
        let s2 = cache.stats();
        assert_eq!(s2.hits, 1);
        assert_eq!(s2.misses, 1);
        assert_eq!(s2.evictions, 1);
    }
}
