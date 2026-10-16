//! Byte-budgeted sparse cache of raw SST file ranges.
//!
//! Ported from HelixDB's `range-cache` crate (Apache-2.0,
//! <https://github.com/xav-db/range-cache>, version 0.1.1): ordered keys map
//! to sparse, non-overlapping byte ranges of immutable objects; adjacent
//! coverage merges; a payload-byte ceiling is enforced by range-level LRU
//! eviction keyed on a monotonically increasing access clock. Kiban keeps
//! its no-dependency rule, so payloads are `Arc<[u8]>` rather than `Bytes`
//! and locking uses `std::sync::Mutex`.
//!
//! Deliberate differences from the original:
//! - Inserts store only the uncovered gaps of their range instead of
//!   replacing overlapped bytes. SST files are immutable, so overlapped bytes
//!   are identical and need not be copied again.
//! - Adjacent merging is bounded by `max_merged_bytes`. Unbounded merging
//!   re-copies the whole growing range on every sequential read-ahead
//!   insert, which is quadratic for a forward scan of one file.
//! - Payloads are shared [`Window`]s. `get_window` returns the whole resident
//!   range containing a request (the analogue of the original's zero-copy
//!   `Bytes` slice), so a sequential reader can pin it and serve the next
//!   blocks without touching the cache again; `insert_window` stores the
//!   caller's buffer itself when it fills exactly one uncovered gap.
//!
//! The block cache holds verified, decoded blocks for repeated point and
//! scan access. This cache sits beneath it and holds raw read-ahead windows
//! fetched by sequential iterators, so neighbouring block misses are served
//! without another positional read. Keys are SST file numbers, which are
//! never reused within one database.

use crate::counter::Counter;
use std::collections::BTreeMap;
use std::ops::{Bound, Range};
use std::sync::{Arc, Mutex, PoisonError};

/// The effect of an insertion attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// At least one uncovered byte was admitted.
    Inserted,
    /// Existing coverage already contained the whole range.
    AlreadyCovered,
    /// The range is larger than the cache capacity (or the cache is
    /// disabled); nothing changed.
    TooLarge,
}

/// The effect of invalidating one key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Invalidation {
    pub ranges: usize,
    pub bytes: usize,
}

/// Raw counters and residency — facts only, like `BlockCacheStats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RangeCacheStats {
    pub capacity_bytes: usize,
    pub resident_bytes: usize,
    pub keys: usize,
    pub ranges: usize,
    /// Lookups fully contained in one resident range.
    pub hits: u64,
    /// Lookups with some, but not all, requested bytes resident.
    pub partial_hits: u64,
    /// Lookups with none of the requested bytes resident.
    pub misses: u64,
    pub insertions: u64,
    pub admissions_rejected_too_large: u64,
    pub evictions: u64,
    /// Positional reads issued by callers to fill this cache.
    pub source_reads: u64,
    /// Bytes those reads fetched.
    pub source_bytes: u64,
}

/// A shared, immutable run of raw file bytes.
pub type Window = Arc<Vec<u8>>;

struct CacheRange {
    end: u64,
    bytes: Window,
    last_access: u64,
}

struct State {
    ranges: BTreeMap<u64, BTreeMap<u64, CacheRange>>,
    /// Access clock -> (key, start). Exactly one entry per resident range.
    lru: BTreeMap<u64, (u64, u64)>,
    next_access: u64,
    resident_bytes: usize,
    resident_ranges: usize,
}

impl State {
    fn tick(&mut self) -> u64 {
        let access = self.next_access;
        // A u64 clock advanced once per access cannot wrap in practice;
        // saturating keeps the invariant checks below meaningful anyway.
        self.next_access = self.next_access.saturating_add(1);
        access
    }

    fn touch(&mut self, key: u64, start: u64) {
        let access = self.tick();
        let range = self
            .ranges
            .get_mut(&key)
            .and_then(|r| r.get_mut(&start))
            .expect("touched range is resident");
        let previous = std::mem::replace(&mut range.last_access, access);
        let entry = self
            .lru
            .remove(&previous)
            .expect("resident range has an LRU entry");
        debug_assert_eq!(entry, (key, start));
        self.lru.insert(access, entry);
    }

