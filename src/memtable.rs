//! The in-memory ordered store that accepts every write.
//!
//! Implements `ARCHITECTURE.md`: byte keys in lexicographic
//! order, tombstone-on-delete, two iteration views.

use std::collections::BTreeMap;

/// A deliberately rough per-entry cost (phase 11.8): key bytes + value
/// bytes (0 for a tombstone) + this fixed overhead. Not a claim about
/// actual heap/RSS usage — cheap, deterministic, and monotonic enough
/// to threshold flushes on is the whole requirement.
const ENTRY_OVERHEAD_BYTES: usize = 32;

fn entry_size(key_len: usize, entry: &Entry) -> usize {
    let value_len = entry.as_value().map(|v| v.len()).unwrap_or(0);
    key_len + value_len + ENTRY_OVERHEAD_BYTES
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Value { value: Vec<u8>, seq: u64 },
    Tombstone { seq: u64 },
}

impl Entry {
    pub fn as_value(&self) -> Option<&[u8]> {
        match self {
            Entry::Value { value, .. } => Some(value),
            Entry::Tombstone { .. } => None,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        matches!(self, Entry::Tombstone { .. })
    }

    /// Sequence number at which this entry was written.
    pub fn seq(&self) -> u64 {
        match self {
            Entry::Value { seq, .. } | Entry::Tombstone { seq } => *seq,
        }
    }
}

/// One user key's retained versions: the live (newest) entry plus any
/// superseded entries still observable through active snapshots.
#[derive(Debug, Clone)]
struct KeyVersions {
    live: Entry,
    /// Superseded entries, oldest first; appending does not shift history. Retained only while an active
    /// snapshot bound requires them.
    history: Vec<Entry>,
}

impl Default for KeyVersions {
    fn default() -> Self {
        KeyVersions {
            // seq 0 is never visible to any read (seqs start at 1), so a
            // placeholder tombstone is equivalent to "no entry".
            live: Entry::Tombstone { seq: 0 },
            history: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Memtable {
    map: BTreeMap<Vec<u8>, KeyVersions>,
    /// Optional low-level retention bounds. Engine snapshots copy the active
    /// memtable, so engine memtables use no retained history by default.
    min_active: u64,
    max_active: u64,
    /// Running total of `entry_size` over every currently resident
    /// entry (live + retained history) — phase 11.8's flush-threshold
    /// signal. Updated incrementally, never recomputed from scratch.
    logical_bytes: usize,
    /// Mutation volume in this generation, independent of overwritten values.
    /// Bounds WAL growth even when a hot working set stays small in memory.
    written_bytes: usize,
}

impl Default for Memtable {
    fn default() -> Self {
        Self::new()
    }
}

impl Memtable {
    pub fn new() -> Self {
        Memtable {
            map: BTreeMap::new(),
            min_active: u64::MAX,
            max_active: u64::MAX,
            logical_bytes: 0,
            written_bytes: 0,
        }
    }

    fn insert_entry(&mut self, key: &[u8], new: Entry) {
        self.written_bytes = self
            .written_bytes
            .saturating_add(entry_size(key.len(), &new));
        let slot = self.map.entry(key.to_vec()).or_default();
        debug_assert!(
            slot.live.seq() < new.seq(),
            "memtable inserts must be seq-ascending per key"
        );
        // Retain the superseded version iff some active snapshot can
        // still observe it; otherwise it stops being resident.
        if self.max_active != u64::MAX && slot.live.seq() != 0 && slot.live.seq() <= self.max_active
        {
            slot.history.push(slot.live.clone());
        } else if slot.live.seq() != 0 {
            self.logical_bytes -= entry_size(key.len(), &slot.live);
        }
        self.logical_bytes += entry_size(key.len(), &new);
        slot.live = new;
    }

    /// A cheap, deterministic estimate of currently resident bytes
    /// (live + retained history versions) — see `entry_size`. Not
    /// exact heap accounting; monotonic enough to threshold flushes on.
    pub fn logical_bytes(&self) -> usize {
        self.logical_bytes
    }

    /// Bytes of logical mutations accepted in this generation.
    pub fn write_bytes(&self) -> usize {
        self.written_bytes
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>, seq: u64) {
        self.insert_entry(
            key.as_ref(),
            Entry::Value {
                value: value.as_ref().to_vec(),
                seq,
            },
        );
    }

    pub fn delete(&mut self, key: impl AsRef<[u8]>, seq: u64) {
        self.insert_entry(key.as_ref(), Entry::Tombstone { seq });
    }

    /// Returns the live value for `key`, or `None` if the key is absent
    /// or deleted.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<Vec<u8>> {
        self.entry(key)?.as_value().map(|v| v.to_vec())
    }

    pub fn contains_key(&self, key: impl AsRef<[u8]>) -> bool {
        self.get(key).is_some()
    }

    pub fn entry(&self, key: impl AsRef<[u8]>) -> Option<&Entry> {
        self.map.get(key.as_ref()).map(|kv| &kv.live)
    }

    /// Newest entry for `key` visible at snapshot bound `snap`: the live
    /// entry if its seq <= snap, otherwise the newest history entry with
    /// seq <= snap.
    pub fn entry_at(&self, key: impl AsRef<[u8]>, snap: u64) -> Option<&Entry> {
        let slot = self.map.get(key.as_ref())?;
        if slot.live.seq() <= snap {
            return Some(&slot.live);
        }
        // Reverse the append-only history to select the newest visible version.
        slot.history.iter().rev().find(|e| e.seq() <= snap)
    }

    /// Updates snapshot retention bounds and prunes history no active
    /// snapshot can observe.
    pub fn set_snapshot_bounds(&mut self, min_active: u64, max_active: u64) {
        self.min_active = min_active;
        self.max_active = max_active;
        let mut removed_bytes = 0usize;
        for (key, kv) in self.map.iter_mut() {
            // Keep the newest version at/below the oldest active boundary,
            // plus newer versions. With no boundaries all history is dead.
            let boundary = kv
                .history
                .iter()
                .rev()
                .find(|e| e.seq() <= min_active)
                .map(Entry::seq);
            kv.history.retain(|e| {
                let keep =
                    max_active != u64::MAX && (e.seq() > min_active || Some(e.seq()) == boundary);
                if !keep {
                    removed_bytes += entry_size(key.len(), e);
                }
                keep
            });
        }
        self.logical_bytes = self.logical_bytes.saturating_sub(removed_bytes);
    }

    /// Iterates every retained version of every key: ascending keys, and
    /// within a key, live first then history (seq descending) — the order
    /// the flush builder requires.
    pub fn iter_all_versions(&self) -> impl DoubleEndedIterator<Item = (&[u8], &Entry)> + '_ {
        self.map.iter().flat_map(|(k, kv)| {
            std::iter::once(&kv.live)
                .chain(kv.history.iter().rev())
                .filter(|e| e.seq() != 0)
                .map(move |e| (k.as_slice(), e))
        })
    }

    /// Number of entries stored, tombstones included.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates all entries in ascending byte-wise key order, tombstones
    /// included.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&[u8], &Entry)> + '_ {
        self.map.iter().map(|(k, kv)| (k.as_slice(), &kv.live))
    }

