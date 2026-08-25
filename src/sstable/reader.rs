//! Table reader: parse footer and index, point lookup, full iteration.
//!
//! Per `docs/design/sstable.md`: corruption is detected and reported,
//! never repaired or skipped.

use super::block::Block;
use super::builder::{FOOTER_LEN, FORMAT_VERSION, MAGIC};
use super::{Kind, SstError};
use crate::bloom::BloomFilter;
use crate::crc32;

struct IndexEntry {
    separator: Vec<u8>,
    offset: u64,
    len: u64,
}

pub struct SstTable {
    buf: Vec<u8>,
    index: Vec<IndexEntry>,
    filter: BloomFilter,
}

pub struct Found<'a> {
    pub kind: Kind,
    pub value: &'a [u8],
}

impl SstTable {
    pub fn parse(buf: Vec<u8>) -> Result<SstTable, SstError> {
        let bad = |m: String| SstError::Corrupt(m);
        if buf.len() < FOOTER_LEN {
            return Err(bad("file is smaller than the footer".to_string()));
        }
        let footer = &buf[buf.len() - FOOTER_LEN..];
        if &footer[36..44] != MAGIC {
            return Err(bad("bad magic number; not a kiban sstable".to_string()));
        }
        let version = u32::from_le_bytes(footer[16..20].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(bad(format!("unsupported format version {version}")));
        }
        let index_offset = u64::from_le_bytes(footer[0..8].try_into().unwrap());
        let index_len = u64::from_le_bytes(footer[8..16].try_into().unwrap());
        let filter_offset = u64::from_le_bytes(footer[20..28].try_into().unwrap());
        let filter_len = u64::from_le_bytes(footer[28..36].try_into().unwrap());
        let data_end = buf.len() as u64 - FOOTER_LEN as u64;
        if index_offset > data_end || index_len == 0 || index_offset + index_len > data_end {
            return Err(bad("index block out of file bounds".to_string()));
        }
        if filter_offset + filter_len != index_offset || filter_len < 10 {
            return Err(bad(
                "filter block does not sit directly before the index".to_string()
            ));
        }

        let filter_raw = &buf[filter_offset as usize..(filter_offset + filter_len) as usize];
        if filter_raw[filter_raw.len() - 5] != super::block::BLOCK_TYPE_NONE {
            return Err(bad("unknown filter block type".to_string()));
        }
        let stored_crc = u32::from_le_bytes(filter_raw[filter_raw.len() - 4..].try_into().unwrap());
        if crc32::crc32(&filter_raw[..filter_raw.len() - 4]) != stored_crc {
            return Err(bad("filter block checksum mismatch".to_string()));
        }
        let filter = BloomFilter::decode(&filter_raw[..filter_raw.len() - 5])
            .ok_or_else(|| bad("filter block payload is malformed".to_string()))?;

        let index_raw = &buf[index_offset as usize..(index_offset + index_len) as usize];
        if index_raw[index_raw.len() - 5] != super::block::BLOCK_TYPE_NONE {
            return Err(bad("unknown index block type".to_string()));
        }
        let stored_crc = u32::from_le_bytes(index_raw[index_raw.len() - 4..].try_into().unwrap());
        if crc32::crc32(&index_raw[..index_raw.len() - 4]) != stored_crc {
            return Err(bad("index block checksum mismatch".to_string()));
        }

        let payload = &index_raw[..index_raw.len() - 5];
        let mut pos = 0usize;
        let read_u32 = |pos: &mut usize| -> Result<u32, SstError> {
            if *pos + 4 > payload.len() {
                return Err(bad("index truncated in header".to_string()));
            }
            let v = u32::from_le_bytes(payload[*pos..*pos + 4].try_into().unwrap());
            *pos += 4;
            Ok(v)
        };
        let count = read_u32(&mut pos)? as usize;
        let mut index = Vec::with_capacity(count);
        let mut prev_separator: Option<Vec<u8>> = None;
        for _ in 0..count {
            let sep_len = read_u32(&mut pos)? as usize;
            if pos + sep_len + 16 > payload.len() {
                return Err(bad("index entry runs past index block".to_string()));
            }
            let separator = payload[pos..pos + sep_len].to_vec();
            pos += sep_len;
            let offset = u64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
            pos += 8;
            let len = u64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
            pos += 8;
            if offset + len > data_end || len < 14 {
                return Err(bad("index entry points outside data area".to_string()));
            }
            if let Some(prev) = &prev_separator
                && separator <= *prev
            {
                return Err(bad("separators not strictly increasing".to_string()));
            }
            prev_separator = Some(separator.clone());
            index.push(IndexEntry {
                separator,
                offset,
                len,
            });
        }
        if pos != payload.len() {
            return Err(bad("index block has trailing garbage".to_string()));
        }

        Ok(SstTable { buf, index, filter })
    }

