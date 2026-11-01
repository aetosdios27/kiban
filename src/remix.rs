//! Immutable cross-SST population order. Locators refer to validated PH3
//! groups; values, keys and MVCC histories remain in their original backing.
use crate::cache::ResidentReservation;
use crate::db::Version;
use crate::sstable::point::compact::{ScanKey, ScanValue};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

const CONTINUES: u64 = 1 << 63;
const STRIDE: usize = 32;

#[derive(Clone, Copy, Default, Debug)]
#[repr(C, align(64))]
struct Prefix {
    key: [u8; 24],
    first: u64,
    lo: u32,
    hi: u32,
    span: u32,
    len: u8,
}
#[derive(Debug)]
#[repr(C)]
struct Anchor {
    key: [u8; 40],
    len: u32,
    position: u32,
}
#[derive(Debug)]
pub(crate) struct Remix {
    prefixes: Vec<Prefix>,
    anchors: Vec<Anchor>,
    locators: Vec<u64>,
    prefix_len: usize,
    build_ns: u64,
    resident_bytes: usize,
    _reservation: ResidentReservation,
}
struct Head<'a> {
    key: ScanKey<'a>,
    run: usize,
    record: usize,
    offset: u32,
}
impl PartialEq for Head<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Head<'_> {}
impl PartialOrd for Head<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Head<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .key
            .cmp_key(self.key)
            .then_with(|| other.run.cmp(&self.run))
    }
}
fn prefix_hash(key: &[u8]) -> usize {
    // Prefixes are short. Full bytes decide every hit after this routing hash.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in key {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    (h ^ (h >> 32)) as usize
}
fn unpack(locator: u64) -> (usize, u32) {
    (((locator & !CONTINUES) >> 32) as usize, locator as u32)
}
impl Remix {
    pub(crate) fn stats(&self) -> crate::db::RangeIndexStats {
        crate::db::RangeIndexStats {
            key_groups: self.locators.len(),
            prefixes: self.prefixes.iter().filter(|p| p.len != 0).count(),
            direct_ten_prefixes: self.prefixes.iter().filter(|p| p.span >= 10).count(),
            resident_bytes: self.resident_bytes,
            build_ns: self.build_ns,
        }
    }
    pub(crate) fn build(version: &Version) -> Option<Self> {
        Self::build_cancellable(version, || false)
    }
    pub(crate) fn build_cancellable(
        version: &Version,
        cancelled: impl Fn() -> bool,
    ) -> Option<Self> {
        let started = std::time::Instant::now();
        let first = version.tables.first()?.table.scan_index()?;
        let prefix_len = first.scan_config()?.0;
        if version.tables.len() >= (1 << 31) {
            return None;
        }
        let mut count = 0usize;
        let mut prefixes = 0usize;
        for t in &version.tables {
            let (len, n, p) = t.table.scan_index()?.scan_config()?;
            if len != prefix_len {
                return None;
            }
            count = count.checked_add(n)?;
            prefixes = prefixes.checked_add(p)?;
        }
        if count > u32::MAX as usize {
            return None;
        }
        let slots = prefixes.checked_mul(2)?.checked_next_power_of_two()?;
        let bytes = count
            .checked_mul(8)?
            .checked_add(count.div_ceil(STRIDE).checked_mul(48)?)?
            .checked_add(slots.checked_mul(64)?)?;
        let cache = version.tables[0].table.scan_cache();
        let reservation = cache.reserve_point(bytes)?;
        let _workspace = cache.reserve_point(
            prefixes
                .checked_mul(64)?
                .checked_add(version.tables.len().checked_mul(64)?)?
                .checked_add(2 * crate::limits::MAX_KEY_BYTES)?,
        )?;
        let mut result = Self {
            prefixes: vec![Prefix::default(); slots],
            anchors: Vec::with_capacity(count.div_ceil(STRIDE)),
            locators: Vec::with_capacity(count),
            prefix_len,
            build_ns: 0,
            resident_bytes: bytes,
            _reservation: reservation,
        };
        let mut heap = BinaryHeap::with_capacity(version.tables.len());
        for (run, t) in version.tables.iter().enumerate() {
            let index = t.table.scan_index()?;
            let offset = index.scan_offset(0);
            heap.push(Head {
                key: index.scan_key(offset),
                run,
                record: 0,
                offset,
            });
        }
        let mut key = Vec::new();
        let mut previous = Vec::new();
        let mut pending: Option<Prefix> = None;
        while let Some(head) = heap.pop() {
            head.key.copy_into(&mut key);
            let pos = result.locators.len();
            if pos.is_multiple_of(1024) && cancelled() {
                return None;
            }
            if pos > 0 && key == previous {
                result.locators[pos - 1] |= CONTINUES;
            }
            let prefix = &key[..key.len().min(prefix_len)];
            if pending
                .as_ref()
                .is_none_or(|p| &p.key[..p.len as usize] != prefix)
            {
                if let Some(mut p) = pending.take() {
                    p.hi = pos as u32;
                    result.insert_prefix(p, version);
                }
                let mut p = Prefix {
                    len: prefix.len() as u8,
                    lo: pos as u32,
                    ..Prefix::default()
                };
                p.key[..prefix.len()].copy_from_slice(prefix);
                pending = Some(p);
            }
            let locator = ((head.run as u64) << 32) | head.offset as u64;
            result.locators.push(locator);
            if pos.is_multiple_of(STRIDE) {
                let mut anchor = Anchor {
                    key: [0; 40],
                    len: key.len() as u32,
                    position: pos as u32,
                };
                let n = key.len().min(40);
                anchor.key[..n].copy_from_slice(&key[..n]);
                result.anchors.push(anchor);
            }
            std::mem::swap(&mut key, &mut previous);
            let index = version.tables[head.run].table.scan_index()?;
            let record = head.record + 1;
            if record < index.scan_config()?.1 {
                let offset = index.scan_offset(record);
                heap.push(Head {
                    key: index.scan_key(offset),
                    run: head.run,
                    record,
                    offset,
                });
            }
        }
        if let Some(mut p) = pending {
            p.hi = count as u32;
            result.insert_prefix(p, version);
        }
        result.build_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        Some(result)
    }
    fn insert_prefix(&mut self, mut p: Prefix, version: &Version) {
        // Short and empty keys are handled by the existing sorted path.
        if p.len as usize != self.prefix_len {
            return;
        }
        p.first = self.locators[p.lo as usize];
        let (run, mut offset) = unpack(p.first);
        let index = version.tables[run].table.scan_index().unwrap();
        for &loc in &self.locators[p.lo as usize..p.hi as usize] {
            if loc & CONTINUES != 0 || unpack(loc) != (run, offset) {
                break;
            }
            p.span += 1;
            offset = index.scan_value(offset, u64::MAX).0;
        }
        let mut slot = prefix_hash(&p.key[..p.len as usize]) & (self.prefixes.len() - 1);
        while self.prefixes[slot].len != 0 {
            slot = (slot + 1) & (self.prefixes.len() - 1);
        }
        self.prefixes[slot] = p;
    }
    fn prefix(&self, key: &[u8]) -> Option<&Prefix> {
        let mut slot = prefix_hash(key) & (self.prefixes.len() - 1);
        loop {
            let p = &self.prefixes[slot];
            if p.len == 0 {
                return None;
            }
            if &p.key[..p.len as usize] == key {
                return Some(p);
            }
            slot = (slot + 1) & (self.prefixes.len() - 1);
        }
    }
    /// The caller supplies the whole extractor prefix, so its population
    /// extent is already the bound. No successor or key comparison is needed.
    pub(crate) fn prefix_cursor<'a>(
        &'a self,
        version: &'a Version,
        prefix: &'a [u8],
        limit: u64,
    ) -> Option<Cursor<'a>> {
        if prefix.len() != self.prefix_len {
            return None;
        }
        let (pos, hi, direct, first) = self.prefix(prefix).map_or((0, 0, 0, 0), |p| {
            (p.lo as usize, p.hi as usize, p.span as usize, p.first)
        });
        Some(Cursor {
            remix: self,
            version,
            pos,
            hi,
            end: prefix,
            prefix_end: true,
            bounded: true,
            limit,
            direct,
            first,
        })
    }
    /// Only take over bounds proven to stay within one extractor prefix.
    pub(crate) fn cursor<'a>(
        &'a self,
        version: &'a Version,
        start: &[u8],
        end: &'a [u8],
        limit: u64,
    ) -> Option<Cursor<'a>> {
        if start.len() < self.prefix_len || start >= end {
            return None;
        }
        let prefix = &start[..self.prefix_len];
        // An upper bound within this prefix or its immediate successor is safe.
        // Other ranges can include following prefixes and must fall back.
        let mut successor = [0; 24];
        successor[..prefix.len()].copy_from_slice(prefix);
        let last = prefix.iter().rposition(|&b| b != 255);
        let within = if let Some(i) = last {
            successor[i] += 1;
            end <= &successor[..i + 1]
        } else {
            true
        };
        if !within {
            return None;
        }
        let p = self.prefix(prefix);
        let (mut pos, hi, span, first) = p.map_or((0, 0, 0, 0), |p| {
            (p.lo as usize, p.hi as usize, p.span as usize, p.first)
        });
        let bounded = !end.starts_with(prefix);
        if start != prefix && pos < hi {
            let a = self.anchors.partition_point(|a| {
                if a.len <= 40 {
                    a.key[..a.len as usize] < *start
                } else {
                    self.key(version, a.position as usize).cmp_bytes(start) == Ordering::Less
                }
            });
            pos = pos.max(a.saturating_sub(1) * STRIDE);
            while pos < hi && self.key(version, pos).cmp_bytes(start) == Ordering::Less {
                pos += 1;
            }
        }
        Some(Cursor {
            remix: self,
            version,
            pos,
            hi,
            end,
            prefix_end: false,
            bounded,
            limit,
            direct: if start == prefix { span } else { 0 },
            first,
        })
    }
    fn key<'a>(&self, version: &'a Version, pos: usize) -> ScanKey<'a> {
        let (run, offset) = unpack(self.locators[pos]);
        version.tables[run]
            .table
            .scan_index()
            .unwrap()
            .scan_key(offset)
    }
}
pub(crate) struct Cursor<'a> {
    remix: &'a Remix,
    version: &'a Version,
    pos: usize,
    hi: usize,
    end: &'a [u8],
    prefix_end: bool,
    bounded: bool,
    limit: u64,
    direct: usize,
    first: u64,
}
impl<'a> Cursor<'a> {
    pub(crate) fn contains_mem_key(&self, key: &[u8]) -> bool {
        if self.prefix_end {
            key.starts_with(self.end)
        } else {
            key < self.end
        }
    }
    pub(crate) fn key(&self) -> Option<ScanKey<'a>> {
        (self.pos < self.hi
            && (self.bounded
                || self.remix.key(self.version, self.pos).cmp_bytes(self.end) == Ordering::Less))
            .then(|| self.remix.key(self.version, self.pos))
    }
    pub(crate) fn pop(&mut self) -> Option<ScanValue<'a>> {
        if self.pos < self.hi {
            if !self.bounded
                && self.remix.key(self.version, self.pos).cmp_bytes(self.end) != Ordering::Less
            {
                self.pos = self.hi;
                return None;
            }
            let mut best: Option<ScanValue<'a>> = None;
            let begin = self.pos;
            loop {
                let loc = self.remix.locators[self.pos];
                let (run, offset) = unpack(loc);
                let value = self.version.tables[run]
                    .table
                    .scan_index()
                    .unwrap()
                    .scan_value(offset, self.limit)
                    .1;
                if let Some(v) = value
                    && best.is_none_or(|b| v.seq > b.seq)
                {
                    best = Some(v);
                }
                self.pos += 1;
                if loc & CONTINUES == 0 {
                    break;
                }
            }
            self.direct = self.direct.saturating_sub(self.pos - begin);
            if self.direct > 0 {
                self.first = self.remix.locators[self.pos];
            }
            return best;
        }
        None
    }
    pub(crate) fn direct_values<T, E>(
        &mut self,
        count: usize,
        map: &mut impl FnMut(&[u8]) -> Result<T, E>,
        out: &mut Vec<T>,
    ) -> Result<(), E> {
        if self.direct == 0 || !self.bounded {
            return Ok(());
        }
        let n = count.min(self.direct);
        let (run, offset) = unpack(self.first);
        let index = self.version.tables[run].table.scan_index().unwrap();
        index.scan_values(offset, n, self.limit, map, out)?;
        self.pos += n;
        self.direct = 0;
        Ok(())
    }
}
