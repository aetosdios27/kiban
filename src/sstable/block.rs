//! Data-block encoding: prefix-compressed entries anchored by restart
//! points, per `docs/design/sstable.md`.

use super::{Kind, SstError, common_prefix_len};
use crate::cache::{ArcBlock, BlockMeta, CachedBlock};
use crate::crc32;

pub const RESTART_INTERVAL: usize = 16;
pub const BLOCK_TYPE_NONE: u8 = 0x00;
const ENTRY_FIXED_LEN: usize = 13;
const TRAILER_LEN: usize = 5;

#[derive(Debug)]
pub struct BlockBuilder {
    buf: Vec<u8>,
    restarts: Vec<u32>,
    counter: usize,
    last_key: Vec<u8>,
    entries: usize,
}

impl Default for BlockBuilder {
    fn default() -> Self {
        BlockBuilder {
            buf: Vec::new(),
            restarts: vec![0],
            counter: 0,
            last_key: Vec::new(),
            entries: 0,
        }
    }
}

impl BlockBuilder {
    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    pub fn estimated_size(&self) -> usize {
        self.buf.len()
    }

    pub fn last_key(&self) -> &[u8] {
        &self.last_key
    }

    pub fn add(&mut self, kind: Kind, key: &[u8], value: &[u8]) {
        debug_assert!(
            self.entries == 0 || self.last_key.as_slice() < key,
            "block keys must strictly increase"
        );
        debug_assert!(kind == Kind::Put || value.is_empty());

        let shared = if self.counter == RESTART_INTERVAL {
            self.restarts.push(self.buf.len() as u32);
            self.counter = 0;
            0
        } else if self.counter == 0 {
            0
        } else {
            common_prefix_len(&self.last_key, key)
        };
        let non_shared = key.len() - shared;

        self.buf.push(kind.to_u8());
        self.buf.extend_from_slice(&(shared as u32).to_le_bytes());
        self.buf
            .extend_from_slice(&(non_shared as u32).to_le_bytes());
        self.buf
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(&key[shared..]);
        if kind == Kind::Put {
            self.buf.extend_from_slice(value);
        }

        self.last_key.clear();
        self.last_key.extend_from_slice(key);
        self.counter += 1;
        self.entries += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        debug_assert!(self.entries > 0);
        for r in &self.restarts {
            self.buf.extend_from_slice(&r.to_le_bytes());
        }
        self.buf
            .extend_from_slice(&(self.restarts.len() as u32).to_le_bytes());
        self.buf.push(BLOCK_TYPE_NONE);
        let crc = crc32::crc32(&self.buf);
        self.buf.extend_from_slice(&crc.to_le_bytes());
        self.buf
    }
}

pub struct Match<'a> {
    pub kind: Kind,
    pub value: &'a [u8],
}

/// A data block whose bytes are owned and whose layout was verified.
#[derive(Debug, Clone)]
pub struct VerifiedBlock {
    data: ArcBlock,
    meta: BlockMeta,
}

impl VerifiedBlock {
    /// Verifies trailer checksum and structural invariants.
    pub fn verify(raw: &[u8]) -> Result<BlockMeta, SstError> {
        let bad = |m: String| SstError::Corrupt(format!("data block: {m}"));
        if raw.len() < ENTRY_FIXED_LEN + 4 + 4 + TRAILER_LEN {
            return Err(bad(
                "shorter than one minimum entry plus trailers".to_string()
            ));
        }
        let block_type = raw[raw.len() - 5];
        if block_type != BLOCK_TYPE_NONE {
            return Err(bad(format!("unknown block type {block_type:#04x}")));
        }
        let stored_crc = u32::from_le_bytes(raw[raw.len() - 4..].try_into().unwrap());
        let actual_crc = crc32::crc32(&raw[..raw.len() - 4]);
        if stored_crc != actual_crc {
            return Err(bad(format!(
                "checksum mismatch (stored {stored_crc:#010x}, computed {actual_crc:#010x})"
            )));
        }
        let num_restarts =
            u32::from_le_bytes(raw[raw.len() - 9..raw.len() - 5].try_into().unwrap()) as usize;
        if num_restarts == 0 {
            return Err(bad("zero restart points".to_string()));
        }
        let restart_start = raw.len() - 9 - num_restarts * 4;
        let entries_end = restart_start;
        if entries_end < ENTRY_FIXED_LEN {
            return Err(bad("no room for a single entry".to_string()));
        }
        for i in 0..num_restarts {
            let off = restart_offset_of(raw, restart_start, i);
            if off >= entries_end || (i > 0 && off <= restart_offset_of(raw, restart_start, i - 1))
            {
                return Err(bad(
                    "restart offsets not strictly ascending in range".to_string()
                ));
            }
        }
        if restart_offset_of(raw, restart_start, 0) != 0 {
            return Err(bad("first restart point is not at offset 0".to_string()));
        }
        Ok(BlockMeta {
            entries_end,
            restart_start,
            num_restarts,
        })
    }

