use super::config::Config;

pub use crate::random::mix;
#[derive(Clone, Debug)]
pub struct Stage {
    pub name: &'static str,
    pub reads: u64,
    pub threads: usize,
    pub sequential: bool,
}
pub fn stages(c: &Config) -> Vec<Stage> {
    let s = |name, reads, threads, sequential| Stage {
        name,
        reads,
        threads,
        sequential,
    };
    match c.workload.as_str() {
        "sequential-write" => vec![s("sequential-write", 0, c.threads, true)],
        "random-read" => vec![s("random-read", 100, c.threads, false)],
        "mixed" => vec![s("mixed", c.read_percent, c.threads, false)],
        "update-heavy" => vec![s("update-heavy", c.read_percent, c.threads, false)],
        "burst-read" => vec![
            s("write-pressure", 0, c.threads, false),
            s("read-heavy", 95, c.threads, false),
        ],
        "burst-recovery" => vec![
            s("steady", c.read_percent, c.threads, false),
            s("write-burst", 0, c.threads, false),
            s("read-recovery", 95, c.threads, false),
        ],
        "concurrency-jump" => vec![
            s("low-concurrency", c.read_percent, 1, false),
            s("high-concurrency", c.read_percent, c.threads, false),
        ],
        "sustained-write" => vec![s("sustained-write", 0, c.threads, false)],
        _ => vec![s("random-write", 0, c.threads, false)],
    }
}
pub fn operation(c: &Config, s: &Stage, stage: u64, id: u64) -> (bool, [u8; 8]) {
    let r = mix(c.seed ^ mix(id) ^ mix(stage));
    let key = if s.sequential {
        id % c.keys
    } else {
        mix(r) % c.working_set
    };
    (r % 100 < s.reads, key.to_be_bytes())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_partition_covers_exact_budget() {
        let c = Config::from_args(vec![]).unwrap();
        let s = stages(&c).remove(0);
        let a: Vec<_> = (0..100).map(|i| operation(&c, &s, 0, i)).collect();
        let mut b: Vec<_> = (0..7)
            .flat_map(|t| (t..100).step_by(7).map(|i| (i, operation(&c, &s, 0, i))))
            .collect();
        b.sort_by_key(|(i, _)| *i);
        assert_eq!(a, b.into_iter().map(|(_, op)| op).collect::<Vec<_>>());
    }
}
