//! CRC-32/IEEE folding adapted from crc32fast 1.5.0's pclmulqdq backend.
//! Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors.
//! MIT license and attribution: licenses/crc32fast-MIT.txt.
//!
//! This entry point accepts and returns Kiban's unfinalized streaming state.
//! The reflected IEEE polynomial is unchanged; this is not SSE4.2 CRC-32C.

use std::arch::x86_64 as arch;

const K1: i64 = 0x154442bd4;
const K2: i64 = 0x1c6e41596;
const K3: i64 = 0x1751997d0;
const K4: i64 = 0x0ccaa009e;
const K5: i64 = 0x163cd6124;
const POLYNOMIAL: i64 = 0x1DB710641;
const RECIPROCAL: i64 = 0x1F7011641;

/// Requires PCLMULQDQ, SSE4.1, and at least 128 readable input bytes.
#[target_feature(enable = "pclmulqdq,sse4.1")]
pub(super) unsafe fn update(state: u32, mut data: &[u8]) -> u32 {
    debug_assert!(data.len() >= 128);
    // SAFETY: The caller checks CPU features and the initial length. Each
    // take16 is either within that first 64 bytes or protected by a loop
    // length check. fold requires the same enabled features as this function.
    let mut x = unsafe {
        let mut x3 = take16(&mut data);
        let mut x2 = take16(&mut data);
        let mut x1 = take16(&mut data);
        let mut x0 = take16(&mut data);
        x3 = arch::_mm_xor_si128(x3, arch::_mm_cvtsi32_si128(state as i32));
        let keys = arch::_mm_set_epi64x(K2, K1);
        while data.len() >= 64 {
            x3 = fold(x3, take16(&mut data), keys);
            x2 = fold(x2, take16(&mut data), keys);
            x1 = fold(x1, take16(&mut data), keys);
            x0 = fold(x0, take16(&mut data), keys);
        }
        let keys = arch::_mm_set_epi64x(K4, K3);
        let mut x = fold(x3, x2, keys);
        x = fold(x, x1, keys);
        x = fold(x, x0, keys);
        while data.len() >= 16 {
            x = fold(x, take16(&mut data), keys);
        }
        x
    };

    // Fold the 128-bit accumulator to 64 bits, then perform the reflected
    // Barrett reduction to the unfinalized 32-bit IEEE state.
    let keys = arch::_mm_set_epi64x(K4, K3);
    x = arch::_mm_xor_si128(
        arch::_mm_clmulepi64_si128(x, keys, 0x10),
        arch::_mm_srli_si128(x, 8),
    );
    x = arch::_mm_xor_si128(
        arch::_mm_clmulepi64_si128(
            arch::_mm_and_si128(x, arch::_mm_set_epi32(0, 0, 0, !0)),
            arch::_mm_set_epi64x(0, K5),
            0x00,
        ),
        arch::_mm_srli_si128(x, 4),
    );
    let keys = arch::_mm_set_epi64x(RECIPROCAL, POLYNOMIAL);
    let t1 = arch::_mm_clmulepi64_si128(
        arch::_mm_and_si128(x, arch::_mm_set_epi32(0, 0, 0, !0)),
        keys,
        0x10,
    );
    let t2 = arch::_mm_clmulepi64_si128(
        arch::_mm_and_si128(t1, arch::_mm_set_epi32(0, 0, 0, !0)),
        keys,
        0x00,
    );
    let state = arch::_mm_extract_epi32(arch::_mm_xor_si128(x, t2), 1) as u32;
    super::update_scalar(state, data)
}

#[inline]
#[target_feature(enable = "pclmulqdq")]
unsafe fn fold(a: arch::__m128i, b: arch::__m128i, keys: arch::__m128i) -> arch::__m128i {
    let low = arch::_mm_clmulepi64_si128(a, keys, 0x00);
    let high = arch::_mm_clmulepi64_si128(a, keys, 0x11);
    arch::_mm_xor_si128(arch::_mm_xor_si128(b, low), high)
}

#[inline]
unsafe fn take16(data: &mut &[u8]) -> arch::__m128i {
    let (head, rest) = data.split_at(16);
    // SAFETY: split_at establishes 16 readable bytes. loadu does not require
    // alignment, and x86_64 always supports SSE2.
    let value = unsafe { arch::_mm_loadu_si128(head.as_ptr().cast()) };
    *data = rest;
    value
}
