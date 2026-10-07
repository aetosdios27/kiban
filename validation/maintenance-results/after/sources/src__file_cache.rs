//! Bounds the number of simultaneously open SST file descriptors
//! (phase 11.6). `SstTable` does not own a permanent file handle;
//! every read leases one from here, keyed by the sstable's file
//! number — stable and never reused for the life of one database, so
//! it works as a cache key.
//!
//! This is deliberately a *different* cache from [`crate::cache::BlockCache`]:
//! that one bounds RAM used by decoded, verified blocks; this one
//! bounds open OS descriptors. A block-cache hit needs no file lease
//! at all — see `SstTable::read_block`.
//!
//! The bound this cache enforces is real: unlike an `Arc<File>` LRU
//! where an evicted-but-still-cloned handle can keep a descriptor
//! alive outside the cache's accounting, callers here only ever get a
//! [`FileLease`] — a private handle that cannot be cloned out from
//! under the cache. `capacity` is therefore a hard ceiling on
//! concurrently open descriptors, not a target.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use crate::sys;

struct Entry {
    file: Arc<sys::File>,
    users: usize,
}

struct Inner {
    entries: HashMap<u64, Entry>,
    /// LRU order, least-recently-touched first. May contain in-use
    /// numbers; eviction skips those and only ever removes an idle
    /// (`users == 0`) one.
    order: VecDeque<u64>,
    /// Count of callers currently blocked in `acquire` because every
    /// resident descriptor is in use — real, condvar-observable state
    /// for deterministic tests, not inferred from elapsed time.
    waiters: usize,
    /// Phase 11.7 raw counters. `hits`: a resident descriptor was
    /// reused. `misses`: a new descriptor had to be opened (whether or
    /// not that also evicted something). `evictions`: an idle entry was
    /// removed to make room. `waits`: an `acquire` call had to block at
    /// least once because the cache was full and every descriptor was
    /// leased — counted once per such call, not once per wakeup.
    hits: u64,
    misses: u64,
    evictions: u64,
    waits: u64,
    /// Largest `entries.len()` ever observed — a test-only high-water
    /// mark, since periodically polling `resident()` from a test thread
    /// could miss a transient spike another thread causes in between
    /// checks; this can't.
    #[cfg(test)]
    max_resident_seen: usize,
}

/// Bounds concurrently-open SST descriptors to `capacity`. Opening
/// beyond capacity blocks — via `Condvar`, never a sleep or a poll —
/// until an existing lease is dropped or an idle entry is evicted.
pub(crate) struct TableFileCache {
    capacity: usize,
    inner: Mutex<Inner>,
    condvar: Condvar,
}

/// Raw counters for a [`TableFileCache`] (phase 11.7) — facts only, no
/// interpretation. `leased` counts descriptors currently checked out
/// (>= 1 use), which can differ from `resident` since a resident but
/// idle descriptor has zero current users.
#[derive(Debug, Clone, Copy)]
pub struct TableFileCacheStats {
    pub capacity: usize,
    pub resident: usize,
    pub leased: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub waits: u64,
}

/// A borrowed, reusable handle to one open SST file. Read through it
/// with [`FileLease::read_range_at`]; dropping it returns the slot to
/// the cache and wakes anyone waiting for room. The underlying
/// `sys::File` is never exposed, so a caller cannot clone a descriptor
/// out from under the cache's bound.
pub(crate) struct FileLease<'a> {
    cache: &'a TableFileCache,
    number: u64,
    file: Arc<sys::File>,
}