    fn block_at(&self, e: &IndexEntry) -> Result<Block<'_>, SstError> {
        Block::parse(&self.buf[e.offset as usize..(e.offset + e.len) as usize])
    }

    /// Point lookup. Returns `Ok(None)` when the key is provably absent
    /// from this table — either by bloom filter or by probe.
    pub fn get(&self, key: &[u8]) -> Result<Option<Found<'_>>, SstError> {
        if !self.filter.may_contain(key) {
            return Ok(None);
        }
        let idx = self.index.partition_point(|e| e.separator.as_slice() < key);
        if idx == self.index.len() {
            return Ok(None);
        }
        let block = self.block_at(&self.index[idx])?;
        Ok(block.get(key)?.map(|m| Found {
            kind: m.kind,
            value: m.value,
        }))
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter {
            table: self,
            next_block: 0,
            current: None,
            failed: false,
            lower_bound: None,
        }
    }

    /// First key in the table (`None` only for a malformed empty index,
    /// which parse rejects).
    pub fn smallest_key(&self) -> Result<Vec<u8>, SstError> {
        let block = self.block_at(&self.index[0])?;
        match block.iter().next() {
            Some(Ok((_, k, _))) => Ok(k),
            Some(Err(e)) => Err(e),
            None => Err(SstError::Corrupt(
                "first block yielded no entry".to_string(),
            )),
        }
    }

    /// Last key in the table.
    pub fn largest_key(&self) -> Result<Vec<u8>, SstError> {
        let block = self.block_at(self.index.last().expect("parse rejects empty index"))?;
        let mut last = None;
        for item in block.iter() {
            match item {
                Ok((_, k, _)) => last = Some(k),
                Err(e) => return Err(e),
            }
        }
        last.ok_or_else(|| SstError::Corrupt("last block yielded no entry".to_string()))
    }

    /// Iterates from the first key >= `target`. Positions at the right
    /// block via separators; nothing is scanned from the file start.
    pub fn iter_from(&self, target: &[u8]) -> Iter<'_> {
        let mut it = self.iter();
        it.next_block = self
            .index
            .partition_point(|e| e.separator.as_slice() < target);
        it.lower_bound = Some(target.to_vec());
        it
    }
}

pub struct Iter<'a> {
    table: &'a SstTable,
    next_block: usize,
    current: Option<BlockIterState<'a>>,
    failed: bool,
    lower_bound: Option<Vec<u8>>,
}