    pub fn from_raw(raw: Vec<u8>) -> Result<VerifiedBlock, SstError> {
        let meta = Self::verify(&raw)?;
        Ok(VerifiedBlock {
            data: raw.into(),
            meta,
        })
    }

    pub fn from_cached(cached: CachedBlock) -> VerifiedBlock {
        VerifiedBlock {
            data: cached.data,
            meta: cached.meta,
        }
    }

    fn restart_offset(&self, i: usize) -> usize {
        restart_offset_of(&self.data, self.meta.restart_start, i)
    }

    fn header(&self, pos: usize) -> Result<(Kind, usize, usize, usize), SstError> {
        header_at(&self.data, &self.meta, pos)
    }

    fn full_key_at_restart(&self, i: usize) -> Result<Vec<u8>, SstError> {
        full_key_at_restart(&self.data, &self.meta, i)
    }

    pub fn get(&self, target: &[u8]) -> Result<Option<Match<'_>>, SstError> {
        let mut lo = 0usize;
        let mut hi = self.meta.num_restarts;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.full_key_at_restart(mid)?.as_slice() <= target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let start = lo.saturating_sub(1);
        let mut key: Vec<u8> = Vec::new();
        let mut pos = self.restart_offset(start);

        loop {
            if pos >= self.meta.entries_end {
                return Ok(None);
            }
            let (kind, shared, non_shared, value_len) = self.header(pos)?;
            key.truncate(shared);
            key.extend_from_slice(
                &self.data[pos + ENTRY_FIXED_LEN..pos + ENTRY_FIXED_LEN + non_shared],
            );
            let value = entry_value(&self.data, &self.meta, pos, kind, non_shared, value_len);
            if key.as_slice() == target {
                return Ok(Some(Match { kind, value }));
            }
            if key.as_slice() > target {
                return Ok(None);
            }
            pos += ENTRY_FIXED_LEN + non_shared + value_len;
        }
    }

    /// First entry in the block.
    pub fn first_entry(&self) -> Result<Option<Entry>, SstError> {
        self.iter().next().transpose()
    }

    /// Last entry in the block.
    pub fn last_entry(&self) -> Result<Option<Entry>, SstError> {
        // decode forward to the final entry; prefix compression makes a
        // true backwards walk start from the previous restart anyway
        let mut last: Option<Entry> = None;
        for item in BlockIter::from_verified(self.clone()) {
            last = Some(item?);
        }
        Ok(last)
    }

    pub fn iter(&self) -> BlockIter {
        BlockIter::from_verified(self.clone())
    }
}

fn header_at(
    data: &[u8],
    meta: &BlockMeta,
    pos: usize,
) -> Result<(Kind, usize, usize, usize), SstError> {
    let bad = |m: String| SstError::Corrupt(format!("data block entry at {pos}: {m}"));
    if pos + ENTRY_FIXED_LEN > meta.entries_end {
        return Err(bad("header runs past entries area".to_string()));
    }
    let kind = Kind::from_u8(data[pos]).ok_or_else(|| bad("unknown entry kind".to_string()))?;
    let shared = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap()) as usize;
    let non_shared = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
    let value_len = u32::from_le_bytes(data[pos + 9..pos + 13].try_into().unwrap()) as usize;
    if kind == Kind::Tombstone && value_len != 0 {
        return Err(bad("tombstone with non-zero value length".to_string()));
    }
    let end = pos + ENTRY_FIXED_LEN + non_shared + value_len;
    if end > meta.entries_end {
        return Err(bad("key/value runs past entries area".to_string()));
    }
    Ok((kind, shared, non_shared, value_len))
}

