use kiban::db::KibanOptions;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub workload: String,
    pub keys: u64,
    pub working_set: u64,
    pub value_size: usize,
    pub operations: u64,
    pub read_percent: u64,
    pub threads: usize,
    pub seed: u64,
    pub duration: f64,
    pub sample_ms: u64,
    pub sync_every: u64,
    pub preload_keys: u64,
    pub cache: String,
    pub reuse: bool,
    pub db: PathBuf,
    pub output: PathBuf,
    pub options: KibanOptions,
}

impl Config {
    pub fn parse() -> Result<Self, String> {
        Self::from_args(std::env::args().skip(1).collect())
    }
    pub fn from_args(args: Vec<String>) -> Result<Self, String> {
        let mut c = Self {
            workload: "random-write".into(),
            keys: 1_000_000,
            working_set: 0,
            value_size: 128,
            operations: 1_000_000,
            read_percent: 50,
            threads: 1,
            seed: 42,
            duration: 0.0,
            sample_ms: 1000,
            sync_every: 0,
            preload_keys: 0,
            cache: "as-is".into(),
            reuse: false,
            db: "scale-db".into(),
            output: "scale-results".into(),
            options: KibanOptions::default(),
        };
        let mut seen = std::collections::HashSet::new();
        for pair in args.chunks(2) {
            if pair.len() != 2 {
                return Err("every option requires a value; see --help".into());
            }
            let k = pair[0].as_str();
            let v = &pair[1];
            if !seen.insert(k) {
                return Err(format!("duplicate option {k}"));
            }
            let n = || {
                v.parse::<u64>()
                    .map_err(|_| format!("invalid integer for {k}"))
            };
            let size = || usize::try_from(n()?).map_err(|_| format!("{k} exceeds usize"));
            match k {
                "--workload" => c.workload = v.clone(),
                "--keys" => c.keys = n()?,
                "--working-set" => c.working_set = n()?,
                "--value-size" => c.value_size = size()?,
                "--operations" => c.operations = n()?,
                "--read-percent" => c.read_percent = n()?,
                "--threads" => c.threads = size()?,
                "--seed" => c.seed = n()?,
                "--duration" => c.duration = v.parse().map_err(|_| "invalid duration")?,
                "--sample-ms" => c.sample_ms = n()?,
                "--sync-every" => c.sync_every = n()?,
                "--preload-keys" => c.preload_keys = n()?,
                "--cache" => c.cache = v.clone(),
                "--reuse" => c.reuse = v.parse().map_err(|_| "--reuse requires true or false")?,
                "--db" => c.db = v.into(),
                "--output" => c.output = v.into(),
                "--write-buffer" => c.options.write_buffer_bytes = size()?,
                "--block-cache" => c.options.block_cache_bytes = size()?,
                "--target-file-size" => c.options.target_file_size = n()?,
                "--base-level-bytes" => c.options.base_level_bytes = n()?,
                "--level-multiplier" => c.options.level_multiplier = n()?,
                "--l0-trigger" => c.options.l0_compaction_trigger = size()?,
                "--l0-stall" => c.options.l0_write_stall_trigger = size()?,
                "--max-open-files" => c.options.max_open_table_files = size()?,
                _ => return Err(format!("unknown option {k}")),
            }
        }
        if c.working_set == 0 {
            c.working_set = c.keys;
        }
        if ![
            "sequential-write",
            "random-write",
            "random-read",
            "mixed",
            "update-heavy",
            "burst-read",
            "sustained-write",
            "burst-recovery",
            "concurrency-jump",
        ]
        .contains(&c.workload.as_str())
        {
            return Err("unknown workload".into());
        }
        if c.keys == 0
            || c.working_set > c.keys
            || c.threads == 0
            || c.threads > 4096
            || c.read_percent > 100
            || c.sample_ms == 0
            || c.preload_keys > c.keys
            || !c.duration.is_finite()
            || c.duration < 0.0
            || c.duration > 31_536_000.0
            || (c.operations == 0 && c.duration == 0.0)
            || c.options.write_buffer_bytes == 0
            || c.options.target_file_size == 0
            || c.options.base_level_bytes == 0
            || c.options.level_multiplier < 2
            || c.options.max_open_table_files == 0
            || c.options.l0_compaction_trigger == 0
            || c.options.l0_write_stall_trigger <= c.options.l0_compaction_trigger
            || c.value_size > (1 << 29)
        {
            return Err("invalid range or engine options; see --help".into());
        }
        if !["as-is", "warm", "reduced"].contains(&c.cache.as_str()) {
            return Err("invalid cache mode".into());
        }
        Ok(c)
    }
}

pub const HELP: &str = "Kiban scale validation (release build only for measurements)
--workload sequential-write|random-write|random-read|mixed|update-heavy|burst-read|sustained-write|burst-recovery|concurrency-jump
--keys N --working-set N --value-size BYTES --operations N --read-percent 0..100
--threads N --seed N --duration SECONDS --sample-ms N --sync-every N
--preload-keys N --cache as-is|warm|reduced --reuse true|false --db DIR --output DIR
--write-buffer BYTES --block-cache BYTES --target-file-size BYTES --base-level-bytes BYTES
--level-multiplier N --l0-trigger N --l0-stall N --max-open-files N
Every flag takes a value. Operations=0 requires a duration; both limits stop at the first reached.
Sync-every=0: end sync only; 1: each write; N: per-thread group every N writes.
No automatic deletion. Existing database requires explicit reuse. Output must be new.
All resolved parameters and engine options are recorded in config.csv.";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_or_invalid_config() {
        for a in [
            vec!["--threads", "0"],
            vec!["--operations", "0"],
            vec!["--seed", "2", "--seed", "3"],
            vec!["--duration", "NaN"],
            vec!["--typo", "3"],
        ] {
            assert!(Config::from_args(a.into_iter().map(String::from).collect()).is_err());
        }
        assert_eq!(
            Config::from_args(vec!["--operations".into(), "1000000000".into()])
                .unwrap()
                .operations,
            1_000_000_000
        );
    }
}