    fn remove(&mut self, key: u64, start: u64) -> Option<CacheRange> {
        let per_key = self.ranges.get_mut(&key)?;
        let range = per_key.remove(&start)?;
        if per_key.is_empty() {
            self.ranges.remove(&key);
        }
        let entry = self
            .lru
            .remove(&range.last_access)
            .expect("removed range has an LRU entry");
        debug_assert_eq!(entry, (key, start));
        self.resident_bytes -= range.bytes.len();
        self.resident_ranges -= 1;
        Some(range)
    }

    fn add(&mut self, key: u64, start: u64, bytes: Window) {
        let access = self.tick();
        let len = bytes.len();
        let previous = self.ranges.entry(key).or_default().insert(
            start,
            CacheRange {
                end: start + len as u64,
                bytes,
                last_access: access,
            },
        );
        assert!(previous.is_none(), "range start must be vacant");
        self.lru.insert(access, (key, start));
        self.resident_bytes += len;
        self.resident_ranges += 1;
    }

    /// Uncovered sub-ranges of `range` for `key`, ascending.
    fn gaps(&self, key: u64, range: Range<u64>) -> Vec<Range<u64>> {
        let Some(per_key) = self.ranges.get(&key) else {
            return vec![range];
        };
        let mut missing = Vec::new();
        let mut cursor = range.start;
        if let Some((_, r)) = per_key
            .range(..=range.start)
            .next_back()
            .filter(|(_, r)| r.end > range.start)
        {
            cursor = r.end.min(range.end);
        }
        for (&start, r) in per_key.range((Bound::Excluded(range.start), Bound::Excluded(range.end)))
        {
            if cursor < start {
                missing.push(cursor..start);
            }
            cursor = cursor.max(r.end.min(range.end));
            if cursor == range.end {
                break;
            }
        }
        if cursor < range.end {
            missing.push(cursor..range.end);
        }
        missing
    }
}

/// See the module documentation.
pub struct RangeCache {
    inner: Mutex<State>,
    capacity: usize,
    max_merged_bytes: usize,
    hits: Counter,
    partial_hits: Counter,
    misses: Counter,
    insertions: Counter,
    rejected: Counter,
    evictions: Counter,
    source_reads: Counter,
    source_bytes: Counter,
}

