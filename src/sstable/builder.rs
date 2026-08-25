//! Table builder: ordered entries in, one atomic byte blob out.
//!
//! Per `docs/design/sstable.md`: soft 4 KiB block cutting, delayed index
//! entries with shortest separators, fixed 28-byte footer with trailing
//! magic.

use super::block::{BLOCK_TYPE_NONE, BlockBuilder};
use super::{Kind, SstError};
use crate::crc32;

pub const TARGET_BLOCK_SIZE: usize = 4096;
pub const FOOTER_LEN: usize = 28;
pub const FORMAT_VERSION: u32 = 1;
pub const MAGIC: &[u8; 8] = b"KIBANSST";

struct PendingBlock {
    last_key: Vec<u8>,
    offset: u64,
    len: u64,
}

struct IndexEntry {
    separator: Vec<u8>,
    offset: u64,
    len: u64,
}

#[derive(Default)]
pub struct TableBuilder {
    out: Vec<u8>,
    block: BlockBuilder,
    pending: Option<PendingBlock>,
    index: Vec<IndexEntry>,
    last_key: Vec<u8>,
    has_last: bool,
}

impl TableBuilder {
    pub fn new() -> Self {
        TableBuilder::default()
    }

    pub fn add(&mut self, kind: Kind, key: &[u8], value: &[u8]) -> Result<(), SstError> {
        if self.has_last && key <= self.last_key.as_slice() {
            return Err(SstError::InvalidArgument(
                "keys must be added in strictly increasing order".to_string(),
            ));
        }
        if kind == Kind::Tombstone && !value.is_empty() {
            return Err(SstError::InvalidArgument(
                "tombstone entries must not carry a value".to_string(),
            ));
        }

        if let Some(p) = self.pending.take() {
            let separator = find_shortest_separator(&p.last_key, key);
            self.index.push(IndexEntry {
                separator,
                offset: p.offset,
                len: p.len,
            });
        }

        if !self.block.is_empty() && self.block.estimated_size() >= TARGET_BLOCK_SIZE {
            self.flush_block();
        }
        self.block.add(kind, key, value);

        self.last_key.clear();
        self.last_key.extend_from_slice(key);
        self.has_last = true;
        Ok(())
    }

    fn flush_block(&mut self) {
        if self.block.is_empty() {
            return;
        }
        let last_key = self.block.last_key().to_vec();
        let block = std::mem::take(&mut self.block);
        let finished = block.finish();
        let offset = self.out.len() as u64;
        let len = finished.len() as u64;
        self.out.extend_from_slice(&finished);
        self.pending = Some(PendingBlock {
            last_key,
            offset,
            len,
        });
    }

    pub fn finish(mut self) -> Result<Vec<u8>, SstError> {
        if self.pending.is_none() && self.block.is_empty() && !self.has_last {
            return Err(SstError::InvalidArgument(
                "cannot finish a table with no entries".to_string(),
            ));
        }
        self.flush_block();
        if let Some(p) = self.pending.take() {
            self.index.push(IndexEntry {
                separator: p.last_key,
                offset: p.offset,
                len: p.len,
            });
        }

        let mut idx = Vec::new();
        idx.extend_from_slice(&(self.index.len() as u32).to_le_bytes());
        for e in &self.index {
            idx.extend_from_slice(&(e.separator.len() as u32).to_le_bytes());
            idx.extend_from_slice(&e.separator);
            idx.extend_from_slice(&e.offset.to_le_bytes());
            idx.extend_from_slice(&e.len.to_le_bytes());
        }
        idx.push(BLOCK_TYPE_NONE);
        let crc = crc32::crc32(&idx);
        idx.extend_from_slice(&crc.to_le_bytes());

        let index_offset = self.out.len() as u64;
        let index_len = idx.len() as u64;
        self.out.extend_from_slice(&idx);

        self.out.extend_from_slice(&index_offset.to_le_bytes());
        self.out.extend_from_slice(&index_len.to_le_bytes());
        self.out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        self.out.extend_from_slice(MAGIC);
        debug_assert_eq!(
            self.out.len() as u64,
            index_offset + index_len + FOOTER_LEN as u64
        );

        Ok(self.out)
    }
}

/// Shortest separator `s` with `lo <= s < hi`, following LevelDB's
/// FindShortestSeparator: increment-truncate `lo` at the first differing
/// byte only when that stays strictly below `hi`; otherwise keep `lo`.
/// The invariant `lo <= s < hi` is what makes single-block point lookup
/// sound.
pub(crate) fn find_shortest_separator(lo: &[u8], hi: &[u8]) -> Vec<u8> {
    debug_assert!(lo < hi);
    let min = lo.len().min(hi.len());
    let mut p = 0usize;
    while p < min && lo[p] == hi[p] {
        p += 1;
    }
    if p < min && lo[p] != 0xFF && lo[p] + 1 < hi[p] {
        let mut sep = lo[..=p].to_vec();
        sep[p] += 1;
        return sep;
    }
    lo.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortest_separator_cases() {
        assert_eq!(
            find_shortest_separator(b"apple", b"application"),
            b"applf".to_vec()
        );
        // prefix case and no-room-to-increment case keep lo unchanged
        assert_eq!(find_shortest_separator(b"app", b"apple"), b"app".to_vec());
        assert_eq!(
            find_shortest_separator(b"a\xff\xff", b"b"),
            b"a\xff\xff".to_vec()
        );
        assert_eq!(find_shortest_separator(b"a", b"\xff"), b"b".to_vec());
        assert_eq!(find_shortest_separator(b"", b"x"), b"".to_vec());
    }

    #[test]
    fn separator_always_between_lo_and_hi() {
        let cases = [
            (&b"apple"[..], &b"application"[..]),
            (b"app", b"apple"),
            (b"k000000", b"k999999"),
            (b"\x00", b"\x00\x00\x01"),
            (b"\xfe\xff", b"\xff"),
            (b"\xff", b"\xff\x00"),
        ];
        for (lo, hi) in cases {
            let sep = find_shortest_separator(lo, hi);
            assert!(
                sep.as_slice() >= lo && sep.as_slice() < hi,
                "{lo:?}/{hi:?} -> {sep:?}"
            );
        }
    }

    #[test]
    fn finishing_empty_table_is_rejected() {
        assert!(TableBuilder::new().finish().is_err());
    }

    #[test]
    fn non_increasing_keys_are_rejected() {
        let mut b = TableBuilder::new();
        b.add(Kind::Put, b"k2", b"v").unwrap();
        assert!(b.add(Kind::Put, b"k2", b"v").is_err());
        assert!(b.add(Kind::Put, b"k1", b"v").is_err());
    }

    #[test]
    fn tombstones_must_not_carry_values() {
        let mut b = TableBuilder::new();
        assert!(b.add(Kind::Tombstone, b"k", b"v").is_err());
        b.add(Kind::Tombstone, b"k", b"").unwrap();
    }

    #[test]
    fn footer_is_exactly_footer_len() {
        let mut b = TableBuilder::new();
        b.add(Kind::Put, b"k", b"v").unwrap();
        let data = b.finish().unwrap();
        assert_eq!(&data[data.len() - 8..], MAGIC);
        assert!(data.len() >= FOOTER_LEN);
    }
}