fn restart_offset_of(data: &[u8], restart_start: usize, i: usize) -> usize {
    u32::from_le_bytes(
        data[restart_start + i * 4..restart_start + (i + 1) * 4]
            .try_into()
            .unwrap(),
    ) as usize
}

fn full_key_at_restart(data: &[u8], meta: &BlockMeta, i: usize) -> Result<Vec<u8>, SstError> {
    let pos = restart_offset_of(data, meta.restart_start, i);
    let (_, shared, non_shared, _) = header_at(data, meta, pos)?;
    if shared != 0 {
        return Err(SstError::Corrupt(
            "restart point entry has shared bytes".to_string(),
        ));
    }
    Ok(data[pos + ENTRY_FIXED_LEN..pos + ENTRY_FIXED_LEN + non_shared].to_vec())
}

fn entry_value<'a>(
    data: &'a [u8],
    _meta: &BlockMeta,
    pos: usize,
    kind: Kind,
    non_shared: usize,
    value_len: usize,
) -> &'a [u8] {
    match kind {
        Kind::Put => {
            let vstart = pos + ENTRY_FIXED_LEN + non_shared;
            &data[vstart..vstart + value_len]
        }
        Kind::Tombstone => &[],
    }
}

/// One owned entry: (kind, key, value).
pub type Entry = (Kind, Vec<u8>, Vec<u8>);

/// Owning iterator over a verified block: yields fully-owned entries so
/// callers never borrow from cache-managed bytes.
pub struct BlockIter {
    block: VerifiedBlock,
    pos: usize,
    key: Vec<u8>,
    failed: bool,
}

impl BlockIter {
    pub(crate) fn from_verified(block: VerifiedBlock) -> BlockIter {
        BlockIter {
            block,
            pos: 0,
            key: Vec::new(),
            failed: false,
        }
    }
}

