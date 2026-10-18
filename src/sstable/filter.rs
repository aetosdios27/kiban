//! Four-wise binary fuse8, with geometry/addresses from FastFilter's
//! 4wise_xor_binary_fuse_filter_lowmem.h (Apache-2.0, licenses/LICENSE.FastFilter).
//! Copyright 2018 the fastfilter_cpp authors.
//! Ordinary degree/xor peeling follows the isolated Kiban filter experiment.
//! The 48-byte revision-1 header, graph mixer and XXH3-64 hash are on-disk format.
use crate::{bloom::BloomFilter, xxh3};

pub(super) const BLOCK_TYPE_FILTER: u8 = 1;
const HEADER: usize = 48;
const MAGIC: &[u8; 8] = b"KIBANFL2";
const RETRIES: u64 = 100;
// Workspace above retained input hashes. Bound before any graph allocation.
const WORKSPACE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(super) enum SstFilter {
    Legacy(BloomFilter),
    Fuse(Fuse8),
    Positive,
}

#[derive(Debug)]
pub(super) struct Fuse8 {
    fp: Vec<u8>,
    segment: usize,
    count_length: usize,
    seed: u64,
    keys: u64,
}

#[inline]
fn mix(mut h: u64) -> u64 {
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
    h ^ (h >> 31)
}
impl Fuse8 {
    #[inline]
    fn positions(&self, h: u64) -> [usize; 4] {
        let a = ((u128::from(h) * self.count_length as u128) >> 64) as usize;
        [
            a,
            (a + self.segment) ^ (h as usize & (self.segment - 1)),
            (a + 2 * self.segment) ^ ((h >> 16) as usize & (self.segment - 1)),
            (a + 3 * self.segment) ^ ((h >> 32) as usize & (self.segment - 1)),
        ]
    }
    #[inline]
    fn contains_hash(&self, raw: u64) -> bool {
        let h = mix(raw.wrapping_add(self.seed));
        let p = self.positions(h);
        self.fp[p[0]] ^ self.fp[p[1]] ^ self.fp[p[2]] ^ self.fp[p[3]] == h as u8
    }
    fn build(hashes: &mut [u64], retries: u64, budget: usize) -> Option<Self> {
        if hashes.is_empty() || hashes.len() > u32::MAX as usize {
            return None;
        }
        // Sorted input keys are distinct. Equal hashes therefore mean a collision.
        hashes.sort_unstable();
        if hashes.windows(2).any(|h| h[0] == h[1]) {
            return None;
        }
        let n = hashes.len().max(2) as f64;
        let segment = (1usize << (n.ln() / 2.91f64.ln() - 0.5).floor().max(0.) as u32).min(262144);
        let factor = 1.075f64.max(0.77 + 0.305 * 600000f64.ln() / n.ln());
        let segments = ((n * factor).ceil() as usize).div_ceil(segment).max(4);
        let cells = segments.checked_mul(segment)?;
        // Fingerprints + degrees + xors + queue (at most cells + 4n pushes)
        // + reverse peel stack (Rust tuple size includes padding).
        let bytes = cells
            .checked_mul(17)?
            .checked_add(hashes.len().checked_mul(32)?)?;
        if cells > u32::MAX as usize || bytes > budget {
            return None;
        }
        let mut f = Self {
            fp: vec![0; cells],
            segment,
            count_length: (segments - 3) * segment,
            seed: 0,
            keys: hashes.len() as u64,
        };
        let mut degree = vec![0u32; cells];
        let mut xor = vec![0u64; cells];
        let mut queue = Vec::with_capacity(cells);
        let mut stack = Vec::with_capacity(hashes.len());
        for attempt in 1..=retries {
            f.seed = mix(attempt);
            degree.fill(0);
            xor.fill(0);
            queue.clear();
            stack.clear();
            for &raw in hashes.iter() {
                let h = mix(raw.wrapping_add(f.seed));
                for p in f.positions(h) {
                    degree[p] = degree[p].checked_add(1)?;
                    xor[p] ^= h;
                }
            }
            queue.extend(
                degree
                    .iter()
                    .enumerate()
                    .filter(|&(_, &d)| d == 1)
                    .map(|(i, _)| i as u32),
            );
            while let Some(p) = queue.pop() {
                let p = p as usize;
                if degree[p] != 1 {
                    continue;
                }
                let h = xor[p];
                stack.push((h, p as u32));
                for q in f.positions(h) {
                    degree[q] -= 1;
                    xor[q] ^= h;
                    if degree[q] == 1 {
                        queue.push(q as u32);
                    }
                }
            }
            if stack.len() != hashes.len() {
                continue;
            }
            for &(h, p) in stack.iter().rev() {
                let mut x = h as u8;
                for q in f.positions(h) {
                    x ^= f.fp[q];
                }
                f.fp[p as usize] = x;
            }
            // In release too: a faulty graph can never become an absence proof.
            if hashes.iter().all(|&h| f.contains_hash(h)) {
                return Some(f);
            }
            return None;
        }
        None
    }
}
impl SstFilter {
    pub(super) fn build(hashes: &mut [u64]) -> Self {
        Fuse8::build(hashes, RETRIES, WORKSPACE_BYTES).map_or(Self::Positive, Self::Fuse)
    }
    #[inline]
    pub(super) fn may_contain(&self, key: &[u8]) -> bool {
        match self {
            Self::Legacy(f) => f.may_contain(key),
            Self::Fuse(f) => f.contains_hash(xxh3::hash(key)),
            Self::Positive => true,
        }
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = vec![0; HEADER];
        out[..8].copy_from_slice(MAGIC);
        out[8] = 1;
        out[12..16].copy_from_slice(&(HEADER as u32).to_le_bytes());
        if let Self::Fuse(f) = self {
            out[9..12].copy_from_slice(&[2, 1, 8]);
            out[16..24].copy_from_slice(&f.keys.to_le_bytes());
            out[24..32].copy_from_slice(&f.seed.to_le_bytes());
            for (at, value) in [
                (32, f.segment),
                (36, f.count_length / f.segment),
                (40, f.fp.len()),
                (44, f.fp.len()),
            ] {
                out[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
            }
            out.extend_from_slice(&f.fp);
        } else {
            assert!(matches!(self, Self::Positive));
        }
        out
    }
    // Caller must verify the block type and CRC before decoding.
    pub(super) fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER
            || &b[..8] != MAGIC
            || b[8] != 1
            || b[12..16] != (HEADER as u32).to_le_bytes()
        {
            return None;
        }
        if b[9] == 0 {
            return (b.len() == HEADER && b[10..12] == [0, 0] && b[16..].iter().all(|&b| b == 0))
                .then_some(Self::Positive);
        }
        if b[9..12] != [2, 1, 8] {
            return None;
        }
        let keys = u64::from_le_bytes(b[16..24].try_into().ok()?);
        let seed = u64::from_le_bytes(b[24..32].try_into().ok()?);
        let word = |at| u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let segment = word(32);
        let count = word(36);
        let cells = word(40);
        if keys == 0
            || keys > u64::from(u32::MAX)
            || !segment.is_power_of_two()
            || segment > 262144
            || count == 0
            || count.checked_add(3)?.checked_mul(segment)? != cells
            || word(44) != cells
            || HEADER.checked_add(cells)? != b.len()
        {
            return None;
        }
        Some(Self::Fuse(Fuse8 {
            fp: b[HEADER..].to_vec(),
            segment,
            count_length: count.checked_mul(segment)?,
            seed,
            keys,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_members_random_and_adversarial_roundtrip() {
        for seed in 0..40u64 {
            for count in [0, 1, 2, 3, 17, 100, 1000, 12000] {
                let keys: Vec<Vec<u8>> = (0..count)
                    .map(|i| {
                        let h = mix(seed ^ i as u64);
                        match seed % 4 {
                            0 => h.to_le_bytes().to_vec(),
                            1 => [b"shared/agent/memory/".as_slice(), &h.to_be_bytes()].concat(),
                            2 => [vec![0xff; 1024], h.to_be_bytes().to_vec()].concat(),
                            _ => vec![i as u8; i % 256],
                        }
                    })
                    .collect();
                // This API receives distinct user-key hashes. Deliberate equal
                // hashes exercise safe collision fallback as well as duplicates.
                let mut hashes: Vec<_> = keys.iter().map(|k| xxh3::hash(k)).collect();
                let f = SstFilter::build(&mut hashes);
                let decoded = SstFilter::decode(&f.encode()).unwrap();
                for k in &keys {
                    assert!(decoded.may_contain(k), "seed={seed} count={count}");
                }
            }
        }
    }
    #[test]
    fn two_million_distinct_members_have_no_false_negatives() {
        for domain in [0u64, 1] {
            let keys: Vec<_> = (0..1_000_000u64)
                .map(|i| {
                    [
                        b"shared/agent/memory/".as_slice(),
                        &[domain as u8],
                        &mix(i).to_be_bytes(),
                    ]
                    .concat()
                })
                .collect();
            let mut hashes: Vec<_> = keys.iter().map(|k| xxh3::hash(k)).collect();
            let f = SstFilter::build(&mut hashes);
            assert!(
                matches!(f, SstFilter::Fuse(_)),
                "property must exercise the graph"
            );
            let decoded = SstFilter::decode(&f.encode()).unwrap();
            for key in keys {
                assert!(decoded.may_contain(&key));
            }
        }
    }
    #[test]
    fn exhausted_seeds_collision_and_workspace_fall_back_positive() {
        for (mut hashes, retries, budget) in [
            (vec![1, 2], 0, usize::MAX),
            (vec![1, 1], 100, usize::MAX),
            (vec![1, 2], 100, 0),
            (vec![], 100, usize::MAX),
        ] {
            assert!(Fuse8::build(&mut hashes, retries, budget).is_none());
        }
        let f = SstFilter::decode(&SstFilter::Positive.encode()).unwrap();
        assert!(f.may_contain(b"anything"));
    }
    #[test]
    fn rejects_truncation_and_impossible_dimensions() {
        let f = SstFilter::build(&mut (0..100).collect::<Vec<_>>()).encode();
        for len in 0..f.len() {
            assert!(SstFilter::decode(&f[..len]).is_none());
        }
        for (at, value) in [
            (8, 2),
            (9, 1),
            (10, 0),
            (11, 16),
            (12, 0),
            (32, 3),
            (36, 0),
            (40, 0),
            (44, 0),
        ] {
            let mut b = f.clone();
            b[at] = value;
            assert!(SstFilter::decode(&b).is_none(), "offset={at}");
        }
    }
}
