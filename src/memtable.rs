//! The in-memory ordered store that accepts every write.
//!
//! Implements `docs/design/memtable.md`: byte keys in lexicographic
//! order, tombstone-on-delete, two iteration views.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Value(Vec<u8>),
    Tombstone,
}

impl Entry {
    pub fn as_value(&self) -> Option<&[u8]> {
        match self {
            Entry::Value(v) => Some(v),
            Entry::Tombstone => None,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        matches!(self, Entry::Tombstone)
    }
}

#[derive(Debug, Default)]
pub struct Memtable {
    map: BTreeMap<Vec<u8>, Entry>,
}

impl Memtable {
    pub fn new() -> Self {
        Memtable {
            map: BTreeMap::new(),
        }
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.map
            .insert(key.as_ref().to_vec(), Entry::Value(value.as_ref().to_vec()));
    }

    pub fn delete(&mut self, key: impl AsRef<[u8]>) {
        self.map.insert(key.as_ref().to_vec(), Entry::Tombstone);
    }

    /// Returns the live value for `key`, or `None` if the key is absent
    /// or deleted.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<Vec<u8>> {
        match self.map.get(key.as_ref()) {
            Some(Entry::Value(v)) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn contains_key(&self, key: impl AsRef<[u8]>) -> bool {
        self.get(key).is_some()
    }

    pub fn entry(&self, key: impl AsRef<[u8]>) -> Option<&Entry> {
        self.map.get(key.as_ref())
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
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &Entry)> + DoubleEndedIterator {
        self.map.iter().map(|(k, e)| (k.as_slice(), e))
    }

    /// Iterates live entries only (tombstones skipped), same ordering.
    pub fn iter_live(&self) -> impl Iterator<Item = (&[u8], &[u8])> + DoubleEndedIterator {
        self.map
            .iter()
            .filter_map(|(k, e)| e.as_value().map(|v| (k.as_slice(), v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get() {
        let mut m = Memtable::new();
        m.put("alpha", b"one".as_slice());
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
        m.put("k", "v1");
        m.put("k", "v2");
        assert_eq!(m.get("k"), Some(b"v2".to_vec()));
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn get_returns_owned_copy() {
        let mut m = Memtable::new();
        let value = b"owned".to_vec();
        m.put("k", value.clone());
        let mut got = m.get("k").unwrap();
        got.reverse();
        assert_eq!(m.get("k"), Some(value));
    }

    #[test]
    fn delete_hides_value_but_keeps_tombstone() {
        let mut m = Memtable::new();
        m.put("k", "v");
        m.delete("k");
        assert_eq!(m.get("k"), None);
        assert!(!m.contains_key("k"));
        assert_eq!(m.len(), 1);
        assert!(matches!(m.entry("k"), Some(Entry::Tombstone)));
    }

    #[test]
    fn delete_absent_key_still_records_tombstone() {
        let mut m = Memtable::new();
        m.delete("ghost");
        assert_eq!(m.get("ghost"), None);
        assert!(matches!(m.entry("ghost"), Some(Entry::Tombstone)));
    }

    #[test]
    fn reinsert_after_delete_is_live_again() {
        let mut m = Memtable::new();
        m.put("k", "v1");
        m.delete("k");
        m.put("k", "v2");
        assert_eq!(m.get("k"), Some(b"v2".to_vec()));
        assert!(matches!(m.entry("k"), Some(Entry::Value(_))));
    }

    #[test]
    fn empty_keys_and_values_are_legal() {
        let mut m = Memtable::new();
        m.put("", "");
        assert_eq!(m.get(""), Some(Vec::new()));
        m.delete("");
        assert_eq!(m.get(""), None);
    }

    #[test]
    fn iteration_is_byte_wise_ordered() {
        let mut m = Memtable::new();
        for key in [b"\xff".as_slice(), b"b", b"", b"a", b"\x00"] {
            m.put(key, "v");
        }
        let keys: Vec<&[u8]> = m.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec![&b""[..], b"\x00", b"a", b"b", b"\xff"]);
    }

    #[test]
    fn iter_includes_tombstones_iter_live_skips_them() {
        let mut m = Memtable::new();
        m.put("a", "1");
        m.put("b", "2");
        m.delete("a");
        m.put("c", "3");

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
            m.put(key, "v");
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
