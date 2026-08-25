//! Table reader: lazy table handles with cached block reads.
//!
//! Per `docs/design/block-cache.md` D1: opening reads only footer,
//! index, and bloom filter; data blocks load on demand through the
//! shared LRU. Corruption is detected and reported, never repaired.

use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;

use super::block::{BlockIter, VerifiedBlock};
use super::builder::{FOOTER_LEN, FORMAT_VERSION, MAGIC};
use super::{Kind, SstError};
use crate::bloom::BloomFilter;
use crate::cache::{ArcBlock, BlockCache, CachedBlock};
use crate::crc32;
use std::fs::File;

struct IndexEntry {
    separator: Vec<u8>,
    offset: u64,
    len: u64,
}

pub struct SstTable {
    number: u64,
    file: File,
    file_len: u64,
    index: Vec<IndexEntry>,
    filter: BloomFilter,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    cache: Arc<BlockCache>,
}

pub struct Found {
    pub kind: Kind,
    pub value: Vec<u8>,
}

impl SstTable {
    /// Opens a handle: footer + index + filter only. Data blocks load on
    /// demand through `cache`.
    pub fn open(number: u64, path: &Path, cache: Arc<BlockCache>) -> Result<SstTable, SstError> {
        let bad = |m: String| SstError::Corrupt(m);
        let file = File::open(path)
            .map_err(|e| SstError::Corrupt(format!("table {number} cannot be opened: {e}")))?;
        let file_len = file.metadata().map_err(|e| bad(e.to_string()))?.len();
        if file_len < FOOTER_LEN as u64 {
            return Err(bad("file is smaller than the footer".to_string()));
        }

        let mut footer = vec![0u8; FOOTER_LEN];
        file.read_exact_at(&mut footer, file_len - FOOTER_LEN as u64)
            .map_err(|e| bad(e.to_string()))?;
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
        let data_end = file_len - FOOTER_LEN as u64;
        if index_offset > data_end || index_len == 0 || index_offset + index_len > data_end {
            return Err(bad("index block out of file bounds".to_string()));
        }
        if filter_offset + filter_len != index_offset || filter_len < 10 {
            return Err(bad(
                "filter block does not sit directly before the index".to_string()
            ));
        }

        let filter_raw = read_range_at(&file, filter_offset, filter_len)?;
        if filter_raw[filter_raw.len() - 5] != super::block::BLOCK_TYPE_NONE {
            return Err(bad("unknown filter block type".to_string()));
        }
        let stored_crc = u32::from_le_bytes(filter_raw[filter_raw.len() - 4..].try_into().unwrap());
        if crc32::crc32(&filter_raw[..filter_raw.len() - 4]) != stored_crc {
            return Err(bad("filter block checksum mismatch".to_string()));
        }
        let filter = BloomFilter::decode(&filter_raw[..filter_raw.len() - 5])
            .ok_or_else(|| bad("filter block payload is malformed".to_string()))?;

        let index_raw = read_range_at(&file, index_offset, index_len)?;
        let index = parse_index(&index_raw, data_end)?;

        let mut table = SstTable {
            number,
            file,
            file_len,
            index,
            filter,
            first_key: Vec::new(),
            last_key: Vec::new(),
            cache,
        };
        // Boundary keys come from the boundary blocks (one cached read each).
        // Boundary keys come from the boundary blocks (two cached reads).
        let first_block = table.read_block(&table.index[0])?;
        let (_, first, _) = first_block
            .first_entry()?
            .ok_or_else(|| bad("first block yielded no entry".to_string()))?;
        let last_block =
            table.read_block(table.index.last().expect("parse rejects empty index"))?;
        let (_, last, _) = last_block
            .last_entry()?
            .ok_or_else(|| bad("last block yielded no entry".to_string()))?;
        table.first_key = first;
        table.last_key = last;
        Ok(table)
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn size_on_disk(&self) -> u64 {
        self.file_len
    }

    pub fn smallest_key(&self) -> &[u8] {
        &self.first_key
    }

    pub fn largest_key(&self) -> &[u8] {
        &self.last_key
    }

    fn read_block(&self, entry: &IndexEntry) -> Result<VerifiedBlock, SstError> {
        let key = (self.number, entry.offset);
        if let Some(cached) = self.cache.get(&key) {
            return Ok(VerifiedBlock::from_cached(cached));
        }
        let raw = read_range_at(&self.file, entry.offset, entry.len)?;
        let meta = VerifiedBlock::verify(&raw)?;
        let cached = CachedBlock {
            data: ArcBlock::from(raw),
            meta,
        };
        self.cache.insert(key, cached.clone());
        Ok(VerifiedBlock::from_cached(cached))
    }

    fn block_index_for(&self, key: &[u8]) -> Option<usize> {
        let idx = self.index.partition_point(|e| e.separator.as_slice() < key);
        if idx == self.index.len() {
            None
        } else {
            Some(idx)
        }
    }

    /// Point lookup. Returns `Ok(None)` when the key is provably absent
    /// from this table — either by bloom filter or by probe.
    pub fn get(&self, key: &[u8]) -> Result<Option<Found>, SstError> {
        if !self.filter.may_contain(key) {
            return Ok(None);
        }
        let Some(idx) = self.block_index_for(key) else {
            return Ok(None);
        };
        let block = self.read_block(&self.index[idx])?;
        Ok(block.get(key)?.map(|m| Found {
            kind: m.kind,
            value: m.value.to_vec(),
        }))
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

    pub fn iter(&self) -> Iter<'_> {
        Iter {
            table: self,
            next_block: 0,
            current: None,
            failed: false,
            lower_bound: None,
        }
    }
}

fn read_range_at(file: &File, offset: u64, len: u64) -> Result<Vec<u8>, SstError> {
    let mut buf = vec![0u8; len as usize];
    file.read_exact_at(&mut buf, offset)
        .map_err(|e| SstError::Corrupt(format!("read failed at offset {offset}: {e}")))?;
    Ok(buf)
}

fn verify_trailer(raw: &[u8], what: &str) -> Result<(), SstError> {
    if raw.len() < 6 || raw[raw.len() - 5] != super::block::BLOCK_TYPE_NONE {
        return Err(SstError::Corrupt(format!("unknown {what} block type")));
    }
    let stored_crc = u32::from_le_bytes(raw[raw.len() - 4..].try_into().unwrap());
    if crc32::crc32(&raw[..raw.len() - 4]) != stored_crc {
        return Err(SstError::Corrupt(format!("{what} block checksum mismatch")));
    }
    Ok(())
}

fn parse_index(index_raw: &[u8], data_end: u64) -> Result<Vec<IndexEntry>, SstError> {
    let bad = |m: String| SstError::Corrupt(m);
    verify_trailer(index_raw, "index")?;
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
    Ok(index)
}

pub struct Iter<'a> {
    table: &'a SstTable,
    next_block: usize,
    current: Option<BlockIter>,
    failed: bool,
    lower_bound: Option<Vec<u8>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<(Kind, Vec<u8>, Vec<u8>), SstError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(state) = &mut self.current {
                match state.next() {
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
            match self.table.read_block(&self.table.index[self.next_block]) {
                Ok(block) => {
                    self.current = Some(BlockIter::from_verified(block));
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
