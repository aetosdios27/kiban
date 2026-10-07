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