impl TableFileCache {
    pub(crate) fn new(capacity: usize) -> TableFileCache {
        TableFileCache {
            capacity,
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                waiters: 0,
                hits: 0,
                misses: 0,
                evictions: 0,
                waits: 0,
                #[cfg(test)]
                max_resident_seen: 0,
            }),
            condvar: Condvar::new(),
        }
    }

    /// Leases the SST file `number` at `path`, opening it (evicting an
    /// idle entry first if the cache is full, or waiting for one to
    /// become idle if every resident descriptor is in use) as needed.
    /// Opening happens under the cache lock — cache misses are rare
    /// enough for that to be acceptable — but the returned lease reads
    /// with no lock held at all.
    pub(crate) fn acquire(&self, number: u64, path: &Path) -> io::Result<FileLease<'_>> {
        let mut inner = self.inner.lock().unwrap();
        // Set once this call has genuinely had to block at least once,
        // so a call that loops through several wake/recheck cycles
        // before succeeding still counts as exactly one wait (11.7) —
        // the number measures blocked *operations*, not wakeups.
        let mut counted_wait = false;
        loop {
            if let Some(entry) = inner.entries.get_mut(&number) {
                entry.users += 1;
                let file = entry.file.clone();
                touch(&mut inner.order, number);
                inner.hits = inner.hits.saturating_add(1);
                return Ok(FileLease {
                    cache: self,
                    number,
                    file,
                });
            }

            if inner.entries.len() < self.capacity {
                inner.misses = inner.misses.saturating_add(1);
                let file = open_and_insert(&mut inner, number, path)?;
                return Ok(FileLease {
                    cache: self,
                    number,
                    file,
                });
            }

            if let Some(victim) = idle_victim(&inner) {
                // Dropping the entry drops its Arc<File>; since an
                // idle entry's only reference is the cache's own, the
                // descriptor closes right here.
                inner.entries.remove(&victim);
                remove(&mut inner.order, victim);
                inner.misses = inner.misses.saturating_add(1);
                inner.evictions = inner.evictions.saturating_add(1);
                let file = open_and_insert(&mut inner, number, path)?;
                return Ok(FileLease {
                    cache: self,
                    number,
                    file,
                });
            }

            // Every resident descriptor is in use: wait for a release.
            // `release`/eviction only ever happen after mutating this
            // same state under this same lock, immediately followed by
            // a notify — so a release landing between our check above
            // and the wait below cannot be missed (same discipline as
            // 11.5's backpressure wait).
            if !counted_wait {
                inner.waits = inner.waits.saturating_add(1);
                counted_wait = true;
            }
            inner.waiters += 1;
            self.condvar.notify_all();
            inner = self.condvar.wait(inner).unwrap();
            inner.waiters -= 1;
        }
    }

    /// A cheap, lock-once read of every counter (phase 11.7). No I/O,
    /// no effect on any counter it reads.
    pub(crate) fn stats(&self) -> TableFileCacheStats {
        let inner = self.inner.lock().unwrap();
        let leased = inner.entries.values().filter(|e| e.users > 0).count();
        TableFileCacheStats {
            capacity: self.capacity,
            resident: inner.entries.len(),
            leased,
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
            waits: inner.waits,
        }
    }

    fn release(&self, number: u64) {
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(entry) = inner.entries.get_mut(&number) {
                entry.users = entry.users.saturating_sub(1);
            }
        }
        self.condvar.notify_all();
    }

    /// Closes and removes table `number`'s cached descriptor, if any.
    /// Called right before physically unlinking an obsolete SST, so
    /// the unlinked inode's disk space isn't held open by a stale idle
    /// cache entry. Blocks until the entry is idle if it's currently
    /// leased — under Kiban's ownership model a table only reaches
    /// this call once nothing (no snapshot, no in-flight compaction)
    /// references it any more, so this should never actually observe
    /// `users > 0`, but it does not rely on that going unverified: it
    /// waits, rather than silently evicting something in use and
    /// letting its descriptor escape the bound untracked.
    pub(crate) fn invalidate(&self, number: u64) {
        let mut inner = self.inner.lock().unwrap();
        loop {
            match inner.entries.get(&number) {
                Some(entry) if entry.users == 0 => {
                    inner.entries.remove(&number);
                    remove(&mut inner.order, number);
                    return;
                }
                Some(_) => {
                    inner = self.condvar.wait(inner).unwrap();
                }
                None => return, // never opened, or already invalidated
            }
        }
    }

    /// How many SST descriptors this cache currently has open. Always
    /// `<= capacity`.
    #[cfg(test)]
    pub(crate) fn resident(&self) -> usize {
        self.inner.lock().unwrap().entries.len()
    }

    /// How many callers are, right now, genuinely blocked in `acquire`
    /// waiting for a free slot.
    #[cfg(test)]
    pub(crate) fn waiters(&self) -> usize {
        self.inner.lock().unwrap().waiters
    }

    /// Blocks until at least one caller is genuinely parked waiting
    /// for a slot — for deterministic tests, instead of inferring
    /// blocking from elapsed time.
    #[cfg(test)]
    pub(crate) fn wait_until_someone_waiting(&self) {
        let mut inner = self.inner.lock().unwrap();
        while inner.waiters == 0 {
            inner = self.condvar.wait(inner).unwrap();
        }
    }

    /// Whether table `number` currently has an open, cached descriptor
    /// — direct ground truth for eviction tests, not an inference from
    /// resident counts.
    #[cfg(test)]
    pub(crate) fn is_resident(&self, number: u64) -> bool {
        self.inner.lock().unwrap().entries.contains_key(&number)
    }

    /// The largest number of resident descriptors ever observed —
    /// robust proof the bound was never exceeded even for a single
    /// instant, unlike periodically polling `resident()` from a test
    /// thread while other threads race concurrently.
    #[cfg(test)]
    pub(crate) fn max_resident_seen(&self) -> usize {
        self.inner.lock().unwrap().max_resident_seen
    }
}

