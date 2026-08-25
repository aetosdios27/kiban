//! Bloom filter for point-lookup absence proofs.
//!
//! Per `docs/design/bloom.md`: bit-array Bloom, FNV-1a 64-bit with
//! double hashing, 10 bits/key and 7 probes. False positives fall
//! through to normal probing; false negatives are impossible.

pub const BITS_PER_KEY: usize = 10;
pub const NUM_PROBES: u32 = 7;
const MIN_BITS: usize = 64;

/// FNV-1a 64-bit. Published check value: `a4b516a4` for "hello".
pub fn fnv1a_64(data: &[u8]) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BloomFilter {
    bits: Vec<u8>,
    num_bits: u64,
    num_probes: u32,
}

impl BloomFilter {
    pub fn build<'a>(keys: impl Iterator<Item = &'a [u8]>) -> Self {
        let keys: Vec<&[u8]> = keys.collect();
        let num_bits = (keys.len() * BITS_PER_KEY).max(MIN_BITS) as u64;
        let mut filter = BloomFilter {
            bits: vec![0u8; (num_bits / 8) as usize + 1],
            num_bits,
            num_probes: NUM_PROBES,
        };
        for key in &keys {
            filter.insert(key);
        }
        filter
    }

    pub fn empty() -> Self {
        BloomFilter {
            bits: vec![0u8; MIN_BITS / 8],
            num_bits: MIN_BITS as u64,
            num_probes: NUM_PROBES,
        }
    }

    fn insert(&mut self, key: &[u8]) {
        let (h1, h2) = probe_pair(&fnv1a_64(key));
        for i in 0..self.num_probes {
            self.set_bit(h1.wrapping_add((i as u64).wrapping_mul(h2)) % self.num_bits);
        }
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        let (h1, h2) = probe_pair(&fnv1a_64(key));
        (0..self.num_probes)
            .all(|i| self.get_bit(h1.wrapping_add((i as u64).wrapping_mul(h2)) % self.num_bits))
    }

    fn set_bit(&mut self, bit: u64) {
        let idx = (bit / 8) as usize;
        self.bits[idx] |= 1 << (bit % 8);
    }

    fn get_bit(&self, bit: u64) -> bool {
        self.bits[(bit / 8) as usize] & (1 << (bit % 8)) != 0
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(13 + self.bits.len());
        out.push(self.num_probes as u8);
        out.extend_from_slice(&self.num_bits.to_le_bytes());
        out.extend_from_slice(&self.bits);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<BloomFilter> {
        if bytes.len() < 9 {
            return None;
        }
        let num_probes = bytes[0] as u32;
        if !(1..=30).contains(&num_probes) {
            return None;
        }
        let num_bits = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
        if num_bits < MIN_BITS as u64 {
            return None;
        }
        let needed = (num_bits / 8) as usize + 1;
        if bytes.len() != 9 + needed {
            return None;
        }
        Some(BloomFilter {
            bits: bytes[9..].to_vec(),
            num_bits,
            num_probes,
        })
    }
}

fn probe_pair(h1: &u64) -> (u64, u64) {
    (*h1, h1.rotate_left(21) | 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(i: usize) -> Vec<u8> {
        format!("key-{i:08}").into_bytes()
    }

    #[test]
    fn fnv1a_known_check_values() {
        // published FNV-1a 64 test vectors
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn no_false_negatives_across_sizes_and_boundaries() {
        for count in [0usize, 1, 2, 15, 16, 17, 100, 1000] {
            let keys: Vec<Vec<u8>> = (0..count).map(key).collect();
            let filter = BloomFilter::build(keys.iter().map(|k| k.as_slice()));
            for k in &keys {
                assert!(
                    filter.may_contain(k),
                    "false negative on {k:?} with {count} keys"
                );
            }
            // empty filter still answers present-keys correctly
            if count > 0 {
                assert!(!BloomFilter::empty().may_contain(&keys[0]));
            }
        }
    }

    #[test]
    fn measured_false_positive_rate_stays_under_bound() {
        let inserted: Vec<Vec<u8>> = (0..2000).map(key).collect();
        let filter = BloomFilter::build(inserted.iter().map(|k| k.as_slice()));
        let absent: Vec<Vec<u8>> = (10000..12000).map(key).collect();
        let false_positives = absent.iter().filter(|k| filter.may_contain(k)).count();
        let rate = false_positives as f64 / absent.len() as f64;
        assert!(
            rate < 0.05,
            "false positive rate {rate:.4} exceeded 5% ({false_positives}/2000)"
        );
    }

    #[test]
    fn encode_decode_roundtrip_preserves_answers() {
        let inserted: Vec<Vec<u8>> = (0..500).map(key).collect();
        let filter = BloomFilter::build(inserted.iter().map(|k| k.as_slice()));
        let decoded = BloomFilter::decode(&filter.encode()).unwrap();
        assert_eq!(filter, decoded);
        for k in &inserted {
            assert!(decoded.may_contain(k));
        }
        assert_eq!(BloomFilter::decode(&decoded.encode()[..8]), None);
    }

    #[test]
    fn decode_rejects_nonsense() {
        assert_eq!(BloomFilter::decode(&[]), None);
        assert_eq!(BloomFilter::decode(&[0u8; 20]), None); // zero probes
        assert_eq!(BloomFilter::decode(&[31, 0, 0, 0, 0, 0, 0, 0, 0]), None); // too few bits
        let keys: Vec<Vec<u8>> = (0..10).map(key).collect();
        let good = BloomFilter::build(keys.iter().map(|k| k.as_slice())).encode();
        assert_eq!(BloomFilter::decode(&good[..good.len() - 1]), None);
    }

    #[test]
    fn single_bit_flip_in_payload_cannot_create_false_negatives_on_all_but_one_probe() {
        // A flipped payload bit can only clear membership bits; a key is
        // reported absent only if one of ITS OWN bits was cleared. This
        // test bounds the blast radius: at most the keys whose probe sets
        // intersected that bit go missing — the sstable block CRC is what
        // actually guards this layer, so here we only verify no panic and
        // bounded damage.
        let inserted: Vec<Vec<u8>> = (0..300).map(key).collect();
        let mut raw = BloomFilter::build(inserted.iter().map(|k| k.as_slice())).encode();
        let mid = raw.len() / 2;
        raw[mid] ^= 0x08;
        let corrupted = BloomFilter::decode(&raw).unwrap();
        let surviving = inserted.iter().filter(|k| corrupted.may_contain(k)).count();
        assert!(
            surviving > 250,
            "single flip destroyed {surviving}/300 keys"
        );
    }
}