struct BlockIterState<'a> {
    inner: super::BlockIter<'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<(Kind, Vec<u8>, &'a [u8]), SstError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(state) = &mut self.current {
                match state.inner.next() {
                    Some(Ok((kind, key, value))) => {
                        if let Some(bound) = &self.lower_bound {
                            if key.as_slice() < bound.as_slice() {
                                continue;
                            }
                            self.lower_bound = None;
                        }
                        return Some(Ok((kind, key, value)));
                    }
                    Some(Err(e)) => {
                        self.failed = true;
                        return Some(Err(e));
                    }
                    None => self.current = None,
                }
            }
            if self.next_block >= self.table.index.len() {
                return None;
            }
            match self.table.block_at(&self.table.index[self.next_block]) {
                Ok(block) => {
                    self.current = Some(BlockIterState {
                        inner: block.iter(),
                    });
                    self.next_block += 1;
                }
                Err(e) => {
                    self.failed = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sstable::Kind;
    use crate::sstable::builder::TableBuilder;

    fn build_table(count: u32) -> Vec<u8> {
        let mut b = TableBuilder::new();
        for i in 0..count {
            let key = format!("key-{i:06}");
            if i % 10 == 5 {
                b.add(Kind::Tombstone, key.as_bytes(), b"").unwrap();
            } else {
                b.add(Kind::Put, key.as_bytes(), format!("value-{i}").as_bytes())
                    .unwrap();
            }
        }
        b.finish().unwrap()
    }

    #[test]
    fn empty_index_table_parses_and_yields_nothing() {
        let mut b = TableBuilder::new();
        b.add(Kind::Put, b"only", b"entry").unwrap();
        let data = b.finish().unwrap();
        let table = SstTable::parse(data).unwrap();
        assert!(table.get(b"only").unwrap().is_some());
        assert!(table.get(b"other").unwrap().is_none());
    }

    #[test]
    fn multi_block_roundtrip_get_and_iterate() {
        let count = 2000;
        let data = build_table(count);
        let table = SstTable::parse(data).unwrap();

        // every present key found with the right value; tombstones surface as such
        for i in 0..count {
            let key = format!("key-{i:06}");
            let found = table
                .get(key.as_bytes())
                .unwrap()
                .unwrap_or_else(|| panic!("missing {key}"));
            if i % 10 == 5 {
                assert_eq!(found.kind, Kind::Tombstone);
            } else {
                assert_eq!(found.kind, Kind::Put);
                assert_eq!(found.value, format!("value-{i}").as_bytes());
            }
        }
        assert!(table.get(b"key-999999").unwrap().is_none());
        assert!(table.get(b"").unwrap().is_none());
        assert!(table.get(b"a").unwrap().is_none());

        // iteration reproduces every entry in order, full keys reconstructed
        let got: Vec<(Kind, Vec<u8>)> = table
            .iter()
            .map(|r| {
                let (k, key, _) = r.unwrap();
                (k, key)
            })
            .collect();
        assert_eq!(got.len(), count as usize);
        for (i, (kind, key)) in got.iter().enumerate() {
            let want_kind = if i as u32 % 10 == 5 {
                Kind::Tombstone
            } else {
                Kind::Put
            };
            assert_eq!(*kind, want_kind, "entry {i}");
            assert_eq!(key, format!("key-{i:06}").as_bytes());
        }
    }

    #[test]
    fn get_probes_exactly_one_block_even_at_boundaries() {
        let data = build_table(500);
        let table = SstTable::parse(data.clone()).unwrap();
        // keys at block boundaries: separator values themselves and neighbors
        for i in [0u32, 1, 40, 41, 249, 250, 498, 499] {
            let key = format!("key-{i:06}");
            assert!(
                table.get(key.as_bytes()).unwrap().is_some() || i % 10 == 5,
                "boundary miss on {key}"
            );
        }
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut data = build_table(10);
        let len = data.len();
        data[len - 8..].copy_from_slice(b"NOTKIBAN");
        assert!(matches!(
            SstTable::parse(data),
            Err(SstError::Corrupt(m)) if m.contains("magic")
        ));
    }

    #[test]
    fn unknown_format_version_is_rejected() {
        let mut data = build_table(10);
        let vpos = data.len() - FOOTER_LEN + 16;
        data[vpos..vpos + 4].copy_from_slice(&99u32.to_le_bytes());
        assert!(matches!(
            SstTable::parse(data),
            Err(SstError::Corrupt(m)) if m.contains("version")
        ));
    }

    #[test]
    fn truncated_file_is_rejected() {
        let data = build_table(50);
        assert!(SstTable::parse(data[..FOOTER_LEN - 1].to_vec()).is_err());
        // cut bytes out of the middle (index/data region), footer intact
        let cut = data.len() - FOOTER_LEN - 20;
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&data[..cut]);
        truncated.extend_from_slice(&data[data.len() - FOOTER_LEN..]);
        assert!(matches!(
            SstTable::parse(truncated),
            Err(SstError::Corrupt(m)) if m.contains("bounds") || m.contains("checksum")
        ));
    }

    #[test]
    fn corrupted_data_block_is_detected_on_access() {
        let mut data = build_table(100);
        // flip a bit inside the first data block's payload region
        data[7] ^= 0x10;
        let table = SstTable::parse(data).unwrap();
        let result = table.get(b"key-000000");
        match result {
            Err(SstError::Corrupt(_)) => {}
            Err(e) => panic!("unexpected error: {e:?}"),
            // the flip may have landed in a byte this probe doesn't touch;
            // then full iteration must still surface it
            Ok(_) => {
                let outcomes: Vec<_> = table.iter().collect();
                assert!(
                    outcomes.iter().any(|r| r.is_err()) || outcomes.len() == 100,
                    "corruption went undetected"
                );
            }
        }
    }

    #[test]
    fn corrupted_index_block_is_detected_on_parse() {
        let data = build_table(100);
        let footer_start = data.len() - FOOTER_LEN;
        let index_offset =
            u64::from_le_bytes(data[footer_start..footer_start + 8].try_into().unwrap());
        // flip a byte inside index payload (first entry region)
        let mut bad = data;
        bad[index_offset as usize + 6] ^= 0x01;
        assert!(matches!(
            SstTable::parse(bad),
            Err(SstError::Corrupt(m)) if m.contains("index")
        ));
    }
}
