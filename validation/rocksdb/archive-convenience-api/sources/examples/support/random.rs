// Counter-based PRNG: operation choice/key depend on (seed, stage, operation ID),
// not OS scheduling or other threads' progress. Arithmetic is explicitly wrapping.
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
pub fn value(seed: u64, size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| mix(seed.wrapping_add(i as u64)) as u8)
        .collect()
}

/// Stable per-key pseudorandom bytes for separately labeled large-memory tests.
pub fn keyed_value(seed: u64, key: u64, output: &mut [u8]) {
    let seed = seed ^ mix(key);
    for (i, chunk) in output.chunks_mut(8).enumerate() {
        let word = mix(seed ^ i as u64).to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyed_values_are_stable_distinct_and_prefix_consistent() {
        for size in [0, 1, 7, 8, 9, 4096] {
            let mut a = vec![0; size];
            let mut b = vec![0; size];
            let mut full = vec![0; 4096];
            keyed_value(42, 10, &mut a);
            keyed_value(42, 10, &mut b);
            keyed_value(42, 10, &mut full);
            assert_eq!(a, b);
            assert_eq!(a, full[..size]);
            keyed_value(42, 11, &mut b);
            if size >= 8 {
                assert_ne!(a, b);
            }
        }
    }
}
