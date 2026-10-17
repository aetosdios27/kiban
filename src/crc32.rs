//! CRC-32/IEEE checksum.
//!
//! Implements the decision in `ARCHITECTURE.md` (D2):
//! runtime-dispatched PCLMUL folding for long x86_64 inputs and a portable
//! slicing-by-eight fallback, without external dependencies. Published check
//! values and an independent bitwise reference verify the polynomial,
//! streaming state and stored checksum bytes.

#[cfg(target_arch = "x86_64")]
#[path = "crc32_pclmul.rs"]
mod pclmul;

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

const fn make_slices() -> [[u32; 256]; 8] {
    let mut slices = [[0; 256]; 8];
    slices[0] = make_table();
    let mut slice = 1;
    while slice < 8 {
        let mut byte = 0;
        while byte < 256 {
            let previous = slices[slice - 1][byte];
            slices[slice][byte] = slices[0][(previous & 255) as usize] ^ (previous >> 8);
            byte += 1;
        }
        slice += 1;
    }
    slices
}

static SLICES: [[u32; 256]; 8] = make_slices();

pub const INITIAL: u32 = 0xFFFF_FFFF;

pub fn update(state: u32, data: &[u8]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    if data.len() >= 128
        && std::is_x86_feature_detected!("pclmulqdq")
        && std::is_x86_feature_detected!("sse4.1")
    {
        // SAFETY: Both required features and the minimum input length are
        // checked here. x86_64 guarantees SSE2; unaligned input is supported.
        return unsafe { pclmul::update(state, data) };
    }
    update_scalar(state, data)
}

fn update_scalar(state: u32, data: &[u8]) -> u32 {
    let mut c = state;
    let (chunks, remainder) = data.as_chunks::<8>();
    for bytes in chunks {
        // Explicit little-endian loads work on every host and require no
        // alignment or unsafe pointer reads. Each slice accounts for the
        // remaining zero-byte advances after its input byte.
        let word = u64::from_le_bytes(*bytes) ^ u64::from(c);
        c = SLICES[7][(word & 255) as usize]
            ^ SLICES[6][((word >> 8) & 255) as usize]
            ^ SLICES[5][((word >> 16) & 255) as usize]
            ^ SLICES[4][((word >> 24) & 255) as usize]
            ^ SLICES[3][((word >> 32) & 255) as usize]
            ^ SLICES[2][((word >> 40) & 255) as usize]
            ^ SLICES[1][((word >> 48) & 255) as usize]
            ^ SLICES[0][(word >> 56) as usize];
    }
    for &b in remainder {
        c = SLICES[0][((c ^ b as u32) & 255) as usize] ^ (c >> 8);
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

    fn bitwise(mut state: u32, data: &[u8]) -> u32 {
        for &byte in data {
            state ^= u32::from(byte);
            for _ in 0..8 {
                state = if state & 1 == 0 {
                    state >> 1
                } else {
                    (state >> 1) ^ POLY
                };
            }
        }
        state
    }

    #[test]
    fn sliced_matches_bitwise_for_lengths_alignments_and_streaming_states() {
        let data: Vec<u8> = (0..65560usize)
            .map(|i| ((i.wrapping_mul(7919) ^ (i >> 3)) & 255) as u8)
            .collect();
        for offset in 0..16 {
            for len in (0..=256).chain([511, 512, 513, 4095, 4096, 4097, 65535]) {
                let bytes = &data[offset..offset + len];
                for state in [INITIAL, 0, 0x1234_5678] {
                    assert_eq!(
                        update(state, bytes),
                        bitwise(state, bytes),
                        "offset={offset} len={len} state={state}"
                    );
                }
            }
        }
        for size in [1, 3, 7, 8, 9, 31, 255, 4096] {
            let state = data.chunks(size).fold(INITIAL, update);
            assert_eq!(state, bitwise(INITIAL, &data), "stream chunk={size}");
        }
    }

    #[test]
    fn dispatched_updates_match_scalar_across_fold_boundaries() {
        let data: Vec<_> = (0..8256usize)
            .map(|i| (i.wrapping_mul(7919) ^ (i >> 3)) as u8)
            .collect();
        for offset in 0..64 {
            for len in (112..=272).chain([511, 512, 513, 4095, 4096, 4097, 8192]) {
                for state in [INITIAL, 0, 0x1234_5678] {
                    let bytes = &data[offset..offset + len];
                    assert_eq!(
                        update(state, bytes),
                        update_scalar(state, bytes),
                        "offset={offset} len={len} state={state}"
                    );
                }
            }
        }
    }

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
