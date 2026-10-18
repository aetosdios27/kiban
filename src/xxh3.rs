//! Seed-zero, default-secret XXH3-64. Safe scalar port of xxHash v0.8.3,
//! https://github.com/Cyan4973/xxHash/blob/v0.8.3/xxhash.h
//! Copyright (c) 2012-2023 Yann Collet. BSD-2-Clause; see licenses/LICENSE.XXHash.
//! Hash bytes and wrapping operations are part of SST filter revision 1.

const P1: u64 = 0x9e3779b185ebca87;
const P2: u64 = 0xc2b2ae3d27d4eb4f;
const P3: u64 = 0x165667b19e3779f9;
const P4: u64 = 0x85ebca77c2b2ae63;
const P5: u64 = 0x27d4eb2f165667c5;
const SECRET: [u8; 192] = [
    0xb8, 0xfe, 0x6c, 0x39, 0x23, 0xa4, 0x4b, 0xbe, 0x7c, 0x01, 0x81, 0x2c, 0xf7, 0x21, 0xad, 0x1c,
    0xde, 0xd4, 0x6d, 0xe9, 0x83, 0x90, 0x97, 0xdb, 0x72, 0x40, 0xa4, 0xa4, 0xb7, 0xb3, 0x67, 0x1f,
    0xcb, 0x79, 0xe6, 0x4e, 0xcc, 0xc0, 0xe5, 0x78, 0x82, 0x5a, 0xd0, 0x7d, 0xcc, 0xff, 0x72, 0x21,
    0xb8, 0x08, 0x46, 0x74, 0xf7, 0x43, 0x24, 0x8e, 0xe0, 0x35, 0x90, 0xe6, 0x81, 0x3a, 0x26, 0x4c,
    0x3c, 0x28, 0x52, 0xbb, 0x91, 0xc3, 0x00, 0xcb, 0x88, 0xd0, 0x65, 0x8b, 0x1b, 0x53, 0x2e, 0xa3,
    0x71, 0x64, 0x48, 0x97, 0xa2, 0x0d, 0xf9, 0x4e, 0x38, 0x19, 0xef, 0x46, 0xa9, 0xde, 0xac, 0xd8,
    0xa8, 0xfa, 0x76, 0x3f, 0xe3, 0x9c, 0x34, 0x3f, 0xf9, 0xdc, 0xbb, 0xc7, 0xc7, 0x0b, 0x4f, 0x1d,
    0x8a, 0x51, 0xe0, 0x4b, 0xcd, 0xb4, 0x59, 0x31, 0xc8, 0x9f, 0x7e, 0xc9, 0xd9, 0x78, 0x73, 0x64,
    0xea, 0xc5, 0xac, 0x83, 0x34, 0xd3, 0xeb, 0xc3, 0xc5, 0x81, 0xa0, 0xff, 0xfa, 0x13, 0x63, 0xeb,
    0x17, 0x0d, 0xdd, 0x51, 0xb7, 0xf0, 0xda, 0x49, 0xd3, 0x16, 0x55, 0x26, 0x29, 0xd4, 0x68, 0x9e,
    0x2b, 0x16, 0xbe, 0x58, 0x7d, 0x47, 0xa1, 0xfc, 0x8f, 0xf8, 0xb8, 0xd1, 0x7a, 0xd0, 0x31, 0xce,
    0x45, 0xcb, 0x3a, 0x8f, 0x95, 0x16, 0x04, 0x28, 0xaf, 0xd7, 0xfb, 0xca, 0xbb, 0x4b, 0x40, 0x7e,
];

#[inline]
fn read64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
#[inline]
fn read32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
#[inline]
fn fold(a: u64, b: u64) -> u64 {
    let p = u128::from(a) * u128::from(b);
    p as u64 ^ (p >> 64) as u64
}
#[inline]
fn avalanche(mut h: u64) -> u64 {
    h ^= h >> 37;
    h = h.wrapping_mul(0x165667919e3779f9);
    h ^ (h >> 32)
}
fn avalanche64(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}
#[inline]
fn mix16(b: &[u8], at: usize, secret: usize) -> u64 {
    fold(
        read64(b, at) ^ read64(&SECRET, secret),
        read64(b, at + 8) ^ read64(&SECRET, secret + 8),
    )
}