impl Iterator for BlockIter {
    type Item = Result<(Kind, Vec<u8>, Vec<u8>), SstError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.pos >= self.block.meta.entries_end {
            return None;
        }
        let data = &self.block.data;
        let meta = &self.block.meta;
        let pos = self.pos;
        let bad = |m: String| SstError::Corrupt(format!("data block entry at {pos}: {m}"));
        let kind = match Kind::from_u8(data[pos]) {
            Some(k) => k,
            None => {
                self.failed = true;
                return Some(Err(bad("unknown entry kind".to_string())));
            }
        };
        let shared = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap()) as usize;
        let non_shared = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
        let value_len = u32::from_le_bytes(data[pos + 9..pos + 13].try_into().unwrap()) as usize;
        let end = pos + ENTRY_FIXED_LEN + non_shared + value_len;
        if end > meta.entries_end || (kind == Kind::Tombstone && value_len != 0) {
            self.failed = true;
            return Some(Err(bad("entry runs past entries area".to_string())));
        }
        if shared > self.key.len() {
            self.failed = true;
            return Some(Err(bad("shared length exceeds previous key".to_string())));
        }
        self.key.truncate(shared);
        self.key
            .extend_from_slice(&data[pos + ENTRY_FIXED_LEN..pos + ENTRY_FIXED_LEN + non_shared]);
        let value = if kind == Kind::Put {
            data[end - value_len..end].to_vec()
        } else {
            Vec::new()
        };
        self.pos = end;
        Some(Ok((kind, self.key.clone(), value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut b = BlockBuilder::default();
        for (k, v) in entries {
            b.add(Kind::Put, k.as_bytes(), v.as_bytes());
        }
        b.finish()
    }

    #[test]
    fn empty_block_builder_produces_nothing() {
        assert!(BlockBuilder::default().is_empty());
    }

    #[test]
    fn roundtrip_small_block() {
        let raw = build(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let block = VerifiedBlock::from_raw(raw.clone()).unwrap();
        assert_eq!(block.get(b"a").unwrap().unwrap().value, b"1");
        assert_eq!(block.get(b"b").unwrap().unwrap().value, b"2");
        assert_eq!(block.get(b"c").unwrap().unwrap().value, b"3");
        assert!(block.get(b"d").unwrap().is_none());
        assert!(block.get(b"aa").unwrap().is_none());
        assert!(block.get(b"").unwrap().is_none());
    }

    #[test]
    fn restart_points_exist_every_interval_entries() {
        let mut b = BlockBuilder::default();
        for i in 0..40 {
            b.add(Kind::Put, format!("key-{i:04}").as_bytes(), b"v");
        }
        let raw = b.finish();
        let n_restarts = u32::from_le_bytes(raw[raw.len() - 9..raw.len() - 5].try_into().unwrap());
        // entries 0,16,32 -> 3 restarts
        assert_eq!(n_restarts, 3);
        let block = VerifiedBlock::from_raw(raw.clone()).unwrap();
        for i in 0..40 {
            let key = format!("key-{i:04}");
            assert_eq!(
                block.get(key.as_bytes()).unwrap().unwrap().value,
                b"v",
                "missed {key}"
            );
        }
    }

    #[test]
    fn prefix_compression_actually_compresses() {
        let keys: Vec<String> = (0..100)
            .map(|i| format!("user:database:table:{i:06}:row"))
            .collect();
        let mut b = BlockBuilder::default();
        for k in &keys {
            b.add(Kind::Put, k.as_bytes(), b"x");
        }
        let compressed_len = b.estimated_size();
        let full_len: usize = keys.iter().map(|k| k.len()).sum::<usize>() + 100 * 14;
        assert!(
            compressed_len < full_len / 2,
            "compression saved less than half: {compressed_len} vs {full_len}"
        );
    }

    #[test]
    fn tombstones_roundtrip_and_lookup_as_tombstones() {
        let mut b = BlockBuilder::default();
        b.add(Kind::Put, b"alive", b"v");
        b.add(Kind::Tombstone, b"dead", b"");
        let raw = b.finish();
        let block = VerifiedBlock::from_raw(raw.clone()).unwrap();
        let dead = block.get(b"dead").unwrap().unwrap();
        assert_eq!(dead.kind, Kind::Tombstone);
        assert_eq!(dead.value, b"");
        let alive = block.get(b"alive").unwrap().unwrap();
        assert_eq!(alive.kind, Kind::Put);
    }

    #[test]
    fn iteration_yields_all_entries_in_order_with_full_keys() {
        let mut b = BlockBuilder::default();
        for i in 0..50u32 {
            b.add(Kind::Put, format!("k{:03}", i * 7).as_bytes(), b"v");
        }
        let raw = b.finish();
        let block = VerifiedBlock::from_raw(raw.clone()).unwrap();
        let got: Vec<String> = block
            .iter()
            .map(|r| r.unwrap().1)
            .map(|k| String::from_utf8(k).unwrap())
            .collect();
        let want: Vec<String> = (0..50u32).map(|i| format!("k{:03}", i * 7)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn any_single_byte_corruption_is_detected_on_parse_or_read() {
        let raw = build(&[
            ("key-000", "value-000"),
            ("key-001", "value-001"),
            ("key-002", "value-002"),
        ]);
        for i in 0..raw.len() {
            let mut bad = raw.clone();
            bad[i] ^= 0x01;
            match VerifiedBlock::from_raw(bad.clone()) {
                Err(SstError::Corrupt(_)) => {}
                Ok(block) => {
                    // a flip that survives parse must still not return wrong data
                    let probe = ["key-000", "key-001", "key-002"]
                        .iter()
                        .all(|k| matches!(block.get(k.as_bytes()), Ok(None) | Err(_)))
                        || std::panic::catch_unwind(|| {
                            let _ = block.get(b"key-001").unwrap();
                        })
                        .is_err();
                    assert!(probe, "flip at byte {i} went undetected");
                }
                Err(e) => panic!("unexpected error at byte {i}: {e:?}"),
            }
        }
    }

    #[test]
    fn truncated_block_is_rejected() {
        let raw = build(&[("k", "v")]);
        for cut in [0, 5, 13, raw.len() / 2] {
            assert!(
                VerifiedBlock::from_raw(raw[..cut].to_vec()).is_err(),
                "cut={cut} accepted"
            );
        }
    }
}
