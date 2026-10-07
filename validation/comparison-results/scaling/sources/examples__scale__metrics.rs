// 32 subdivisions per power of two. Upper-bound estimates have <3.125%
// relative bucket error above 32ns. No retained per-operation samples.
const BINS: usize = 64 * 32;
#[derive(Clone, Debug)]
pub struct Histogram {
    bins: Vec<u64>,
    pub count: u64,
    pub max: u64,
}
impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: vec![0; BINS],
            count: 0,
            max: 0,
        }
    }
}
impl Histogram {
    pub fn record(&mut self, ns: u64) {
        let ns = ns.max(1);
        let exponent = 63 - ns.leading_zeros() as usize;
        let base = 1u64 << exponent;
        let fraction = (((ns - base) as u128 * 32) / base as u128) as usize;
        self.bins[exponent * 32 + fraction] += 1;
        self.count += 1;
        self.max = self.max.max(ns);
    }
    pub fn merge(&mut self, other: &Self) {
        for (a, b) in self.bins.iter_mut().zip(&other.bins) {
            *a += b;
        }
        self.count += other.count;
        self.max = self.max.max(other.max);
    }
    pub fn percentile(&self, numerator: u64, denominator: u64) -> Option<u64> {
        if self.count == 0 || (numerator == 999 && self.count < 10_000) {
            return None;
        }
        let rank = (self.count as u128 * numerator as u128).div_ceil(denominator as u128);
        let mut n = 0u128;
        for (i, b) in self.bins.iter().enumerate() {
            n += *b as u128;
            if n >= rank {
                let base = 1u128 << (i / 32);
                return Some(
                    (base + (base * (i % 32 + 1) as u128).div_ceil(32)).min(self.max as u128)
                        as u64,
                );
            }
        }
        Some(self.max)
    }
    pub fn fields(&self) -> String {
        [
            self.percentile(50, 100),
            self.percentile(95, 100),
            self.percentile(99, 100),
            self.percentile(999, 1000),
        ]
        .map(|n| n.map(|n| n.to_string()).unwrap_or_default())
        .join(",")
    }
}
#[derive(Default, Clone, Debug)]
pub struct Metrics {
    pub reads: Histogram,
    pub writes: Histogram,
    pub misses: u64,
    pub app_bytes: u64,
    pub syncs: u64,
}
impl Metrics {
    pub fn merge(&mut self, other: &Self) {
        self.reads.merge(&other.reads);
        self.writes.merge(&other.writes);
        self.misses += other.misses;
        self.app_bytes += other.app_bytes;
        self.syncs += other.syncs;
    }
    pub fn all(&self) -> Histogram {
        let mut h = self.reads.clone();
        h.merge(&self.writes);
        h
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_merge() {
        let mut h = Histogram::default();
        for i in 1..=100_000 {
            h.record(i);
        }
        for (p, d, exact) in [
            (50, 100, 50_000),
            (95, 100, 95_000),
            (99, 100, 99_000),
            (999, 1000, 99_900),
        ] {
            let q = h.percentile(p, d).unwrap();
            assert!(q >= exact && q <= exact + exact / 32 + 1);
        }
        let mut merged = Histogram::default();
        merged.merge(&h);
        assert_eq!(merged.fields(), h.fields());
        assert_eq!(Histogram::default().percentile(50, 100), None);
    }
}