    /// Iterates all entries from `start` (inclusive) in ascending
    /// byte-wise key order, tombstones included.
    pub fn iter_from(&self, start: &[u8]) -> impl DoubleEndedIterator<Item = (&[u8], &Entry)> + '_ {
        let start = start.to_vec();
        self.map.range(start..).flat_map(|(k, kv)| {
            std::iter::once(&kv.live)
                .chain(kv.history.iter().rev())
                .map(move |e| (k.as_slice(), e))
        })
    }

    /// Iterates live entries only (tombstones skipped), same ordering.
    pub fn iter_live(&self) -> impl DoubleEndedIterator<Item = (&[u8], &[u8])> + '_ {
        self.iter()
            .filter_map(|(k, e)| e.as_value().map(|v| (k, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get() {
        let mut m = Memtable::new();
        m.put("alpha", b"one".as_slice(), 1);
        assert_eq!(m.get("alpha"), Some(b"one".to_vec()));
    }

    #[test]
    fn missing_key_is_none() {
        let m = Memtable::new();
        assert_eq!(m.get("nope"), None);
    }

    #[test]
    fn overwrite_replaces_value() {
        let mut m = Memtable::new();
        m.put("k", "v1", 2);
        m.put("k", "v2", 3);
        assert_eq!(m.get("k"), Some(b"v2".to_vec()));
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn get_returns_owned_copy() {
        let mut m = Memtable::new();
        let value = b"owned".to_vec();
        m.put("k", value.clone(), 4);
        let mut got = m.get("k").unwrap();
        got.reverse();
        assert_eq!(m.get("k"), Some(value));
    }

    #[test]
    fn delete_hides_value_but_keeps_tombstone() {
        let mut m = Memtable::new();
        m.put("k", "v", 5);
        m.delete("k", 6);
        assert_eq!(m.get("k"), None);
        assert!(!m.contains_key("k"));
        assert_eq!(m.len(), 1);
        assert!(matches!(m.entry("k"), Some(Entry::Tombstone { .. })));
    }

    #[test]
    fn delete_absent_key_still_records_tombstone() {
        let mut m = Memtable::new();
        m.delete("ghost", 7);
        assert_eq!(m.get("ghost"), None);
        assert!(matches!(m.entry("ghost"), Some(Entry::Tombstone { .. })));
    }

    #[test]
    fn reinsert_after_delete_is_live_again() {
        let mut m = Memtable::new();
        m.put("k", "v1", 8);
        m.delete("k", 9);
        m.put("k", "v2", 10);
        assert_eq!(m.get("k"), Some(b"v2".to_vec()));
        assert!(matches!(m.entry("k"), Some(Entry::Value { .. })));
    }

    #[test]
    fn empty_keys_and_values_are_legal() {
        let mut m = Memtable::new();
        m.put("", "", 11);
        assert_eq!(m.get(""), Some(Vec::new()));
        m.delete("", 12);
        assert_eq!(m.get(""), None);
    }

    #[test]
    fn iteration_is_byte_wise_ordered() {
        let mut m = Memtable::new();
        for key in [b"\xff".as_slice(), b"b", b"", b"a", b"\x00"] {
            m.put(key, "v", 13);
        }
        let keys: Vec<&[u8]> = m.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec![&b""[..], b"\x00", b"a", b"b", b"\xff"]);
    }

    #[test]
    fn iter_includes_tombstones_iter_live_skips_them() {
        let mut m = Memtable::new();
        m.put("a", "1", 14);
        m.put("b", "2", 15);
        m.delete("a", 16);
        m.put("c", "3", 17);

        let raw: Vec<(&[u8], bool)> = m.iter().map(|(k, e)| (k, e.is_tombstone())).collect();
        assert_eq!(
            raw,
            vec![(&b"a"[..], true), (&b"b"[..], false), (&b"c"[..], false)]
        );

        let live: Vec<&[u8]> = m.iter_live().map(|(k, _)| k).collect();
        assert_eq!(live, vec![&b"b"[..], b"c"]);
    }

    #[test]
    fn iterators_reverse_traverse() {
        let mut m = Memtable::new();
        for key in ["a", "b", "c"] {
            m.put(key, "v", 18);
        }
        let rev: Vec<&[u8]> = m.iter_live().rev().map(|(k, _)| k).collect();
        assert_eq!(rev, vec![&b"c"[..], b"b", b"a"]);
    }

    #[test]
    fn empty_memtable_is_empty_and_iterates_nothing() {
        let m = Memtable::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert_eq!(m.iter().count(), 0);
    }
}