/// Opens `path` under the cache lock and inserts it as a fresh,
/// single-user entry — the one place a new descriptor is created, so
/// the test-only high-water mark is updated here and nowhere else.
fn open_and_insert(inner: &mut Inner, number: u64, path: &Path) -> io::Result<Arc<sys::File>> {
    let file = Arc::new(sys::File::open_read(path)?);
    inner.entries.insert(
        number,
        Entry {
            file: file.clone(),
            users: 1,
        },
    );
    inner.order.push_back(number);
    #[cfg(test)]
    {
        inner.max_resident_seen = inner.max_resident_seen.max(inner.entries.len());
    }
    Ok(file)
}

fn touch(order: &mut VecDeque<u64>, number: u64) {
    if let Some(pos) = order.iter().position(|n| *n == number) {
        order.remove(pos);
    }
    order.push_back(number);
}

fn remove(order: &mut VecDeque<u64>, number: u64) {
    if let Some(pos) = order.iter().position(|n| *n == number) {
        order.remove(pos);
    }
}

fn idle_victim(inner: &Inner) -> Option<u64> {
    inner
        .order
        .iter()
        .find(|n| inner.entries.get(n).is_some_and(|e| e.users == 0))
        .copied()
}

impl FileLease<'_> {
    /// Positioned read through the leased file — no cache lock is held
    /// while this runs.
    pub(crate) fn read_range_at(
        &self,
        path_for_sim: &Path,
        offset: u64,
        len: u64,
    ) -> io::Result<Vec<u8>> {
        self.file.read_range_at(path_for_sim, offset, len)
    }

    pub(crate) fn len(&self) -> io::Result<u64> {
        self.file.len()
    }
}

