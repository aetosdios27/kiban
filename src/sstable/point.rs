//! Bucket-displacement MPHF with full-key MVCC locators into sorted records.
//! The byte encoding is documented in validation/v2/DESIGN.md.

use super::{Found, Kind, SstError};
use crate::cache::ResidentReservation;

const HEADER: usize = 32;
const SLOT: usize = 12;
const DIRECT: u32 = 1 << 31;
const MAGIC: &[u8; 8] = b"KIBANPH2";
const STEP: u64 = 0x9e37_79b9_7f4a_7c15;

fn bad(message: &str) -> SstError {
    SstError::Corrupt(format!("point index: {message}"))
}
fn u32_at(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().unwrap())
}
fn u64_at(raw: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(raw[at..at + 8].try_into().unwrap())
}
fn mix(mut h: u64) -> u64 {
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}
fn hash(key: &[u8], seed: u64) -> u64 {
    let mut h = mix(seed ^ key.len() as u64);
    let (chunks, remainder) = key.as_chunks::<8>();
    for &chunk in chunks {
        h = (h ^ u64::from_le_bytes(chunk))
            .rotate_left(27)
            .wrapping_mul(STEP);
    }
    let mut tail = [0; 8];
    tail[..remainder.len()].copy_from_slice(remainder);
    mix(h ^ u64::from_le_bytes(tail))
}
fn fingerprint(h: u64) -> u16 {
    (mix(h ^ 0xd6e8_feb8_6659_fd93) >> 48) as u16
}
fn bucket(h: u64, count: usize) -> usize {
    (h % count as u64) as usize
}
fn position(h: u64, pilot: u32, n: usize) -> usize {
    (mix(h ^ (pilot as u64).wrapping_mul(STEP)) % n as u64) as usize
}
fn index(h: u64, pilots: &[u32], n: usize) -> usize {
    let pilot = pilots[bucket(h, pilots.len())];
    if pilot & DIRECT != 0 {
        (pilot & !DIRECT) as usize
    } else {
        position(h, pilot, n)
    }
}

/// Exactly N occupied slots. Hash collisions and failed placement retry the
/// full byte-key hash seed, rather than collapsing distinct user keys.
fn construct(keys: &[&[u8]]) -> Result<(u64, Vec<u32>, Vec<usize>), SstError> {
    let n = keys.len();
    if n == 0 || n >= DIRECT as usize {
        return Err(SstError::InvalidArgument("point index key count".into()));
    }
    let buckets = n.div_ceil(3);
    for attempt in 0..8 {
        let seed = mix(STEP.wrapping_mul(attempt + 1));
        let hashes: Vec<_> = keys.iter().map(|key| hash(key, seed)).collect();
        let mut sorted = hashes.clone();
        sorted.sort_unstable();
        if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
            continue;
        }
        drop(sorted);
        let mut members = vec![Vec::new(); buckets];
        for (i, &h) in hashes.iter().enumerate() {
            members[bucket(h, buckets)].push(i);
        }
        let mut order: Vec<_> = (0..buckets).collect();
        order.sort_unstable_by_key(|&b| std::cmp::Reverse(members[b].len()));
        let mut pilots = vec![0; buckets];
        let mut occupied = vec![false; n];
        let mut slots = vec![usize::MAX; n];
        let mut candidates = Vec::new();
        let mut failed = false;
        for &b in &order {
            if members[b].len() <= 1 {
                continue;
            }
            let found = (0..100_000).find(|&pilot| {
                candidates.clear();
                for &member in &members[b] {
                    let at = position(hashes[member], pilot, n);
                    if occupied[at] || candidates.contains(&at) {
                        return false;
                    }
                    candidates.push(at);
                }
                true
            });
            let Some(pilot) = found else {
                failed = true;
                break;
            };
            pilots[b] = pilot;
            for (&member, &at) in members[b].iter().zip(&candidates) {
                occupied[at] = true;
                slots[at] = member;
            }
        }
        if failed {
            continue;
        }
        let mut free = (0..n).filter(|&i| !occupied[i]);
        for &b in &order {
            if members[b].len() == 1 {
                let at = free.next().expect("one free slot per singleton");
                pilots[b] = DIRECT | at as u32;
                slots[at] = members[b][0];
            }
        }
        if slots.contains(&usize::MAX) {
            return Err(bad("construction did not fill every slot"));
        }
        // Validate bijection before any bytes can be published.
        for (at, &member) in slots.iter().enumerate() {
            if index(hashes[member], &pilots, n) != at {
                return Err(bad("construction slot mismatch"));
            }
        }
        return Ok((seed, pilots, slots));
    }
    Err(SstError::InvalidArgument(
        "point index construction exhausted seed retries".into(),
    ))
}

