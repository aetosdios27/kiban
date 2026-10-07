//! CRC-32/IEEE checksum.
//!
//! Implements the decision in `docs/design/record-framing.md` (D2):
//! table-driven CRC-32/IEEE, no external dependencies, verified against
//! published check values.

const POLY: u32 = 0xEDB8_8320;

const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { POLY ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static TABLE: [u32; 256] = make_table();

pub const INITIAL: u32 = 0xFFFF_FFFF;

pub fn update(state: u32, data: &[u8]) -> u32 {
    let mut c = state;
    for &b in data {
        c = TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c
}

pub fn finalize(state: u32) -> u32 {
    !state
}

pub fn crc32(data: &[u8]) -> u32 {
    finalize(update(INITIAL, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_check_values() {
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data = b"split me across several updates to verify streaming state";
        let one_shot = crc32(data);
        let streamed = finalize(update(update(INITIAL, &data[..7]), &data[7..]));
        assert_eq!(one_shot, streamed);
    }

    #[test]
    fn every_single_byte_flip_is_detected() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        let good = crc32(&data);
        for i in 0..data.len() {
            let mut bad = data.clone();
            bad[i] ^= 0x01;
            assert_ne!(crc32(&bad), good, "undetected flip at byte {i}");
        }
    }

    #[test]
    fn every_two_bit_flip_within_a_word_is_detected() {
        let data = [0u8; 64];
        let good = crc32(&data);
        for a in 0..64usize {
            for b in (a + 1)..64 {
                let mut bad = data;
                bad[a] ^= 0x80;
                bad[b] ^= 0x80;
                assert_ne!(crc32(&bad), good, "undetected double flip at {a},{b}");
            }
        }
    }
}
