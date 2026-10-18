//! KIBANPH3: anchored key suffixes and compact MVCC locators.
use super::*;
use crate::cache::{ArcBlock, BlockMeta};
use std::sync::Arc;

pub(in crate::sstable) struct ValueLocator {
    pub kind: Kind,
    pub seq: u64,
    pub offset: usize,
    pub len: usize,
    pub checksum: u32,
}

pub(super) const MAGIC: &[u8; 8] = b"KIBANPH3";
const HEADER: usize = 64;
const CHECKSUM_FLAG: u32 = 1;
const SCAN_FLAG: u32 = 2;
const SCAN_MAGIC: &[u8; 8] = b"KIBANSC1";
const SCAN_HEADER: usize = 24;
pub(super) const GROUP: usize = 1024;

#[inline(always)]
fn uint(raw: &[u8], at: usize, width: usize) -> u64 {
    match width {
        1 => raw[at] as u64,
        2 => u16::from_le_bytes(raw[at..at + 2].try_into().unwrap()) as u64,
        4 => u32_at(raw, at) as u64,
        8 => u64_at(raw, at),
        _ => unreachable!("validated integer width"),
    }
}
fn put_uint(bytes: &mut Vec<u8>, value: u64, width: usize) {
    bytes.extend_from_slice(&value.to_le_bytes()[..width]);
}
fn width(max: u64) -> usize {
    if max <= u8::MAX as u64 {
        1
    } else if max <= u16::MAX as u64 {
        2
    } else if max <= u32::MAX as u64 {
        4
    } else {
        8
    }
}

#[derive(Clone, Copy)]
pub(super) struct Encoding {
    address: usize,
    key: usize,
    count: usize,
    seq: usize,
    value: usize,
    checksum: bool,
}
impl Encoding {
    pub(super) fn for_builder(b: &PointBuilder) -> Self {
        let mut result = Self {
            address: 4,
            checksum: true,
            key: width(b.max_key_len as u64),
            count: width(b.max_versions as u64),
            seq: width(b.max_seq - b.min_seq),
            value: if b.max_value_len < 1 << 15 { 2 } else { 4 },
        };
        if result.arena_len(b) > u32::MAX as usize || b.last_value_offset > u32::MAX as u64 {
            result.address = 8;
        }
        result
    }
    fn key_header(self) -> usize {
        self.address + 2 * self.key + self.count
    }
    fn version(self) -> usize {
        self.seq + self.value + self.address + usize::from(self.checksum) * 4
    }
    fn arena_len(self, b: &PointBuilder) -> usize {
        b.heads.len() * self.key_header() + b.suffix_bytes + b.versions * self.version()
    }
    pub(super) fn section_len(self, b: &PointBuilder) -> usize {
        HEADER
            + b.heads.len().div_ceil(2) * 4
            + b.heads.len() * (self.address + 2)
            + self.arena_len(b)
            + if b.scan_prefix_len > 0 {
                SCAN_HEADER + b.heads.len() * self.address + 8
            } else {
                0
            }
            + 5
    }
    fn tombstone(self) -> u64 {
        1 << (self.value * 8 - 1)
    }
}