#[derive(Default)]
pub(super) struct PointBuilder {
    arena: Vec<u8>,
    heads: Vec<usize>,
}
impl PointBuilder {
    pub(super) fn approximate_size(&self) -> usize {
        HEADER + self.heads.len().div_ceil(3) * 4 + self.heads.len() * SLOT + self.arena.len() + 5
    }
    pub(super) fn add(
        &mut self,
        kind: Kind,
        key: &[u8],
        value: &[u8],
        seq: u64,
        new_key: bool,
    ) -> Result<usize, SstError> {
        crate::limits::mutation(key, value)
            .map_err(|e| SstError::InvalidArgument(e.to_string()))?;
        if new_key {
            self.heads.push(self.arena.len());
            self.arena
                .extend_from_slice(&(key.len() as u32).to_le_bytes());
            self.arena.extend_from_slice(&1u32.to_le_bytes());
            self.arena.extend_from_slice(key);
        } else {
            let at = self.heads.last().expect("a previous key") + 4;
            let count = u32_at(&self.arena, at)
                .checked_add(1)
                .ok_or_else(|| SstError::InvalidArgument("too many point versions".into()))?;
            self.arena[at..at + 4].copy_from_slice(&count.to_le_bytes());
        }
        self.arena.extend_from_slice(&seq.to_le_bytes());
        self.arena.push(kind.to_u8());
        self.arena
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        let patch = self.arena.len();
        self.arena.extend_from_slice(&0u64.to_le_bytes());
        Ok(patch)
    }
    pub(super) fn set_value_offset(&mut self, patch: usize, value_offset: u64) {
        self.arena[patch..patch + 8].copy_from_slice(&value_offset.to_le_bytes());
    }
    pub(super) fn finish(self) -> Result<Vec<u8>, SstError> {
        let keys: Vec<_> = self
            .heads
            .iter()
            .map(|&p| &self.arena[p + 8..p + 8 + u32_at(&self.arena, p) as usize])
            .collect();
        let (seed, pilots, slots) = construct(&keys)?;
        let arena_start = HEADER + pilots.len() * 4 + slots.len() * SLOT;
        let mut bytes = Vec::with_capacity(arena_start + self.arena.len() + 5);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&seed.to_le_bytes());
        bytes.extend_from_slice(&(slots.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(pilots.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(arena_start as u64).to_le_bytes());
        for pilot in pilots {
            bytes.extend_from_slice(&pilot.to_le_bytes());
        }
        for member in slots {
            bytes.extend_from_slice(&(self.heads[member] as u64).to_le_bytes());
            bytes.extend_from_slice(&fingerprint(hash(keys[member], seed)).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }
        bytes.extend_from_slice(&self.arena);
        bytes.push(0);
        let crc = crate::crc32::crc32(&bytes);
        bytes.extend_from_slice(&crc.to_le_bytes());
        Ok(bytes)
    }
}

pub(super) struct PointIndex {
    bytes: crate::cache::ArcBlock,
    data: crate::cache::ArcBlock,
    blocks: Vec<(usize, usize, crate::cache::BlockMeta)>,
    seed: u64,
    count: usize,
    buckets: usize,
    arena: usize,
    _reservation: ResidentReservation,
}
impl PointIndex {
    pub(super) fn decode(
        bytes: crate::cache::ArcBlock,
        data: crate::cache::ArcBlock,
        reservation: ResidentReservation,
        blocks: impl IntoIterator<Item = (u64, u64)>,
    ) -> Result<Self, SstError> {
        super::reader::verify_trailer(&bytes, "point index")?;
        if bytes.len() < HEADER + 5 || &bytes[..8] != MAGIC {
            return Err(bad("unsupported header"));
        }
        let seed = u64_at(&bytes, 8);
        let count = u32_at(&bytes, 16) as usize;
        let buckets = u32_at(&bytes, 20) as usize;
        let arena =
            usize::try_from(u64_at(&bytes, 24)).map_err(|_| bad("arena offset overflow"))?;
        if count == 0
            || count >= DIRECT as usize
            || buckets != count.div_ceil(3)
            || buckets
                .checked_mul(4)
                .and_then(|n| HEADER.checked_add(n))
                .and_then(|n| count.checked_mul(SLOT).and_then(|size| n.checked_add(size)))
                != Some(arena)
            || arena >= bytes.len() - 5
        {
            return Err(bad("invalid section dimensions"));
        }
        let mut result = Self {
            bytes,
            data,
            blocks: Vec::new(),
            seed,
            count,
            buckets,
            arena,
            _reservation: reservation,
        };
        result.validate()?;
        result.validate_backing_records(blocks)?;
        Ok(result)
    }
    fn slot(&self, h: u64) -> usize {
        let pilot = u32_at(&self.bytes, HEADER + bucket(h, self.buckets) * 4);
        let at = if pilot & DIRECT != 0 {
            (pilot & !DIRECT) as usize
        } else {
            position(h, pilot, self.count)
        };
        HEADER + self.buckets * 4 + at * SLOT
    }
    fn validate(&self) -> Result<(), SstError> {
        let raw = &self.bytes[..self.bytes.len() - 5];
        for b in 0..self.buckets {
            let pilot = u32_at(raw, HEADER + b * 4);
            if pilot & DIRECT != 0 && (pilot & !DIRECT) as usize >= self.count {
                return Err(bad("singleton slot out of bounds"));
            }
        }
        let mut p = self.arena;
        let mut previous: Option<&[u8]> = None;
        for _ in 0..self.count {
            let start = p;
            if raw.len().saturating_sub(p) < 8 {
                return Err(bad("truncated key header"));
            }
            let kl = u32_at(raw, p) as usize;
            let versions = u32_at(raw, p + 4) as usize;
            p += 8;
            if kl > crate::limits::MAX_KEY_BYTES || kl > raw.len() - p || versions == 0 {
                return Err(bad("invalid key length or version count"));
            }
            let key = &raw[p..p + kl];
            p += kl;
            if previous.is_some_and(|prev| prev >= key) {
                return Err(bad("keys are not strictly increasing"));
            }
            previous = Some(key);
            let h = hash(key, self.seed);
            let s = self.slot(h);
            if u64_at(raw, s) != (start - self.arena) as u64
                || u32_at(raw, s + 8) != fingerprint(h) as u32
            {
                return Err(bad("slot does not identify its full-key record"));
            }
            let mut last_seq = None;
            for _ in 0..versions {
                if raw.len().saturating_sub(p) < 21 {
                    return Err(bad("truncated version header"));
                }
                let seq = u64_at(raw, p);
                let kind = Kind::from_u8(raw[p + 8]).ok_or_else(|| bad("invalid record kind"))?;
                let vl = u32_at(raw, p + 9) as usize;
                let offset = u64_at(raw, p + 13);
                p += 21;
                if vl > crate::limits::MAX_VALUE_BYTES
                    || offset
                        .checked_add(vl as u64)
                        .is_none_or(|end| end > self.data.len() as u64)
                    || (kind == Kind::Tombstone && vl != 0)
                    || last_seq.is_some_and(|last| last <= seq)
                {
                    return Err(bad("invalid value length or version ordering"));
                }
                last_seq = Some(seq);
            }
        }
        if p != raw.len() {
            return Err(bad("record arena has trailing bytes"));
        }
        Ok(())
    }
    fn validate_backing_records(
        &mut self,
        blocks: impl IntoIterator<Item = (u64, u64)>,
    ) -> Result<(), SstError> {
        let mut pos = self.arena;
        let end = self.bytes.len() - 5;
        let mut remaining = 0u32;
        let mut point_key: &[u8] = &[];
        let mut key = Vec::new();
        let mut data_end = 0;
        for (offset, len) in blocks {
            let offset = usize::try_from(offset).map_err(|_| bad("data offset overflow"))?;
            let len = usize::try_from(len).map_err(|_| bad("data length overflow"))?;
            if offset != data_end
                || offset
                    .checked_add(len)
                    .is_none_or(|end| end > self.data.len())
            {
                return Err(bad("invalid backing block bounds"));
            }
            let raw = &self.data[offset..offset + len];
            let meta = super::block::VerifiedBlock::verify(raw)?;
            super::block::visit_records(
                raw,
                &meta,
                &mut key,
                |kind, seq, key, value_offset, value_len| {
                    if remaining == 0 {
                        if pos == end {
                            return Err(bad("sorted record missing from point index"));
                        }
                        let kl = u32_at(&self.bytes, pos) as usize;
                        remaining = u32_at(&self.bytes, pos + 4);
                        pos += 8;
                        point_key = &self.bytes[pos..pos + kl];
                        pos += kl;
                    }
                    if point_key != key
                        || u64_at(&self.bytes, pos) != seq
                        || self.bytes[pos + 8] != kind.to_u8()
                        || u32_at(&self.bytes, pos + 9) as usize != value_len
                        || u64_at(&self.bytes, pos + 13) != (offset + value_offset) as u64
                    {
                        return Err(bad("point locator disagrees with sorted record"));
                    }
                    pos += 21;
                    remaining -= 1;
                    Ok(())
                },
            )?;
            self.blocks.push((offset, len, meta));
            data_end = offset + len;
        }
        if data_end != self.data.len() || pos != end || remaining != 0 {
            return Err(bad("point and sorted record coverage differ"));
        }
        Ok(())
    }

    pub(super) fn block(&self, idx: usize) -> super::block::VerifiedBlock {
        let (offset, len, meta) = self.blocks[idx];
        super::block::VerifiedBlock::from_shared(self.data.clone(), offset, len, meta)
    }

    pub(super) fn block_at(&self, offset: u64) -> super::block::VerifiedBlock {
        let idx = self
            .blocks
            .binary_search_by_key(&offset, |b| b.0 as u64)
            .expect("validated sorted block offset");
        self.block(idx)
    }

    pub(super) fn get(&self, key: &[u8], limit: Option<u64>) -> Option<Found> {
        let h = hash(key, self.seed);
        let s = self.slot(h);
        if u32_at(&self.bytes, s + 8) as u16 != fingerprint(h) {
            return None;
        }
        // Decode validated addresses only. The complete key comparison is
        // mandatory even when the fingerprint matches an absent key.
        let mut p = self.arena + u64_at(&self.bytes, s) as usize;
        let kl = u32_at(&self.bytes, p) as usize;
        let versions = u32_at(&self.bytes, p + 4);
        p += 8;
        if &self.bytes[p..p + kl] != key {
            return None;
        }
        p += kl;
        for _ in 0..versions {
            let seq = u64_at(&self.bytes, p);
            let kind = Kind::from_u8(self.bytes[p + 8]).expect("validated kind");
            let vl = u32_at(&self.bytes, p + 9) as usize;
            let offset = u64_at(&self.bytes, p + 13) as usize;
            p += 21;
            if limit.is_none_or(|lim| seq <= lim) {
                return Some(Found {
                    kind,
                    seq,
                    value: self.data[offset..offset + vl].to_vec(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
struct Fixture {
    bytes: Vec<u8>,
    data: Vec<u8>,
    blocks: Vec<(u64, u64)>,
}
#[cfg(test)]
fn fixture(builder: super::TableBuilder) -> Fixture {
    let raw = builder.finish().unwrap();
    let footer = raw.len() - super::builder::FOOTER_LEN;
    let extra = footer - super::builder::POINT_FOOTER_EXTRA;
    let point = u64_at(&raw, extra) as usize;
    let len = u64_at(&raw, extra + 8) as usize;
    let mut pos = u64_at(&raw, footer) as usize;
    let count = u32_at(&raw, pos);
    pos += 4;
    let mut blocks = Vec::new();
    for _ in 0..count {
        let kl = u32_at(&raw, pos) as usize;
        pos += 4 + kl;
        blocks.push((u64_at(&raw, pos), u64_at(&raw, pos + 8)));
        pos += 16;
    }
    Fixture {
        bytes: raw[point..point + len].to_vec(),
        data: raw[..point].to_vec(),
        blocks,
    }
}
#[cfg(test)]
fn decode_fixture(f: Fixture) -> Result<PointIndex, SstError> {
    let cache = std::sync::Arc::new(crate::cache::BlockCache::new(f.bytes.len() + f.data.len()));
    let reservation = cache.reserve_point(f.bytes.len() + f.data.len()).unwrap();
    PointIndex::decode(f.bytes.into(), f.data.into(), reservation, f.blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_decode_retains_shared_allocations_and_accounts_for_both_regions() {
        let mut builder = super::super::TableBuilder::with_point_index(true);
        builder.add(Kind::Put, b"key", &[7; 100_000], 1).unwrap();
        let f = fixture(builder);
        let bytes: crate::cache::ArcBlock = f.bytes.into();
        let data: crate::cache::ArcBlock = f.data.into();
        let resident = bytes.len() + data.len();
        let cache = std::sync::Arc::new(crate::cache::BlockCache::new(resident));
        let reservation = cache.reserve_point(resident).unwrap();
        let point = PointIndex::decode(bytes.clone(), data.clone(), reservation, f.blocks).unwrap();
        assert!(std::sync::Arc::ptr_eq(&point.bytes, &bytes));
        assert!(std::sync::Arc::ptr_eq(&point.data, &data));
        assert_eq!(cache.stats().point_index_bytes, resident);
        assert!(cache.reserve_point(1).is_none());
        assert_eq!(point.get(b"key", None).unwrap().value, [7; 100_000]);
        drop(point);
        assert_eq!(cache.resident_bytes(), 0);
    }

    #[test]
    fn point_index_roundtrip_versions_and_binary_keys() {
        let mut builder = super::super::TableBuilder::with_point_index(true);
        builder.add(Kind::Put, b"", b"empty", 1).unwrap();
        builder.add(Kind::Tombstone, b"\0\xff", b"", 9).unwrap();
        builder.add(Kind::Put, b"\0\xff", b"old", 7).unwrap();
        builder.add(Kind::Put, b"long", &[3; 100_000], 2).unwrap();
        let index = decode_fixture(fixture(builder)).unwrap();
        assert_eq!(index.get(b"", None).unwrap().value, b"empty");
        assert_eq!(index.get(b"\0\xff", None).unwrap().kind, Kind::Tombstone);
        assert_eq!(index.get(b"\0\xff", Some(8)).unwrap().value, b"old");
        assert!(index.get(b"\0\xff", Some(6)).is_none());
        assert!(index.get(b"missing", None).is_none());
        assert_eq!(index.get(b"long", None).unwrap().value, [3; 100_000]);
    }

    #[test]
    fn on_disk_bucket_reduction_uses_all_64_hash_bits() {
        // Truncating h to usize before reduction would route this to 2 on
        // 32-bit readers, making files written by 64-bit writers unreadable.
        assert_eq!(bucket((1u64 << 32) + 42, 10), 8);
        let pilots: Vec<_> = (0..10).map(|i| DIRECT | i).collect();
        assert_eq!(index((1u64 << 32) + 42, &pilots, 10), 8);
    }

    #[test]
    fn point_index_bijection_property() {
        for n in [1, 2, 3, 8, 17, 64, 511, 4096] {
            let keys: Vec<_> = (0..n as u64)
                .map(|i| {
                    let mut key = vec![0; i as usize % 127];
                    key.extend_from_slice(&i.to_be_bytes());
                    key
                })
                .collect();
            let refs: Vec<_> = keys.iter().map(Vec::as_slice).collect();
            let (seed, pilots, slots) = construct(&refs).unwrap();
            let mut occupied = vec![false; n];
            for (member, key) in keys.iter().enumerate() {
                let at = index(hash(key, seed), &pilots, n);
                assert!(!occupied[at]);
                occupied[at] = true;
                assert_eq!(slots[at], member);
            }
            assert!(occupied.into_iter().all(|v| v));
        }
    }
}

#[cfg(test)]
mod corruption_tests {
    use super::*;
    use crate::cache::BlockCache;
    use std::sync::Arc;

    #[test]
    fn matching_fingerprint_for_absent_key_requires_full_key_comparison() {
        let mut builder = super::super::TableBuilder::with_point_index(true);
        builder.add(Kind::Put, b"present", b"secret", 1).unwrap();
        let point = decode_fixture(fixture(builder)).unwrap();
        let wanted = fingerprint(hash(b"present", point.seed));
        let absent = (0u64..2_000_000)
            .find_map(|i| {
                let key = i.to_be_bytes();
                (fingerprint(hash(&key, point.seed)) == wanted).then_some(key)
            })
            .expect("a colliding fingerprint");
        // Singleton MPHF routes every byte string to its one slot.
        assert_eq!(
            point.slot(hash(&absent, point.seed)),
            point.slot(hash(b"present", point.seed))
        );
        assert!(point.get(&absent, None).is_none());
        assert_eq!(point.get(b"present", None).unwrap().value, b"secret");
    }

    #[test]
    fn malformed_checksummed_point_records_are_rejected_and_release_budget() {
        let mut builder = super::super::TableBuilder::with_point_index(true);
        builder.add(Kind::Put, b"a", b"one", 4).unwrap();
        builder.add(Kind::Put, b"b", b"two", 3).unwrap();
        let f = fixture(builder);
        let original = &f.bytes;
        let arena = u64_at(original, 24) as usize;
        let slots = HEADER + u32_at(original, 20) as usize * 4;
        // Header count/dimensions, singleton index, slot offsets/fingerprint,
        // record lengths, version count, kind, and key ordering.
        for at in [
            16,
            20,
            24,
            HEADER,
            slots,
            slots + 8,
            arena,
            arena + 4,
            arena + 8,
            arena + 17,
        ] {
            let mut bytes = original.to_vec();
            bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            let end = bytes.len() - 4;
            let crc = crate::crc32::crc32(&bytes[..end]);
            bytes[end..].copy_from_slice(&crc.to_le_bytes());
            let cache = Arc::new(BlockCache::new(bytes.len() + f.data.len()));
            let reservation = cache.reserve_point(bytes.len() + f.data.len()).unwrap();
            assert!(
                PointIndex::decode(
                    bytes.into(),
                    f.data.clone().into(),
                    reservation,
                    f.blocks.clone()
                )
                .is_err(),
                "accepted invalid field at {at}"
            );
            assert_eq!(cache.resident_bytes(), 0);
        }
    }
    #[test]
    fn in_range_wrong_value_locator_and_backing_bit_flip_are_rejected() {
        let mut builder = super::super::TableBuilder::with_point_index(true);
        builder.add(Kind::Put, b"a", b"one", 4).unwrap();
        let mut f = fixture(builder);
        let arena = u64_at(&f.bytes, 24) as usize;
        f.bytes[arena + 22..arena + 30].copy_from_slice(&0u64.to_le_bytes());
        let end = f.bytes.len() - 4;
        let crc = crate::crc32::crc32(&f.bytes[..end]);
        f.bytes[end..].copy_from_slice(&crc.to_le_bytes());
        assert!(decode_fixture(f).is_err());
        let mut builder = super::super::TableBuilder::with_point_index(true);
        builder.add(Kind::Put, b"a", b"one", 4).unwrap();
        let mut f = fixture(builder);
        f.data[22] ^= 1;
        assert!(decode_fixture(f).is_err());
    }
}