impl std::fmt::Debug for RangeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeCache")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl RangeCache {
    /// A cache holding at most `capacity_bytes` of payload. Zero disables
    /// it: lookups miss and inserts are rejected without locking.
    /// `max_merged_bytes` bounds how large adjacent merges may grow.
    pub fn new(capacity_bytes: usize, max_merged_bytes: usize) -> RangeCache {
        RangeCache {
            inner: Mutex::new(State {
                ranges: BTreeMap::new(),
                lru: BTreeMap::new(),
                next_access: 0,
                resident_bytes: 0,
                resident_ranges: 0,
            }),
            capacity: capacity_bytes,
            max_merged_bytes: max_merged_bytes.max(1),
            hits: Counter::default(),
            partial_hits: Counter::default(),
            misses: Counter::default(),
            insertions: Counter::default(),
            rejected: Counter::default(),
            evictions: Counter::default(),
            source_reads: Counter::default(),
            source_bytes: Counter::default(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn is_enabled(&self) -> bool {
        self.capacity > 0
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // Every mutation restores its invariants before it can panic only
        // through an `expect` on a violated invariant; continuing after such
        // a panic would be no worse than the panic itself.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns a copy of `range` when it is entirely resident.
    pub fn get(&self, key: u64, range: Range<u64>) -> Option<Vec<u8>> {
        if range.start > range.end {
            return None;
        }
        if range.is_empty() {
            return Some(Vec::new());
        }
        let (start, window) = self.get_window(key, range.clone())?;
        let offset = (range.start - start) as usize;
        Some(window[offset..offset + (range.end - range.start) as usize].to_vec())
    }

    /// Returns the start and bytes of the resident range wholly containing
    /// `range`, refreshing its LRU position. Partial coverage is a miss.
    pub fn get_window(&self, key: u64, range: Range<u64>) -> Option<(u64, Window)> {
        if !self.is_enabled() || range.start >= range.end {
            return None;
        }
        let mut state = self.state();
        let hit = state.ranges.get(&key).and_then(|per_key| {
            per_key
                .range(..=range.start)
                .next_back()
                .filter(|(_, r)| range.end <= r.end)
                .map(|(&start, r)| (start, r.bytes.clone()))
        });
        match hit {
            Some((start, window)) => {
                state.touch(key, start);
                drop(state);
                self.hits.increment();
                Some((start, window))
            }
            None => {
                let covered = state.gaps(key, range.clone()) != [range.clone()];
                drop(state);
                if covered {
                    self.partial_hits.increment();
                } else {
                    self.misses.increment();
                }
                None
            }
        }
    }

    /// The gaps within `range` that are not resident for `key`.
    pub fn missing_ranges(&self, key: u64, range: Range<u64>) -> Vec<Range<u64>> {
        if range.start >= range.end {
            return Vec::new();
        }
        if !self.is_enabled() {
            return vec![range];
        }
        self.state().gaps(key, range)
    }

    /// Records one caller positional read of `bytes` made to fill this cache.
    pub fn record_source_read(&self, bytes: usize) {
        self.source_reads.increment();
        self.source_bytes.add(bytes as u64);
    }

    /// Admits the uncovered parts of `[start, start + bytes.len())`.
    /// Adjacent resident ranges merge while the result stays within
    /// `max_merged_bytes`; otherwise gaps are stored as separate ranges.
    /// Existing ranges never overlap afterwards.
    pub fn insert(&self, key: u64, start: u64, bytes: &[u8]) -> InsertOutcome {
        self.admit(key, start, bytes, None)
    }

    /// Like `insert`, but stores `window` itself without copying when it
    /// fills exactly one uncovered gap and merges with nothing.
    pub fn insert_window(&self, key: u64, start: u64, window: &Window) -> InsertOutcome {
        self.admit(key, start, window, Some(window))
    }

    fn admit(&self, key: u64, start: u64, bytes: &[u8], whole: Option<&Window>) -> InsertOutcome {
        if bytes.is_empty() {
            return InsertOutcome::AlreadyCovered;
        }
        if bytes.len() > self.capacity {
            self.rejected.increment();
            return InsertOutcome::TooLarge;
        }
        let end = start + bytes.len() as u64;
        let mut state = self.state();
        let gaps = state.gaps(key, start..end);
        if gaps.is_empty() {
            return InsertOutcome::AlreadyCovered;
        }
        for gap in gaps {
            let payload = &bytes[(gap.start - start) as usize..(gap.end - start) as usize];
            let left = state.ranges.get(&key).and_then(|per_key| {
                per_key
                    .range(..gap.start)
                    .next_back()
                    .filter(|(_, r)| r.end == gap.start)
                    .map(|(&s, r)| (s, r.bytes.len()))
            });
            let right = state
                .ranges
                .get(&key)
                .and_then(|per_key| per_key.get(&gap.end).map(|r| r.bytes.len()));
            let mut merged_start = gap.start;
            let mut total = payload.len();
            let take_left = left.filter(|&(_, len)| total + len <= self.max_merged_bytes);
            if let Some((s, len)) = take_left {
                merged_start = s;
                total += len;
            }
            let take_right = right.filter(|&len| total + len <= self.max_merged_bytes);
            if let Some(len) = take_right {
                total += len;
            }
            let merged: Window = if take_left.is_none() && take_right.is_none() {
                match whole {
                    Some(window) if gap.start == start && gap.end == end => window.clone(),
                    _ => Arc::new(payload.to_vec()),
                }
            } else {
                let mut out = Vec::with_capacity(total);
                if let Some((s, _)) = take_left {
                    out.extend_from_slice(&state.remove(key, s).expect("left neighbour").bytes);
                }
                out.extend_from_slice(payload);
                if take_right.is_some() {
                    out.extend_from_slice(
                        &state.remove(key, gap.end).expect("right neighbour").bytes,
                    );
                }
                Arc::new(out)
            };
            state.add(key, merged_start, merged);
        }
        let mut evicted = 0;
        while state.resident_bytes > self.capacity {
            let (&_, &(victim_key, victim_start)) = state
                .lru
                .first_key_value()
                .expect("resident bytes require an LRU entry");
            state.remove(victim_key, victim_start);
            evicted += 1;
        }
        drop(state);
        self.insertions.increment();
        self.evictions.add(evicted);
        InsertOutcome::Inserted
    }

    /// Removes every range for `key` (an SST file being deleted).
    pub fn invalidate(&self, key: u64) -> Invalidation {
        if !self.is_enabled() {
            return Invalidation::default();
        }
        let mut state = self.state();
        let starts: Vec<u64> = state
            .ranges
            .get(&key)
            .map(|per_key| per_key.keys().copied().collect())
            .unwrap_or_default();
        let mut invalidation = Invalidation::default();
        for start in starts {
            let range = state.remove(key, start).expect("listed range is resident");
            invalidation.ranges += 1;
            invalidation.bytes += range.bytes.len();
        }
        invalidation
    }

    pub fn stats(&self) -> RangeCacheStats {
        let (resident_bytes, keys, ranges) = {
            let state = self.state();
            (
                state.resident_bytes,
                state.ranges.len(),
                state.resident_ranges,
            )
        };
        RangeCacheStats {
            capacity_bytes: self.capacity,
            resident_bytes,
            keys,
            ranges,
            hits: self.hits.load(),
            partial_hits: self.partial_hits.load(),
            misses: self.misses.load(),
            insertions: self.insertions.load(),
            admissions_rejected_too_large: self.rejected.load(),
            evictions: self.evictions.load(),
            source_reads: self.source_reads.load(),
            source_bytes: self.source_bytes.load(),
        }
    }

    #[cfg(test)]
    fn check_invariants(&self) {
        let state = self.state();
        let mut bytes = 0;
        let mut count = 0;
        for (&key, per_key) in &state.ranges {
            assert!(!per_key.is_empty());
            let mut previous_end = None;
            for (&start, r) in per_key {
                assert_eq!(r.end - start, r.bytes.len() as u64);
                assert!(previous_end.is_none_or(|e| e <= start), "ranges overlap");
                previous_end = Some(r.end);
                assert_eq!(state.lru.get(&r.last_access), Some(&(key, start)));
                bytes += r.bytes.len();
                count += 1;
            }
        }
        assert_eq!(bytes, state.resident_bytes);
        assert_eq!(count, state.resident_ranges);
        assert_eq!(count, state.lru.len());
        assert!(state.resident_bytes <= self.capacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn hit_partial_and_miss_are_counted() {
        let cache = RangeCache::new(1024, 1024);
        let file = data(64);
        assert_eq!(cache.insert(1, 4, &file[4..8]), InsertOutcome::Inserted);
        assert_eq!(cache.get(1, 5..7).as_deref(), Some(&file[5..7]));
        assert_eq!(cache.get(1, 2..6), None);
        assert_eq!(cache.get(1, 20..30), None);
        assert_eq!(cache.get(2, 4..8), None);
        let s = cache.stats();
        assert_eq!((s.hits, s.partial_hits, s.misses), (1, 1, 2));
        assert_eq!(cache.missing_ranges(1, 2..10), vec![2..4, 8..10]);
        cache.check_invariants();
    }

    #[test]
    fn adjacent_inserts_merge_within_the_bound_and_split_beyond_it() {
        let file = data(64);
        let cache = RangeCache::new(1024, 16);
        cache.insert(1, 0, &file[0..8]);
        cache.insert(1, 8, &file[8..16]);
        assert_eq!(cache.stats().ranges, 1, "merged within bound");
        cache.insert(1, 16, &file[16..24]);
        assert_eq!(cache.stats().ranges, 2, "bound prevents growth");
        assert_eq!(cache.get(1, 0..16).as_deref(), Some(&file[0..16]));
        assert_eq!(cache.get(1, 16..24).as_deref(), Some(&file[16..24]));
        assert_eq!(cache.missing_ranges(1, 0..24), Vec::<Range<u64>>::new());
        cache.check_invariants();
    }

    #[test]
    fn overlapping_insert_stores_only_gaps_and_bridges() {
        let file = data(64);
        let cache = RangeCache::new(1024, 1024);
        cache.insert(1, 8, &file[8..16]);
        cache.insert(1, 24, &file[24..32]);
        assert_eq!(
            cache.insert(1, 10, &file[10..12]),
            InsertOutcome::AlreadyCovered
        );
        assert_eq!(cache.insert(1, 4, &file[4..36]), InsertOutcome::Inserted);
        assert_eq!(cache.stats().ranges, 1);
        assert_eq!(cache.get(1, 4..36).as_deref(), Some(&file[4..36]));
        cache.check_invariants();
    }

    #[test]
    fn lru_evicts_least_recently_used_range() {
        let file = data(64);
        let cache = RangeCache::new(24, 8);
        cache.insert(1, 0, &file[0..8]);
        cache.insert(2, 0, &file[0..8]);
        cache.insert(3, 0, &file[0..8]);
        assert!(cache.get(1, 0..8).is_some());
        cache.insert(4, 0, &file[0..8]);
        assert!(cache.get(2, 0..8).is_none(), "untouched range evicted");
        assert!(cache.get(1, 0..8).is_some());
        assert!(cache.get(3, 0..8).is_some());
        assert_eq!(cache.stats().evictions, 1);
        cache.check_invariants();
    }

    #[test]
    fn oversized_and_disabled_caches_reject_without_state_change() {
        let cache = RangeCache::new(8, 8);
        cache.insert(1, 0, &[1; 4]);
        assert_eq!(cache.insert(1, 100, &[0; 9]), InsertOutcome::TooLarge);
        assert_eq!(cache.stats().resident_bytes, 4);
        let disabled = RangeCache::new(0, 8);
        assert_eq!(disabled.insert(1, 0, &[1]), InsertOutcome::TooLarge);
        assert_eq!(disabled.get(1, 0..1), None);
        assert_eq!(disabled.missing_ranges(1, 0..4), vec![0..4]);
    }

    #[test]
    fn windows_are_shared_without_copying_and_returned_whole() {
        let cache = RangeCache::new(1024, 8);
        let window: Window = Arc::new(data(64));
        assert_eq!(
            cache.insert_window(1, 100, &window),
            InsertOutcome::Inserted
        );
        let (start, got) = cache.get_window(1, 110..120).unwrap();
        assert_eq!(start, 100);
        assert!(
            Arc::ptr_eq(&got, &window),
            "exact gap is stored by reference"
        );
        assert!(cache.get_window(1, 90..110).is_none());
        // Overlapping windows store only their copied gaps.
        let wider: Window = Arc::new(data(96));
        cache.insert_window(1, 80, &wider);
        assert_eq!(cache.missing_ranges(1, 80..176), Vec::<Range<u64>>::new());
        cache.check_invariants();
    }

    #[test]
    fn invalidate_removes_only_that_key() {
        let file = data(64);
        let cache = RangeCache::new(1024, 8);
        cache.insert(1, 0, &file[0..8]);
        cache.insert(1, 16, &file[16..24]);
        cache.insert(2, 0, &file[0..8]);
        assert_eq!(
            cache.invalidate(1),
            Invalidation {
                ranges: 2,
                bytes: 16
            }
        );
        assert!(cache.get(1, 0..8).is_none());
        assert!(cache.get(2, 0..8).is_some());
        cache.check_invariants();
    }

    /// Randomized model check against a byte-level reference.
    #[test]
    fn randomized_operations_match_a_byte_model() {
        let file = data(4096);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        for (capacity, merge) in [(64, 16), (512, 64), (4096, 4096), (300, 7)] {
            let cache = RangeCache::new(capacity, merge);
            for _ in 0..4000 {
                let key = next(3);
                let start = next(4000);
                let len = 1 + next(96);
                let end = (start + len).min(4096);
                match next(4) {
                    0 | 1 => {
                        cache.insert(key, start, &file[start as usize..end as usize]);
                    }
                    2 => {
                        if let Some(got) = cache.get(key, start..end) {
                            assert_eq!(got, &file[start as usize..end as usize]);
                        }
                    }
                    _ => {
                        for gap in cache.missing_ranges(key, start..end) {
                            assert!(gap.start >= start && gap.end <= end && gap.start < gap.end);
                        }
                    }
                }
                if next(500) == 0 {
                    cache.invalidate(key);
                }
            }
            cache.check_invariants();
        }
    }
}
