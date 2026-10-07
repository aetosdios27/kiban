use crate::config::Config;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct Settings {
    pub engine: String,
    pub profile: String,
    pub drain_seconds: f64,
    pub verify: bool,
    pub rocks_statistics: bool,
    pub workload: Config,
}
impl Settings {
    pub fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut engine = None;
        let mut profile = "matched".to_string();
        let mut drain_seconds: f64 = 5.0;
        let mut verify = true;
        let mut rocks_statistics = false;
        let mut rest = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for pair in args.chunks(2) {
            if pair.len() != 2 || !seen.insert(pair[0].clone()) {
                return Err("each option takes one value; duplicates are forbidden".into());
            }
            let value = &pair[1];
            match pair[0].as_str() {
                "--engine" => engine = Some(value.clone()),
                "--profile" => profile = value.clone(),
                "--drain-seconds" => {
                    drain_seconds = value.parse().map_err(|_| "invalid drain duration")?
                }
                "--verify" => verify = value.parse().map_err(|_| "verify requires true/false")?,
                "--rocks-statistics" => {
                    rocks_statistics = value
                        .parse()
                        .map_err(|_| "statistics requires true/false")?
                }
                _ => rest.extend_from_slice(pair),
            }
        }
        let engine = engine.ok_or("--engine is required")?;
        if !["kiban", "rocksdb"].contains(&engine.as_str())
            || !["matched", "defaults"].contains(&profile.as_str())
        {
            return Err("engine must be kiban/rocksdb; profile must be matched/defaults".into());
        }
        if profile == "defaults"
            && rest.chunks(2).any(|p| {
                [
                    "--write-buffer",
                    "--block-cache",
                    "--target-file-size",
                    "--base-level-bytes",
                    "--level-multiplier",
                    "--l0-trigger",
                    "--l0-stall",
                    "--max-open-files",
                ]
                .contains(&p[0].as_str())
            })
        {
            return Err("engine option overrides require --profile matched".into());
        }
        if !drain_seconds.is_finite() || !(0.0..=3600.0).contains(&drain_seconds) {
            return Err("drain duration must be in 0..3600 seconds".into());
        }
        let workload = Config::from_args(rest)?;
        if workload.value_pattern != "constant" || workload.preload_mode != "write" {
            return Err("comparison adapter supports only the historical constant value pattern".into());
        }
        if workload.reuse {
            return Err("comparison runs require fresh DBs; preload/cache modes recreate the same logical dataset".into());
        }
        Ok(Self {
            engine,
            profile,
            drain_seconds,
            verify,
            rocks_statistics,
            workload,
        })
    }
    pub fn json(&self) -> Value {
        let c = &self.workload;
        json!({"engine":self.engine,"profile":self.profile,"verify":self.verify,"drain_seconds":self.drain_seconds,"rocks_statistics":self.rocks_statistics,
            "workload":c.workload,"keys":c.keys,"working_set":c.working_set,"value_size":c.value_size,"operations":c.operations,"read_percent":c.read_percent,
            "threads":c.threads,"seed":c.seed,"duration_s":c.duration,"sample_ms":c.sample_ms,"sync_every_per_thread":c.sync_every,"preload_keys":c.preload_keys,
            "cache":c.cache,"database":c.db,"output":c.output,"matched_knobs":{"write_buffer_bytes":c.options.write_buffer_bytes,"block_cache_bytes":c.options.block_cache_bytes,
            "target_file_size":c.options.target_file_size,"base_level_bytes":c.options.base_level_bytes,"level_multiplier":c.options.level_multiplier,"l0_trigger":c.options.l0_compaction_trigger,
            "l0_stop":c.options.l0_write_stall_trigger,"max_open_files":c.options.max_open_table_files},
            "kiban_source":"76d9d857c61bed3127fa3f7a2136701cd9b49380","rocksdb":"11.8.1","rust_binding":"0.25.0",
            "durability":if c.sync_every == 0 { "buffered; final durable sync outside foreground window" } else if c.sync_every == 1 { "durable acknowledgement per write" } else { "per-thread periodic durable sync; intervening puts are not durable acknowledgements" },
            "instrumentation":"common per-op wall timing, bounded thread-local histograms, per-second engine/OS sampling; Kiban native counters always enabled; Rocks statistics opt-in",
            "memory_budget_note":"matched named knobs and two memtables; not a hard/equal total RSS limit; metadata/background behavior differ",
            "compression":if self.profile == "matched" { "none in both engines" } else { "Kiban none; RocksDB native Snappy" }})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn rejects_ignored_overrides_and_ambiguous_runs() {
        for a in [
            vec![
                "--engine",
                "kiban",
                "--profile",
                "defaults",
                "--write-buffer",
                "1024",
            ],
            vec!["--engine", "rocksdb", "--reuse", "true"],
            vec!["--engine", "kiban", "--engine", "rocksdb"],
            vec!["--engine", "rocksdb", "--drain-seconds", "NaN"],
        ] {
            assert!(Settings::parse(args(&a)).is_err());
        }
    }
    #[test]
    fn adapters_receive_same_logical_trace() {
        let a = Settings::parse(args(&[
            "--engine",
            "kiban",
            "--workload",
            "burst-recovery",
            "--threads",
            "4",
        ]))
        .unwrap();
        let b = Settings::parse(args(&[
            "--engine",
            "rocksdb",
            "--workload",
            "burst-recovery",
            "--threads",
            "4",
        ]))
        .unwrap();
        for (index, (sa, sb)) in crate::workload::stages(&a.workload)
            .iter()
            .zip(crate::workload::stages(&b.workload))
            .enumerate()
        {
            for id in 0..10000 {
                assert_eq!(
                    crate::workload::operation(&a.workload, sa, index as u64, id),
                    crate::workload::operation(&b.workload, &sb, index as u64, id)
                );
            }
        }
    }
}