/// Identical to XXH3_64bits, for every slice length, on either byte order.
#[inline]
pub(crate) fn hash(b: &[u8]) -> u64 {
    let n = b.len();
    if n <= 16 {
        if n > 8 {
            let lo = read64(b, 0) ^ (read64(&SECRET, 24) ^ read64(&SECRET, 32));
            let hi = read64(b, n - 8) ^ (read64(&SECRET, 40) ^ read64(&SECRET, 48));
            return avalanche(
                (n as u64)
                    .wrapping_add(lo.swap_bytes())
                    .wrapping_add(hi)
                    .wrapping_add(fold(lo, hi)),
            );
        }
        if n >= 4 {
            let input = u64::from(read32(b, n - 4)) | (u64::from(read32(b, 0)) << 32);
            let mut h = input ^ (read64(&SECRET, 8) ^ read64(&SECRET, 16));
            h ^= h.rotate_left(49) ^ h.rotate_left(24);
            h = h.wrapping_mul(0x9fb21c651e98df25);
            h ^= (h >> 35).wrapping_add(n as u64);
            h = h.wrapping_mul(0x9fb21c651e98df25);
            return h ^ (h >> 28);
        }
        if n > 0 {
            let combined = (u32::from(b[0]) << 16)
                | (u32::from(b[n >> 1]) << 24)
                | u32::from(b[n - 1])
                | ((n as u32) << 8);
            return avalanche64(u64::from(
                combined ^ read32(&SECRET, 0) ^ read32(&SECRET, 4),
            ));
        }
        return avalanche64(read64(&SECRET, 56) ^ read64(&SECRET, 64));
    }
    if n <= 128 {
        let mut acc = (n as u64).wrapping_mul(P1);
        for i in 0..=(n - 1) / 32 {
            acc = acc.wrapping_add(mix16(b, 16 * i, 32 * i));
            acc = acc.wrapping_add(mix16(b, n - 16 * (i + 1), 32 * i + 16));
        }
        return avalanche(acc);
    }
    if n <= 240 {
        let mut acc = (n as u64).wrapping_mul(P1);
        for i in 0..8 {
            acc = acc.wrapping_add(mix16(b, i * 16, i * 16));
        }
        acc = avalanche(acc);
        let mut end = mix16(b, n - 16, 136 - 17);
        for i in 8..n / 16 {
            end = end.wrapping_add(mix16(b, i * 16, 16 * (i - 8) + 3));
        }
        return avalanche(acc.wrapping_add(end));
    }
    hash_long(b)
}

fn accumulate(acc: &mut [u64; 8], b: &[u8], at: usize, secret: usize) {
    for lane in 0..8 {
        let data = read64(b, at + lane * 8);
        let keyed = data ^ read64(&SECRET, secret + lane * 8);
        acc[lane ^ 1] = acc[lane ^ 1].wrapping_add(data);
        acc[lane] = acc[lane].wrapping_add(u64::from(keyed as u32) * (keyed >> 32));
    }
}
fn hash_long(b: &[u8]) -> u64 {
    let mut acc = [0xc2b2ae3d, P1, P2, P3, P4, 0x85ebca77, P5, 0x9e3779b1];
    let blocks = (b.len() - 1) / 1024;
    for block in 0..blocks {
        for stripe in 0..16 {
            accumulate(&mut acc, b, block * 1024 + stripe * 64, stripe * 8);
        }
        for (lane, a) in acc.iter_mut().enumerate() {
            *a = (*a ^ (*a >> 47) ^ read64(&SECRET, 128 + lane * 8)).wrapping_mul(0x9e3779b1);
        }
    }
    for stripe in 0..(b.len() - 1 - blocks * 1024) / 64 {
        accumulate(&mut acc, b, blocks * 1024 + stripe * 64, stripe * 8);
    }
    accumulate(&mut acc, b, b.len() - 64, 121);
    let mut result = (b.len() as u64).wrapping_mul(P1);
    for i in 0..4 {
        result = result.wrapping_add(fold(
            acc[2 * i] ^ read64(&SECRET, 11 + 16 * i),
            acc[2 * i + 1] ^ read64(&SECRET, 19 + 16 * i),
        ));
    }
    avalanche(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_vectors_all_lengths_and_boundaries() {
        // Generated by validation/fuse-engine/xxh3-vectors.c against upstream
        // xxHash v0.8.3. Includes unaligned slices and long stripe/block edges.
        for line in include_str!("../validation/fuse-engine/xxh3-vectors.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let n: usize = fields[0].parse().unwrap();
            let pattern: u32 = fields[1].parse().unwrap();
            let expected = u64::from_str_radix(fields[2], 16).unwrap();
            let mut bytes = vec![0; n + 1];
            for (i, b) in bytes[1..].iter_mut().enumerate() {
                *b = (i as u32 * 131 + pattern * 17 + (i as u32 >> 3)) as u8;
            }
            assert_eq!(hash(&bytes[1..]), expected, "length={n} pattern={pattern}");
        }
    }
}