pub(super) fn encode(builder: PointBuilder) -> Result<Vec<u8>, SstError> {
    if builder.scan_prefix_len > 24 {
        return Err(SstError::InvalidArgument(
            "scan prefix length exceeds 24".into(),
        ));
    }
    let encoding = Encoding::for_builder(&builder);
    encode_with(builder, encoding)
}
#[cfg(test)]
pub(super) fn encode_without_crc(builder: PointBuilder) -> Result<Vec<u8>, SstError> {
    let mut encoding = Encoding::for_builder(&builder);
    encoding.checksum = false;
    encode_with(builder, encoding)
}
fn encode_with(builder: PointBuilder, encoding: Encoding) -> Result<Vec<u8>, SstError> {
    let keys: Vec<_> = builder
        .heads
        .iter()
        .map(|&p| &builder.arena[p + 8..p + 8 + u32_at(&builder.arena, p) as usize])
        .collect();
    let (seed, pilots, slots) = construct_with_bucket_size(&keys, 2)?;
    let mut arena = Vec::with_capacity(encoding.arena_len(&builder));
    let mut heads = Vec::with_capacity(keys.len());
    let mut anchor = 0;
    let mut checksums = builder.checksums.iter();
    for (i, &key) in keys.iter().enumerate() {
        let shared = if i.is_multiple_of(GROUP) {
            0
        } else {
            super::super::common_prefix_len(keys[i / GROUP * GROUP], key)
        };
        let versions = u32_at(&builder.arena, builder.heads[i] + 4);
        heads.push(arena.len());
        if i.is_multiple_of(GROUP) {
            anchor = arena.len() + encoding.key_header();
        }
        put_uint(&mut arena, anchor as u64, encoding.address);
        put_uint(&mut arena, shared as u64, encoding.key);
        put_uint(&mut arena, (key.len() - shared) as u64, encoding.key);
        put_uint(&mut arena, versions as u64, encoding.count);
        arena.extend_from_slice(&key[shared..]);
        let mut pos = builder.heads[i] + 8 + key.len();
        for _ in 0..versions {
            let seq = u64_at(&builder.arena, pos);
            let kind = Kind::from_u8(builder.arena[pos + 8]).expect("builder kind");
            let len = u32_at(&builder.arena, pos + 9) as u64;
            let value = len
                | if kind == Kind::Tombstone {
                    encoding.tombstone()
                } else {
                    0
                };
            put_uint(&mut arena, seq - builder.min_seq, encoding.seq);
            put_uint(&mut arena, value, encoding.value);
            put_uint(
                &mut arena,
                u64_at(&builder.arena, pos + 13),
                encoding.address,
            );
            let checksum = *checksums.next().expect("one CRC per version");
            if encoding.checksum {
                arena.extend_from_slice(&checksum.to_le_bytes());
            }
            pos += 21;
        }
    }
    let arena_start = HEADER + pilots.len() * 4 + slots.len() * (encoding.address + 2);
    let mut bytes = Vec::with_capacity(encoding.section_len(&builder));
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&seed.to_le_bytes());
    bytes.extend_from_slice(&(slots.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(pilots.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(arena_start as u64).to_le_bytes());
    for w in [
        encoding.address,
        encoding.key,
        encoding.count,
        encoding.seq,
        encoding.value,
        (if encoding.checksum {
            CHECKSUM_FLAG as usize
        } else {
            0
        }) | if builder.scan_prefix_len > 0 {
            SCAN_FLAG as usize
        } else {
            0
        },
    ] {
        bytes.extend_from_slice(&(w as u32).to_le_bytes());
    }
    bytes.extend_from_slice(&builder.min_seq.to_le_bytes());
    for pilot in pilots {
        bytes.extend_from_slice(&pilot.to_le_bytes());
    }
    for member in slots {
        put_uint(&mut bytes, heads[member] as u64, encoding.address);
        bytes.extend_from_slice(&fingerprint(hash(keys[member], seed)).to_le_bytes());
    }
    bytes.extend_from_slice(&arena);
    if builder.scan_prefix_len > 0 {
        let start = bytes.len();
        bytes.extend_from_slice(SCAN_MAGIC);
        bytes.extend_from_slice(&(builder.scan_prefix_len as u32).to_le_bytes());
        bytes.extend_from_slice(&32u32.to_le_bytes());
        let prefixes = keys
            .iter()
            .enumerate()
            .filter(|&(i, key)| {
                i == 0
                    || key[..key.len().min(builder.scan_prefix_len)]
                        != keys[i - 1][..keys[i - 1].len().min(builder.scan_prefix_len)]
            })
            .count();
        bytes.extend_from_slice(&(prefixes as u64).to_le_bytes());
        for &head in &heads {
            put_uint(&mut bytes, head as u64, encoding.address);
        }
        let len = bytes.len() - start;
        bytes.extend_from_slice(&(len as u64).to_le_bytes());
    }
    bytes.push(0);
    let crc = crate::crc32::crc32(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    Ok(bytes)
}

pub(crate) struct CompactIndex {
    bytes: ArcBlock,
    data: ArcBlock,
    blocks: Vec<(usize, usize, BlockMeta)>,
    seed: u64,
    min_seq: u64,
    count: usize,
    buckets: usize,
    arena: usize,
    encoding: Encoding,
    scan: usize,
    packed: bool,
    _reservation: ResidentReservation,
}
impl CompactIndex {
    pub(super) fn decode(
        bytes: ArcBlock,
        data: ArcBlock,
        reservation: ResidentReservation,
        blocks: impl IntoIterator<Item = (u64, u64)>,
    ) -> Result<Self, SstError> {
        let mut result = Self::header(bytes, data, reservation)?;
        let data = result.data.clone();
        result.validate(blocks, data.len(), |offset, len| {
            Ok(std::borrow::Cow::Borrowed(&data[offset..offset + len]))
        })?;
        Ok(result)
    }
    fn header(
        bytes: ArcBlock,
        data: ArcBlock,
        reservation: ResidentReservation,
    ) -> Result<Self, SstError> {
        super::super::reader::verify_trailer(&bytes, "point index")?;
        if bytes.len() < HEADER + 5 || &bytes[..8] != MAGIC {
            return Err(bad("unsupported compact header"));
        }
        let seed = u64_at(&bytes, 8);
        let count = u32_at(&bytes, 16) as usize;
        let buckets = u32_at(&bytes, 20) as usize;
        let arena =
            usize::try_from(u64_at(&bytes, 24)).map_err(|_| bad("arena offset overflow"))?;
        let encoding = Encoding {
            address: u32_at(&bytes, 32) as usize,
            key: u32_at(&bytes, 36) as usize,
            count: u32_at(&bytes, 40) as usize,
            seq: u32_at(&bytes, 44) as usize,
            value: u32_at(&bytes, 48) as usize,
            checksum: u32_at(&bytes, 52) & CHECKSUM_FLAG != 0,
        };
        let min_seq = u64_at(&bytes, 56);
        if count == 0
            || count >= DIRECT as usize
            || buckets != count.div_ceil(2)
            || !matches!(encoding.address, 4 | 8)
            || !matches!(encoding.key, 1 | 2 | 4)
            || !matches!(encoding.count, 1 | 2 | 4)
            || !matches!(encoding.seq, 1 | 2 | 4 | 8)
            || !matches!(encoding.value, 2 | 4)
            || u32_at(&bytes, 52) & !(CHECKSUM_FLAG | SCAN_FLAG) != 0
            || buckets
                .checked_mul(4)
                .and_then(|n| HEADER.checked_add(n))
                .and_then(|n| {
                    count
                        .checked_mul(encoding.address + 2)
                        .and_then(|size| n.checked_add(size))
                })
                != Some(arena)
            || arena >= bytes.len() - 5
        {
            return Err(bad("invalid compact dimensions"));
        }
        let arena_end = bytes.len() - 5;
        let scan = if u32_at(&bytes, 52) & SCAN_FLAG != 0 {
            if arena_end < 8 {
                return Err(bad("truncated scan length"));
            }
            let len = usize::try_from(u64_at(&bytes, arena_end - 8))
                .map_err(|_| bad("scan length overflow"))?;
            let start = (arena_end - 8)
                .checked_sub(len)
                .filter(|&p| p > arena)
                .ok_or_else(|| bad("scan bounds"))?;
            if len != SCAN_HEADER + count * encoding.address
                || &bytes[start..start + 8] != SCAN_MAGIC
            {
                return Err(bad("unsupported scan dimensions"));
            }
            let prefix_len = u32_at(&bytes, start + 8) as usize;
            let prefixes = usize::try_from(u64_at(&bytes, start + 16))
                .map_err(|_| bad("scan prefix count overflow"))?;
            if !(1..=24).contains(&prefix_len)
                || u32_at(&bytes, start + 12) != 32
                || prefixes == 0
                || prefixes > count
            {
                return Err(bad("unsupported scan extractor or stride"));
            }
            start
        } else {
            0
        };
        Ok(Self {
            bytes,
            data,
            blocks: Vec::new(),
            seed,
            min_seq,
            count,
            buckets,
            arena,
            encoding,
            scan,
            packed: false,
            _reservation: reservation,
        })
    }
    // PH3's disk encoding is unchanged. Only its validated in-memory hash tables
    // omit fingerprints (full key equality remains mandatory) and use u24 pilots.
    pub(in crate::sstable) fn locator_bytes(
        header: &[u8],
        len: u64,
        data_len: u64,
    ) -> Option<usize> {
        if header.len() < HEADER
            || &header[..8] != MAGIC
            || u32_at(header, 52) & CHECKSUM_FLAG == 0
            || u32_at(header, 52) & !(CHECKSUM_FLAG | SCAN_FLAG) != 0
            || usize::try_from(data_len).is_err()
        {
            return None;
        }
        let n = u32_at(header, 16) as u64;
        let b = u32_at(header, 20) as u64;
        if n >= 1 << 23 || b != n.div_ceil(2) {
            return None;
        }
        let scan = if u32_at(header, 52) & SCAN_FLAG != 0 {
            SCAN_HEADER as u64 + 8 + n * u32_at(header, 32) as u64
        } else {
            0
        };
        usize::try_from(len.checked_sub(b + 2 * n + scan)?).ok()
    }
    pub(in crate::sstable) fn decode_locators<'a>(
        bytes: ArcBlock,
        data_len: usize,
        reservation: ResidentReservation,
        blocks: impl IntoIterator<Item = (u64, u64)>,
        read: impl FnMut(usize, usize) -> Result<std::borrow::Cow<'a, [u8]>, SstError>,
    ) -> Result<Option<Self>, SstError> {
        let mut result = Self::header(bytes, Arc::from([]), reservation)?;
        if !result.encoding.checksum || result.count >= 1 << 23 {
            return Err(bad("unsupported locator admission"));
        }
        if (0..result.buckets).any(|b| u32_at(&result.bytes, HEADER + b * 4) & !DIRECT >= 1 << 23) {
            return Ok(None);
        }
        result.validate(blocks, data_len, read)?;
        let mut packed = Vec::with_capacity(result.bytes.len() - result.buckets - 2 * result.count);
        packed.extend_from_slice(&result.bytes[..HEADER]);
        for b in 0..result.buckets {
            let p = u32_at(&result.bytes, HEADER + b * 4);
            let p = (p & !DIRECT) | ((p & DIRECT) >> 8);
            packed.extend_from_slice(&p.to_le_bytes()[..3]);
        }
        let slots = HEADER + result.buckets * 4;
        for i in 0..result.count {
            let at = slots + i * (result.encoding.address + 2);
            packed.extend_from_slice(&result.bytes[at..at + result.encoding.address]);
        }
        let arena = packed.len();
        // Locator mode never iterates keys, so the scan extension is dropped.
        let arena_end = if result.scan == 0 {
            result.bytes.len() - 5
        } else {
            result.scan
        };
        packed.extend_from_slice(&result.bytes[result.arena..arena_end]);
        packed.extend_from_slice(&result.bytes[result.bytes.len() - 5..]);
        result.scan = 0;
        result.arena = arena;
        result.bytes = packed.into();
        result.blocks.clear();
        result.blocks.shrink_to_fit();
        result.packed = true;
        Ok(Some(result))
    }
    #[inline(always)]
    fn slot(&self, h: u64) -> usize {
        let pilot = if self.packed {
            let at = HEADER + bucket(h, self.buckets) * 3;
            let p = u32::from_le_bytes([self.bytes[at], self.bytes[at + 1], self.bytes[at + 2], 0]);
            (p & 0x7f_ffff) | ((p & 0x80_0000) << 8)
        } else {
            u32_at(&self.bytes, HEADER + bucket(h, self.buckets) * 4)
        };
        let at = if pilot & DIRECT != 0 {
            (pilot & !DIRECT) as usize
        } else {
            position(h, pilot, self.count)
        };
        if self.packed {
            HEADER + self.buckets * 3 + at * self.encoding.address
        } else {
            HEADER + self.buckets * 4 + at * (self.encoding.address + 2)
        }
    }

    fn validate<'a>(
        &mut self,
        blocks: impl IntoIterator<Item = (u64, u64)>,
        data_len: usize,
        mut read: impl FnMut(usize, usize) -> Result<std::borrow::Cow<'a, [u8]>, SstError>,
    ) -> Result<(), SstError> {
        let e = self.encoding;
        let end = if self.scan == 0 {
            self.bytes.len() - 5
        } else {
            self.scan
        };
        for b in 0..self.buckets {
            let pilot = u32_at(&self.bytes, HEADER + b * 4);
            if pilot & DIRECT != 0 && (pilot & !DIRECT) as usize >= self.count {
                return Err(bad("compact singleton slot out of bounds"));
            }
        }
        let mut pos = self.arena;
        let mut groups = 0usize;
        let mut remaining = 0usize;
        let mut prefix = &[][..];
        let mut suffix = &[][..];
        let mut anchor = 0;
        let mut anchor_len = 0;
        let mut previous = Vec::new();
        let mut scan_prefixes = 0;
        let mut last_seq = None;
        let mut key = Vec::new();
        let mut data_end = 0;
        let mut verified_blocks = Vec::new();
        for (offset, len) in blocks {
            let offset = usize::try_from(offset).map_err(|_| bad("data offset overflow"))?;
            let len = usize::try_from(len).map_err(|_| bad("data length overflow"))?;
            if offset != data_end || offset.checked_add(len).is_none_or(|end| end > data_len) {
                return Err(bad("invalid compact backing bounds"));
            }
            let raw = read(offset, len)?;
            if raw.len() != len {
                return Err(bad("short backing read"));
            }
            let meta = super::super::block::VerifiedBlock::verify(&raw)?;
            super::super::block::visit_records(
                &raw,
                &meta,
                &mut key,
                |kind, seq, key, value_offset, value_len| {
                    if key.len() > crate::limits::MAX_KEY_BYTES
                        || value_len > crate::limits::MAX_VALUE_BYTES
                    {
                        return Err(bad("compact backing key/value limit"));
                    }
                    if remaining == 0 {
                        let start = pos;
                        if groups == self.count || end.saturating_sub(pos) < e.key_header() {
                            return Err(bad("truncated compact key header"));
                        }
                        let stored_anchor = uint(&self.bytes, pos, e.address);
                        let shared = uint(&self.bytes, pos + e.address, e.key) as usize;
                        let non_shared = uint(&self.bytes, pos + e.address + e.key, e.key) as usize;
                        remaining =
                            uint(&self.bytes, pos + e.address + 2 * e.key, e.count) as usize;
                        pos += e.key_header();
                        if groups.is_multiple_of(GROUP) {
                            anchor = pos - self.arena;
                            anchor_len = non_shared;
                            if shared != 0 {
                                return Err(bad("compact anchor has a prefix"));
                            }
                        }
                        if stored_anchor != anchor as u64
                            || shared > anchor_len
                            || non_shared > end - pos
                            || remaining == 0
                            || shared
                                .checked_add(non_shared)
                                .is_none_or(|n| n > crate::limits::MAX_KEY_BYTES)
                        {
                            return Err(bad("invalid compact key dimensions"));
                        }
                        prefix = &self.bytes[self.arena + anchor..self.arena + anchor + shared];
                        suffix = &self.bytes[pos..pos + non_shared];
                        pos += non_shared;
                        if remaining
                            .checked_mul(e.version())
                            .is_none_or(|n| n > end - pos)
                        {
                            return Err(bad("truncated compact versions"));
                        }
                        if groups != 0 && previous.as_slice() >= key {
                            return Err(bad("compact key ordering"));
                        }
                        if self.scan != 0 {
                            let prefix_len = u32_at(&self.bytes, self.scan + 8) as usize;
                            if uint(
                                &self.bytes,
                                self.scan + SCAN_HEADER + groups * e.address,
                                e.address,
                            ) != (start - self.arena) as u64
                            {
                                return Err(bad("scan locator disagrees with PH3 arena"));
                            }
                            if groups == 0
                                || key[..key.len().min(prefix_len)]
                                    != previous[..previous.len().min(prefix_len)]
                            {
                                scan_prefixes += 1;
                            }
                        }
                        previous.clear();
                        previous.extend_from_slice(key);
                        let h = hash(key, self.seed);
                        let slot = self.slot(h);
                        if uint(&self.bytes, slot, e.address) != (start - self.arena) as u64
                            || uint(&self.bytes, slot + e.address, 2) != fingerprint(h) as u64
                        {
                            return Err(bad("compact slot does not identify record"));
                        }
                        groups += 1;
                        last_seq = None;
                    }
                    let stored_seq = self.min_seq.checked_add(uint(&self.bytes, pos, e.seq));
                    let stored_value = uint(&self.bytes, pos + e.seq, e.value);
                    let stored_kind = if stored_value & e.tombstone() != 0 {
                        Kind::Tombstone
                    } else {
                        Kind::Put
                    };
                    if key.len() != prefix.len() + suffix.len()
                        || !key.starts_with(prefix)
                        || &key[prefix.len()..] != suffix
                        || last_seq.is_some_and(|last| last <= seq)
                        || stored_seq != Some(seq)
                        || stored_kind != kind
                        || stored_value & !e.tombstone() != value_len as u64
                        || uint(&self.bytes, pos + e.seq + e.value, e.address)
                            != (offset + value_offset) as u64
                    {
                        return Err(bad("compact locator disagrees with sorted record"));
                    }
                    if e.checksum
                        && u32_at(&self.bytes, pos + e.seq + e.value + e.address)
                            != crate::crc32::crc32(&raw[value_offset..value_offset + value_len])
                    {
                        return Err(bad("value checksum disagrees with sorted record"));
                    }
                    pos += e.version();
                    remaining -= 1;
                    last_seq = Some(seq);
                    Ok(())
                },
            )?;
            verified_blocks.push((offset, len, meta));
            data_end = offset + len;
        }
        if data_end != data_len || pos != end || remaining != 0 || groups != self.count {
            return Err(bad("compact and sorted coverage differ"));
        }
        if self.scan != 0 && u64_at(&self.bytes, self.scan + 16) as usize != scan_prefixes {
            return Err(bad("scan prefix coverage differs"));
        }
        self.blocks = verified_blocks;
        Ok(())
    }
    pub(super) fn block(&self, idx: usize) -> super::super::block::VerifiedBlock {
        let (offset, len, meta) = self.blocks[idx];
        super::super::block::VerifiedBlock::from_shared(self.data.clone(), offset, len, meta)
    }
    pub(super) fn block_at(&self, offset: u64) -> super::super::block::VerifiedBlock {
        self.block(
            self.blocks
                .binary_search_by_key(&offset, |b| b.0 as u64)
                .expect("validated compact block"),
        )
    }
    pub(super) fn get(&self, key: &[u8], limit: Option<u64>) -> Option<Found> {
        self.locate(key, limit).map(|v| Found {
            kind: v.kind,
            seq: v.seq,
            value: self.data[v.offset..v.offset + v.len].to_vec(),
        })
    }
    pub(in crate::sstable) fn locate(
        &self,
        key: &[u8],
        limit: Option<u64>,
    ) -> Option<ValueLocator> {
        let e = self.encoding;
        // Constant widths remove repeated decoder dispatch from the common
        // single-version layout. All encodings share the same lookup body.
        match (e.address, e.key, e.count, e.seq, e.value) {
            (4, 1, 1, 2, 2) => self.get_encoded(
                key,
                limit,
                Encoding {
                    address: 4,
                    key: 1,
                    count: 1,
                    seq: 2,
                    value: 2,
                    checksum: e.checksum,
                },
            ),
            (4, 1, 1, 4, 2) => self.get_encoded(
                key,
                limit,
                Encoding {
                    address: 4,
                    key: 1,
                    count: 1,
                    seq: 4,
                    value: 2,
                    checksum: e.checksum,
                },
            ),
            _ => self.get_encoded(key, limit, e),
        }
    }
    #[inline(always)]
    fn get_encoded(&self, key: &[u8], limit: Option<u64>, e: Encoding) -> Option<ValueLocator> {
        let h = hash(key, self.seed);
        let slot = self.slot(h);
        if !self.packed && uint(&self.bytes, slot + e.address, 2) != fingerprint(h) as u64 {
            return None;
        }
        let mut pos = self.arena + uint(&self.bytes, slot, e.address) as usize;
        let anchor = self.arena + uint(&self.bytes, pos, e.address) as usize;
        let shared = uint(&self.bytes, pos + e.address, e.key) as usize;
        let suffix = uint(&self.bytes, pos + e.address + e.key, e.key) as usize;
        let versions = uint(&self.bytes, pos + e.address + 2 * e.key, e.count);
        pos += e.key_header();
        if key.len() != shared + suffix
            || key[..shared] != self.bytes[anchor..anchor + shared]
            || key[shared..] != self.bytes[pos..pos + suffix]
        {
            return None;
        }
        pos += suffix;
        for _ in 0..versions {
            let seq = self.min_seq + uint(&self.bytes, pos, e.seq);
            if limit.is_none_or(|limit| seq <= limit) {
                let value = uint(&self.bytes, pos + e.seq, e.value);
                let kind = if value & e.tombstone() != 0 {
                    Kind::Tombstone
                } else {
                    Kind::Put
                };
                let offset = uint(&self.bytes, pos + e.seq + e.value, e.address) as usize;
                let len = (value & !e.tombstone()) as usize;
                return Some(ValueLocator {
                    kind,
                    seq,
                    offset,
                    len,
                    checksum: if e.checksum {
                        u32_at(&self.bytes, pos + e.seq + e.value + e.address)
                    } else {
                        0
                    },
                });
            }
            pos += e.version();
        }
        None
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ScanKey<'a> {
    prefix: &'a [u8],
    suffix: &'a [u8],
}
impl ScanKey<'_> {
    pub(crate) fn cmp_bytes(self, key: &[u8]) -> std::cmp::Ordering {
        let n = self.prefix.len().min(key.len());
        self.prefix[..n].cmp(&key[..n]).then_with(|| {
            if key.len() < self.prefix.len() {
                std::cmp::Ordering::Greater
            } else {
                self.suffix.cmp(&key[n..])
            }
        })
    }
    pub(crate) fn cmp_key(self, other: Self) -> std::cmp::Ordering {
        self.prefix
            .iter()
            .chain(self.suffix)
            .cmp(other.prefix.iter().chain(other.suffix))
    }
    pub(crate) fn copy_into(self, key: &mut Vec<u8>) {
        key.clear();
        key.reserve_exact(self.prefix.len() + self.suffix.len());
        key.extend_from_slice(self.prefix);
        key.extend_from_slice(self.suffix);
    }
}
#[derive(Clone, Copy)]
pub(crate) struct ScanValue<'a> {
    pub(crate) seq: u64,
    pub(crate) kind: Kind,
    pub(crate) value: &'a [u8],
}
impl CompactIndex {
    pub(crate) fn scan_config(&self) -> Option<(usize, usize, usize)> {
        (self.scan != 0 && self.encoding.address == 4).then(|| {
            (
                u32_at(&self.bytes, self.scan + 8) as usize,
                self.count,
                u64_at(&self.bytes, self.scan + 16) as usize,
            )
        })
    }
    pub(crate) fn scan_offset(&self, index: usize) -> u32 {
        uint(
            &self.bytes,
            self.scan + SCAN_HEADER + index * self.encoding.address,
            self.encoding.address,
        ) as u32
    }
    #[inline]
    pub(crate) fn scan_key(&self, offset: u32) -> ScanKey<'_> {
        let e = self.encoding;
        let p = self.arena + offset as usize;
        let anchor = self.arena + uint(&self.bytes, p, e.address) as usize;
        let shared = uint(&self.bytes, p + e.address, e.key) as usize;
        let len = uint(&self.bytes, p + e.address + e.key, e.key) as usize;
        let suffix = p + e.key_header();
        ScanKey {
            prefix: &self.bytes[anchor..anchor + shared],
            suffix: &self.bytes[suffix..suffix + len],
        }
    }
    #[inline]
    pub(crate) fn scan_value(&self, offset: u32, limit: u64) -> (u32, Option<ScanValue<'_>>) {
        let e = self.encoding;
        match (e.key, e.count, e.seq, e.value) {
            (1, 1, 2, 2) => self.scan_value_encoded(
                offset,
                limit,
                Encoding {
                    address: 4,
                    key: 1,
                    count: 1,
                    seq: 2,
                    value: 2,
                    checksum: e.checksum,
                },
            ),
            (1, 1, 4, 2) => self.scan_value_encoded(
                offset,
                limit,
                Encoding {
                    address: 4,
                    key: 1,
                    count: 1,
                    seq: 4,
                    value: 2,
                    checksum: e.checksum,
                },
            ),
            _ => self.scan_value_encoded(offset, limit, e),
        }
    }
    pub(crate) fn scan_values<T, E>(
        &self,
        offset: u32,
        count: usize,
        limit: u64,
        map: &mut impl FnMut(&[u8]) -> Result<T, E>,
        out: &mut Vec<T>,
    ) -> Result<(), E> {
        let e = self.encoding;
        match (e.address, e.key, e.count, e.seq, e.value) {
            (4, 1, 1, 2, 2) => self.scan_span(
                offset,
                count,
                limit,
                Encoding {
                    address: 4,
                    key: 1,
                    count: 1,
                    seq: 2,
                    value: 2,
                    checksum: e.checksum,
                },
                map,
                out,
            ),
            (4, 1, 1, 4, 2) => self.scan_span(
                offset,
                count,
                limit,
                Encoding {
                    address: 4,
                    key: 1,
                    count: 1,
                    seq: 4,
                    value: 2,
                    checksum: e.checksum,
                },
                map,
                out,
            ),
            _ => self.scan_span(offset, count, limit, e, map, out),
        }
    }
    #[inline(always)]
    fn scan_span<T, E>(
        &self,
        offset: u32,
        count: usize,
        limit: u64,
        e: Encoding,
        map: &mut impl FnMut(&[u8]) -> Result<T, E>,
        out: &mut Vec<T>,
    ) -> Result<(), E> {
        if limit == u64::MAX {
            self.scan_span_encoded::<true, T, E>(offset, count, limit, e, map, out)
        } else {
            self.scan_span_encoded::<false, T, E>(offset, count, limit, e, map, out)
        }
    }
    #[inline(always)]
    fn scan_span_encoded<const CURRENT: bool, T, E>(
        &self,
        mut offset: u32,
        count: usize,
        limit: u64,
        e: Encoding,
        map: &mut impl FnMut(&[u8]) -> Result<T, E>,
        out: &mut Vec<T>,
    ) -> Result<(), E> {
        for _ in 0..count {
            if CURRENT {
                let p = self.arena + offset as usize;
                let suffix = uint(&self.bytes, p + e.address + e.key, e.key) as usize;
                let versions = uint(&self.bytes, p + e.address + 2 * e.key, e.count) as usize;
                let p = p + e.key_header() + suffix;
                offset = (p + versions * e.version() - self.arena) as u32;
                let value = uint(&self.bytes, p + e.seq, e.value);
                if value & e.tombstone() == 0 {
                    let at = uint(&self.bytes, p + e.seq + e.value, e.address) as usize;
                    let len = value as usize;
                    out.push(map(&self.data[at..at + len])?);
                }
            } else {
                let (next, value) = self.scan_value_encoded(offset, limit, e);
                offset = next;
                if let Some(value) = value
                    && value.kind == Kind::Put
                {
                    out.push(map(value.value)?);
                }
            }
        }
        Ok(())
    }
    #[inline(always)]
    fn scan_value_encoded(
        &self,
        offset: u32,
        limit: u64,
        e: Encoding,
    ) -> (u32, Option<ScanValue<'_>>) {
        let start = self.arena + offset as usize;
        let suffix = uint(&self.bytes, start + e.address + e.key, e.key) as usize;
        let count = uint(&self.bytes, start + e.address + 2 * e.key, e.count) as usize;
        let mut p = start + e.key_header() + suffix;
        let next = (p + count * e.version() - self.arena) as u32;
        for _ in 0..count {
            let seq = self.min_seq + uint(&self.bytes, p, e.seq);
            if seq <= limit {
                let value = uint(&self.bytes, p + e.seq, e.value);
                let at = uint(&self.bytes, p + e.seq + e.value, e.address) as usize;
                let len = (value & !e.tombstone()) as usize;
                return (
                    next,
                    Some(ScanValue {
                        seq,
                        kind: if value & e.tombstone() == 0 {
                            Kind::Put
                        } else {
                            Kind::Tombstone
                        },
                        value: &self.data[at..at + len],
                    }),
                );
            }
            p += e.version();
        }
        (next, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::BlockCache;
    use std::sync::Arc;

    fn compact_fixture(address_width: usize) -> Fixture {
        let mut table = super::super::super::TableBuilder::with_point_index(true);
        let mut builder = PointBuilder::default();
        let mut patches = Vec::new();
        let mut keys = vec![Vec::new(), vec![0, 255]];
        for i in 0..1030u64 {
            keys.push([vec![b'a'; 512], i.to_be_bytes().to_vec()].concat());
        }
        for (i, key) in keys.iter().enumerate() {
            for (seq, kind, value) in [
                (9, Kind::Tombstone, Vec::new()),
                (7, Kind::Put, vec![i as u8; 50]),
            ] {
                table.add(kind, key, &value, seq).unwrap();
                patches.push(builder.add(kind, key, &value, seq, seq == 9).unwrap());
            }
        }
        let mut f = fixture(table);
        let mut key = Vec::new();
        let mut patches = patches.into_iter();
        for &(offset, len) in &f.blocks {
            let raw = &f.data[offset as usize..(offset + len) as usize];
            let meta = super::super::super::block::VerifiedBlock::verify(raw).unwrap();
            super::super::super::block::visit_records(
                raw,
                &meta,
                &mut key,
                |_, _, _, value_offset, _| {
                    builder.set_value_offset(patches.next().unwrap(), offset + value_offset as u64);
                    Ok(())
                },
            )
            .unwrap();
        }
        let e = Encoding {
            address: address_width,
            ..Encoding::for_builder(&builder)
        };
        f.bytes = encode_with(builder, e).unwrap();
        f
    }
    fn decode(f: Fixture) -> Result<CompactIndex, SstError> {
        let cache = Arc::new(BlockCache::new(f.bytes.len() + f.data.len()));
        let reservation = cache.reserve_point(f.bytes.len() + f.data.len()).unwrap();
        CompactIndex::decode(f.bytes.into(), f.data.into(), reservation, f.blocks)
    }
    #[test]
    fn compact_point_narrow_and_wide_roundtrip_versions_and_anchors() {
        for width in [4, 8] {
            let point = decode(compact_fixture(width)).unwrap();
            assert_eq!(point.get(b"", None).unwrap().kind, Kind::Tombstone);
            assert_eq!(point.get(b"", Some(8)).unwrap().value, vec![0; 50]);
            assert!(point.get(b"", Some(6)).is_none());
            for i in 0..1030u64 {
                let key = [vec![b'a'; 512], i.to_be_bytes().to_vec()].concat();
                assert_eq!(
                    point.get(&key, Some(7)).unwrap().value,
                    vec![(i + 2) as u8; 50]
                );
            }
            assert!(point.get(b"absent", None).is_none());
        }
    }
    #[test]
    fn compact_point_large_fields_and_sequence_extremes() {
        let mut table = super::super::super::TableBuilder::with_point_index(true);
        table.add(Kind::Put, b"a", b"old", 0).unwrap();
        let key = vec![b'z'; 90000];
        table
            .add(Kind::Put, &key, &vec![7; 100000], u64::MAX)
            .unwrap();
        let point = decode(fixture(table)).unwrap();
        assert_eq!(point.get(b"a", Some(0)).unwrap().value, b"old");
        assert!(point.get(&key, Some(u64::MAX - 1)).is_none());
        assert_eq!(point.get(&key, None).unwrap().value, vec![7; 100000]);
        let mut table = super::super::super::TableBuilder::with_point_index(true);
        for seq in (0..=65536).rev() {
            table.add(Kind::Put, b"key", b"v", seq).unwrap();
        }
        let point = decode(fixture(table)).unwrap();
        for seq in [0, 1, 255, 256, 65535, 65536] {
            assert_eq!(point.get(b"key", Some(seq)).unwrap().seq, seq);
        }
    }
    #[test]
    fn compact_point_checksummed_malformed_fields_release_reservation() {
        for width in [4, 8] {
            let f = compact_fixture(width);
            let arena = u64_at(&f.bytes, 24) as usize;
            let slots = HEADER + u32_at(&f.bytes, 20) as usize * 4;
            let e = Encoding {
                address: width,
                key: u32_at(&f.bytes, 36) as usize,
                count: u32_at(&f.bytes, 40) as usize,
                seq: u32_at(&f.bytes, 44) as usize,
                value: u32_at(&f.bytes, 48) as usize,
                checksum: true,
            };
            for at in [
                16,
                20,
                24,
                32,
                36,
                40,
                44,
                48,
                52,
                56,
                HEADER,
                slots,
                slots + width,
                arena,
                arena + width,
                arena + width + e.key,
                arena + width + 2 * e.key,
                arena + e.key_header(),
                arena + e.key_header() + e.seq,
                arena + e.key_header() + e.seq + e.value,
            ] {
                let mut bytes = f.bytes.clone();
                bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
                let end = bytes.len() - 4;
                let crc = crate::crc32::crc32(&bytes[..end]);
                bytes[end..].copy_from_slice(&crc.to_le_bytes());
                let cache = Arc::new(BlockCache::new(bytes.len() + f.data.len()));
                let reservation = cache.reserve_point(bytes.len() + f.data.len()).unwrap();
                assert!(
                    CompactIndex::decode(
                        bytes.into(),
                        f.data.clone().into(),
                        reservation,
                        f.blocks.clone()
                    )
                    .is_err(),
                    "accepted width {width}, field {at}"
                );
                assert_eq!(cache.resident_bytes(), 0);
            }
            // A checksummed pointer to another in-range value is still invalid.
            let mut wrong = compact_fixture(width);
            let at = arena + e.key_header() + e.seq + e.value;
            let next = at + e.version();
            wrong.bytes[at..at + width].copy_from_slice(&f.bytes[next..next + width]);
            let end = wrong.bytes.len() - 4;
            let crc = crate::crc32::crc32(&wrong.bytes[..end]);
            wrong.bytes[end..].copy_from_slice(&crc.to_le_bytes());
            assert!(decode(wrong).is_err());
            let mut wrong = compact_fixture(width);
            wrong.data[25] ^= 1;
            assert!(decode(wrong).is_err());
        }
    }
    #[test]
    fn locator_crc_fields_and_budget_are_validated_before_admission() {
        for width in [4, 8] {
            for corrupt in [false, true] {
                let mut f = compact_fixture(width);
                if corrupt {
                    let arena = u64_at(&f.bytes, 24) as usize;
                    let e = Encoding {
                        address: width,
                        key: 2,
                        count: 1,
                        seq: 1,
                        value: 2,
                        checksum: true,
                    };
                    // The first key is empty; corrupt its first value CRC and
                    // recompute the section CRC to exercise semantic validation.
                    let crc_at = arena + e.key_header() + e.seq + e.value + e.address;
                    f.bytes[crc_at] ^= 1;
                    let end = f.bytes.len() - 4;
                    let crc = crate::crc32::crc32(&f.bytes[..end]);
                    f.bytes[end..].copy_from_slice(&crc.to_le_bytes());
                }
                let charge = CompactIndex::locator_bytes(
                    &f.bytes,
                    f.bytes.len() as u64,
                    f.data.len() as u64,
                )
                .unwrap();
                let cache = Arc::new(BlockCache::new(charge));
                let reservation = cache.reserve_point(charge).unwrap();
                let point = CompactIndex::decode_locators(
                    f.bytes.into(),
                    f.data.len(),
                    reservation,
                    f.blocks,
                    |offset, len| Ok(std::borrow::Cow::Borrowed(&f.data[offset..offset + len])),
                );
                if corrupt {
                    assert!(point.is_err());
                    assert_eq!(cache.stats().point_index_bytes, 0);
                } else {
                    let point = point.unwrap().unwrap();
                    assert_eq!(point.bytes.len(), charge);
                    let v = point.locate(b"", Some(7)).unwrap();
                    assert_eq!(
                        v.checksum,
                        crate::crc32::crc32(&f.data[v.offset..v.offset + v.len])
                    );
                    assert_eq!(cache.stats().point_index_bytes, charge);
                    drop(point);
                    assert_eq!(cache.stats().point_index_bytes, 0);
                }
            }
        }
    }
    #[test]
    fn locator_admission_respects_address_space_and_legacy_verification() {
        let mut f = compact_fixture(4);
        f.bytes[52..56].copy_from_slice(&0u32.to_le_bytes());
        assert!(CompactIndex::locator_bytes(&f.bytes, f.bytes.len() as u64, 1).is_none());
        f.bytes[52..56].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            CompactIndex::locator_bytes(&f.bytes, f.bytes.len() as u64, u64::MAX).is_some(),
            usize::try_from(u64::MAX).is_ok()
        );
    }
    #[test]
    fn compact_point_fingerprint_collision_still_compares_complete_key() {
        let mut table = super::super::super::TableBuilder::with_point_index(true);
        table.add(Kind::Put, b"present", b"secret", 1).unwrap();
        let point = decode(fixture(table)).unwrap();
        let wanted = fingerprint(hash(b"present", point.seed));
        let absent = (0u64..2_000_000)
            .find_map(|i| {
                let key = i.to_be_bytes();
                (fingerprint(hash(&key, point.seed)) == wanted).then_some(key)
            })
            .expect("a colliding fingerprint");
        assert!(point.get(&absent, None).is_none());
    }
}
