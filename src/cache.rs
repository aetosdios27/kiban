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
}

#[derive(Debug)]
pub struct BlockCache {
    inner: Mutex<Inner>,
    capacity: usize,
}

impl BlockCache {
    pub fn new(capacity_bytes: usize) -> BlockCache {
        BlockCache {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: std::collections::VecDeque::new(),
                bytes: 0,
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

    /// Promotes on hit.
    pub fn get(&self, key: &(u64, u64)) -> Option<CachedBlock> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.map.get_mut(key)?;
        let block = entry.block.clone();
        Self::touch(&mut inner.order, *key);
        Some(block)
    }

    /// Inserts a verified block. Blocks larger than the whole budget are
    /// not admitted (the caller already holds the data it needs).
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
}