impl Drop for FileLease<'_> {
    fn drop(&mut self) {
        self.cache.release(self.number);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;

    fn make_file(dir: &std::path::Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    /// Test 2 (hard capacity): tracked resident descriptors never
    /// exceed `capacity`, across many more tables than that.
    #[test]
    fn resident_count_never_exceeds_capacity() {
        let td = TempDir::new("fc-capacity");
        let cache = TableFileCache::new(3);
        for n in 0..50u64 {
            let path = make_file(td.path(), &format!("{n}.sst"), b"data");
            let lease = cache.acquire(n, &path).unwrap();
            let bytes = lease.read_range_at(&path, 0, 4).unwrap();
            assert_eq!(bytes, b"data");
            drop(lease);
            assert!(cache.resident() <= 3, "resident exceeded capacity");
        }
        assert!(cache.max_resident_seen() <= 3);
    }

    /// Test 3 (LRU): capacity 3, access A B C A D — D's arrival must
    /// evict B, the least-recently-touched idle entry (A was touched
    /// again after C, C after B).
    #[test]
    fn lru_evicts_the_least_recently_touched_idle_entry() {
        let td = TempDir::new("fc-lru");
        let cache = TableFileCache::new(3);
        let pa = make_file(td.path(), "1.sst", b"a");
        let pb = make_file(td.path(), "2.sst", b"b");
        let pc = make_file(td.path(), "3.sst", b"c");
        let pd = make_file(td.path(), "4.sst", b"d");

        drop(cache.acquire(1, &pa).unwrap()); // A
        drop(cache.acquire(2, &pb).unwrap()); // B
        drop(cache.acquire(3, &pc).unwrap()); // C
        drop(cache.acquire(1, &pa).unwrap()); // A again: touched, LRU order is now B, C, A
        drop(cache.acquire(4, &pd).unwrap()); // D: must evict B, the LRU idle entry

        assert_eq!(cache.resident(), 3);
        assert!(cache.is_resident(1), "A must survive");
        assert!(!cache.is_resident(2), "B must have been evicted");
        assert!(cache.is_resident(3), "C must survive");
        assert!(cache.is_resident(4), "D must be the newly opened entry");
    }

    /// Test 4 (in-use handle cannot be evicted): capacity 1; a held
    /// lease on A blocks a second acquire on B until A is dropped —
    /// B must not evict A, open anyway, or exceed capacity.
    #[test]
    fn in_use_handle_blocks_rather_than_being_evicted() {
        let td = TempDir::new("fc-inuse");
        let cache = std::sync::Arc::new(TableFileCache::new(1));
        let pa = make_file(td.path(), "1.sst", b"a");
        let pb = make_file(td.path(), "2.sst", b"b");

        let lease_a = cache.acquire(1, &pa).unwrap();
        assert_eq!(cache.resident(), 1);

        let cache2 = cache.clone();
        let handle = std::thread::spawn(move || {
            let lease_b = cache2.acquire(2, &pb).unwrap();
            lease_b.read_range_at(&pb, 0, 1).unwrap()
        });

        cache.wait_until_someone_waiting();
        assert_eq!(cache.waiters(), 1);
        assert!(!handle.is_finished(), "B must wait, not proceed");
        assert_eq!(cache.resident(), 1, "capacity must not be exceeded");

        drop(lease_a);
        let got = handle.join().unwrap();
        assert_eq!(got, b"b");
        assert_eq!(cache.resident(), 1);
    }

    /// Test 5 (no missed wakeup): repeated full-cache / release /
    /// reacquire cycles across threads. A lost-wakeup bug would hang
    /// this test rather than fail an assertion — that is the proof.
    #[test]
    fn no_missed_wakeups_under_repeated_pressure() {
        let td = TempDir::new("fc-no-missed-wakeup");
        let cache = std::sync::Arc::new(TableFileCache::new(2));
        let paths: Vec<_> = (0..6u64)
            .map(|n| make_file(td.path(), &format!("{n}.sst"), b"x"))
            .collect();

        for _round in 0..8 {
            let handles: Vec<_> = (0..6u64)
                .map(|n| {
                    let cache = cache.clone();
                    let path = paths[n as usize].clone();
                    std::thread::spawn(move || {
                        for _ in 0..10 {
                            let lease = cache.acquire(n, &path).unwrap();
                            let _ = lease.read_range_at(&path, 0, 1).unwrap();
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
        }
        assert!(cache.max_resident_seen() <= 2);
    }

    /// A file's cache entry can be invalidated (and its descriptor
    /// closed) once idle, and re-acquiring it afterward reopens
    /// cleanly.
    #[test]
    fn invalidate_removes_an_idle_entry_and_allows_reacquire() {
        let td = TempDir::new("fc-invalidate");
        let cache = TableFileCache::new(4);
        let path = make_file(td.path(), "1.sst", b"z");

        drop(cache.acquire(1, &path).unwrap());
        assert_eq!(cache.resident(), 1);
        cache.invalidate(1);
        assert_eq!(cache.resident(), 0);

        // reacquiring after invalidation reopens without issue
        let lease = cache.acquire(1, &path).unwrap();
        assert_eq!(lease.read_range_at(&path, 0, 1).unwrap(), b"z");
    }

    /// Invalidating a number that was never opened (or already
    /// invalidated) is a harmless no-op.
    #[test]
    fn invalidate_of_unknown_number_is_a_no_op() {
        let cache = TableFileCache::new(4);
        cache.invalidate(999);
        assert_eq!(cache.resident(), 0);
    }

    /// Phase 11.7, Test 4: capacity 2, access sequence A A B C — exact
    /// hit/miss/eviction counts, no fuzzy assertions.
    #[test]
    fn counters_track_hits_misses_and_evictions_exactly() {
        let td = TempDir::new("fc-counters");
        let cache = TableFileCache::new(2);
        let pa = make_file(td.path(), "1.sst", b"a");
        let pb = make_file(td.path(), "2.sst", b"b");
        let pc = make_file(td.path(), "3.sst", b"c");

        drop(cache.acquire(1, &pa).unwrap()); // A: miss
        drop(cache.acquire(1, &pa).unwrap()); // A again: hit
        drop(cache.acquire(2, &pb).unwrap()); // B: miss (room available)
        drop(cache.acquire(3, &pc).unwrap()); // C: miss + eviction (cache full)

        let s = cache.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 3);
        assert_eq!(s.evictions, 1);
        assert!(s.resident <= s.capacity);

        // reading stats itself must not move any counter
        let s2 = cache.stats();
        assert_eq!((s2.hits, s2.misses, s2.evictions), (1, 3, 1));
    }

    /// Phase 11.7, Test 5: one `acquire` call that blocks (however many
    /// times it wakes and rechecks) counts as exactly one wait, not one
    /// per wakeup. Capacity 1; A is held while a second thread requests
    /// B and must park.
    #[test]
    fn wait_counter_counts_blocked_calls_not_wakeups() {
        let td = TempDir::new("fc-wait-counter");
        let cache = std::sync::Arc::new(TableFileCache::new(1));
        let pa = make_file(td.path(), "1.sst", b"a");
        let pb = make_file(td.path(), "2.sst", b"b");

        let lease_a = cache.acquire(1, &pa).unwrap();
        assert_eq!(cache.stats().waits, 0);

        let cache2 = cache.clone();
        let handle = std::thread::spawn(move || {
            let lease_b = cache2.acquire(2, &pb).unwrap();
            lease_b.read_range_at(&pb, 0, 1).unwrap()
        });

        cache.wait_until_someone_waiting();
        // Nudge extra spurious-wakeup-shaped activity: notify_all with
        // the condition still false. A correct implementation only
        // counts the blocked acquire once regardless.
        cache.condvar.notify_all();
        std::thread::yield_now();
        assert!(!handle.is_finished(), "B must still be waiting");

        drop(lease_a);
        let got = handle.join().unwrap();
        assert_eq!(got, b"b");

        assert_eq!(cache.stats().waits, 1);
    }
}
