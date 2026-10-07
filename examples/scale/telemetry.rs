use super::{config::Config, metrics::Metrics};
use kiban::db::{KibanStats, SharedKiban};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

pub fn csv(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
pub fn disk_bytes(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for e in std::fs::read_dir(dir)? {
        match e?.metadata() {
            Ok(m) if m.is_file() => total += m.len(),
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}
/// File-allocated sectors, distinct from logical lengths on compressed filesystems.
pub fn allocated_bytes(dir: &Path) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let mut total = 0;
    for e in std::fs::read_dir(dir)? {
        match e?.metadata() {
            Ok(m) if m.is_file() => total += m.blocks() * 512,
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}
pub fn proc_number(path: &str, field: &str, multiplier: u64) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix(field)?;
            Some(rest.split_whitespace().next()?.parse::<u64>().ok()? * multiplier)
        })
}
fn optional(v: Option<u64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}
pub fn debt(stats: &KibanStats, c: &Config) -> (u64, f64, usize) {
    let mut bytes = 0u64;
    let mut ratio = 0f64;
    let mut levels = 0;
    for l in &stats.levels {
        if l.level == 0 {
            continue;
        }
        let budget = c
            .options
            .base_level_bytes
            .saturating_mul(c.options.level_multiplier.saturating_pow(l.level - 1))
            .max(1);
        bytes = bytes.saturating_add(l.bytes.saturating_sub(budget));
        ratio = ratio.max(l.bytes as f64 / budget as f64);
        levels += usize::from(l.bytes > budget);
    }
    (bytes, ratio, levels)
}
pub struct Reporter {
    samples: File,
    levels: File,
    decisions: File,
    summaries: File,
    #[cfg(feature = "diagnostics")]
    acquisitions: File,
    pub start: Instant,
    last: Instant,
    last_sequence: u64,
    pub baseline: KibanStats,
    pub io_read_baseline: Option<u64>,
    pub io_write_baseline: Option<u64>,
}
impl Reporter {
    pub fn new(c: &Config, db: &SharedKiban) -> io::Result<Self> {
        let mut samples = create(&c.output.join("samples.csv"))?;
        writeln!(
            samples,
            "elapsed_s,stage,interval_s,reads,writes,read_ops_s,write_ops_s,all_p50_ns,all_p95_ns,all_p99_ns,all_p999_ns,read_p50_ns,read_p95_ns,read_p99_ns,read_p999_ns,write_p50_ns,write_p95_ns,write_p99_ns,write_p999_ns,read_misses,app_bytes_total,sst_bytes_written,sst_write_amp,db_bytes,l0_files,debt_bytes,max_debt_ratio,overbudget_levels,pending_flush,memtable_bytes,immutable_bytes,compactions,compaction_input_bytes,compaction_output_bytes,compaction_elapsed_ns,flushes,flush_elapsed_ns,stalls,stall_ns,waiting_writers,maintenance_running,rss_bytes,fds,process_read_bytes,process_write_bytes,cache_hits,cache_misses,file_waits,governor_mode,governor_pressure,pressure_l0,pressure_debt,pressure_read,mode_switches,avg_batch_size,trivial_moves,obsolete_files,immutable_stalls,l0_stalls,both_stalls,immutable_wait_ns,l0_wait_ns,both_wait_ns,flush_queue_ns,flush_queue_max_ns,flush_build_ns,flush_commit_wait_ns,flush_commit_ns,compaction_build_ns,compaction_commit_wait_ns,compaction_commit_ns,gets_total,memtable_gets,immutable_gets,l0_probes,leveled_probes,not_found_gets,sample_cost_ns"
        )?;
        let mut levels = create(&c.output.join("levels.csv"))?;
        writeln!(levels, "elapsed_s,stage,level,tables,bytes")?;
        let mut decisions = create(&c.output.join("decisions.csv"))?;
        writeln!(
            decisions,
            "observed_elapsed_s,stage,sequence,gap_before,pressure_l0,pressure_debt,pressure_read,dominant,tier,chosen,candidates,predicted_rewrite_bytes,l0_relief,debt_relief,read_relief"
        )?;
        let mut summaries = create(&c.output.join("summary.csv"))?;
        writeln!(
            summaries,
            "stage,threads,elapsed_s,operations,reads,writes,total_ops_s,read_ops_s,write_ops_s,all_p50_ns,all_p95_ns,all_p99_ns,all_p999_ns,read_p50_ns,read_p95_ns,read_p99_ns,read_p999_ns,write_p50_ns,write_p95_ns,write_p99_ns,write_p999_ns,read_misses,app_bytes,sync_calls,all_max_ns,read_max_ns,write_max_ns"
        )?;
        let now = Instant::now();
        #[cfg(feature = "diagnostics")]
        let acquisitions = {
            let mut f = create(&c.output.join("acquisitions.csv"))?;
            writeln!(f, "elapsed_s,stage,phase,samples,elapsed_ns,maximum_ns")?;
            f
        };
        Ok(Self {
            samples,
            levels,
            decisions,
            summaries,
            #[cfg(feature = "diagnostics")]
            acquisitions,
            start: now,
            last: now,
            last_sequence: 0,
            baseline: db.stats().map_err(io::Error::other)?,
            io_read_baseline: proc_number("/proc/self/io", "read_bytes:", 1),
            io_write_baseline: proc_number("/proc/self/io", "write_bytes:", 1),
        })
    }
    pub fn sample(
        &mut self,
        c: &Config,
        db: &SharedKiban,
        stage: &str,
        interval: &Metrics,
        total: &Metrics,
    ) -> io::Result<()> {
        let cost = Instant::now();
        let at = self.start.elapsed().as_secs_f64();
        let dt = self.last.elapsed().as_secs_f64().max(1e-9);
        self.last = Instant::now();
        let s = db.stats().map_err(io::Error::other)?;
        #[cfg(feature = "diagnostics")]
        for d in kiban::diagnostics::stats() {
            writeln!(
                self.acquisitions,
                "{at},{stage},{},{},{},{}",
                d.phase, d.samples, d.elapsed_ns, d.maximum_ns
            )?;
        }
        if let Some(e) = db.maintenance_error() {
            return Err(io::Error::other(e));
        }
        let m = s.maintenance;
        let b = self.baseline.maintenance;
        let sst = m.flush_output_bytes + m.compaction_output_bytes
            - b.flush_output_bytes
            - b.compaction_output_bytes;
        let wa = if total.app_bytes == 0 {
            String::new()
        } else {
            (sst as f64 / total.app_bytes as f64).to_string()
        };
        let (debt, ratio, over) = debt(&s, c);
        let g = s.governor;
        let fields = vec![
            at.to_string(),
            csv(stage),
            dt.to_string(),
            interval.reads.count.to_string(),
            interval.writes.count.to_string(),
            (interval.reads.count as f64 / dt).to_string(),
            (interval.writes.count as f64 / dt).to_string(),
            interval.all().fields(),
            interval.reads.fields(),
            interval.writes.fields(),
            interval.misses.to_string(),
            total.app_bytes.to_string(),
            sst.to_string(),
            wa,
            disk_bytes(&c.db)?.to_string(),
            s.levels
                .iter()
                .find(|l| l.level == 0)
                .map_or(0, |l| l.tables)
                .to_string(),
            debt.to_string(),
            ratio.to_string(),
            over.to_string(),
            s.immutable_present.to_string(),
            s.memtable_logical_bytes.to_string(),
            s.immutable_logical_bytes.to_string(),
            (m.compactions_completed - b.compactions_completed).to_string(),
            (m.compaction_input_bytes - b.compaction_input_bytes).to_string(),
            (m.compaction_output_bytes - b.compaction_output_bytes).to_string(),
            (m.compaction_elapsed_ns - b.compaction_elapsed_ns).to_string(),
            (m.flushes_completed - b.flushes_completed).to_string(),
            (m.flush_elapsed_ns - b.flush_elapsed_ns).to_string(),
            (m.write_stalls - b.write_stalls).to_string(),
            (m.write_stall_ns - b.write_stall_ns).to_string(),
            m.waiting_writers.to_string(),
            m.compaction_running.to_string(),
            optional(proc_number("/proc/self/status", "VmRSS:", 1024)),
            optional(
                std::fs::read_dir("/proc/self/fd")
                    .ok()
                    .map(|it| it.count() as u64),
            ),
            optional(
                proc_number("/proc/self/io", "read_bytes:", 1)
                    .zip(self.io_read_baseline)
                    .map(|(a, b)| a.saturating_sub(b)),
            ),
            optional(
                proc_number("/proc/self/io", "write_bytes:", 1)
                    .zip(self.io_write_baseline)
                    .map(|(a, b)| a.saturating_sub(b)),
            ),
            (s.block_cache.hits - self.baseline.block_cache.hits).to_string(),
            (s.block_cache.misses - self.baseline.block_cache.misses).to_string(),
            (s.table_files.waits - self.baseline.table_files.waits).to_string(),
            format!("{:?}", g.mode),
            g.pressure.to_string(),
            g.pressure_vector.l0.to_string(),
            g.pressure_vector.deep_debt.to_string(),
            g.pressure_vector.read.to_string(),
            g.mode_switches.to_string(),
            g.avg_batch_size.to_string(),
            g.trivial_moves_performed.to_string(),
            s.obsolete_files_pending.to_string(),
            (m.timings.immutable_stalls - b.timings.immutable_stalls).to_string(),
            (m.timings.l0_stalls - b.timings.l0_stalls).to_string(),
            (m.timings.both_stalls - b.timings.both_stalls).to_string(),
            (m.timings.immutable_wait_ns - b.timings.immutable_wait_ns).to_string(),
            (m.timings.l0_wait_ns - b.timings.l0_wait_ns).to_string(),
            (m.timings.both_wait_ns - b.timings.both_wait_ns).to_string(),
            (m.timings.flush_queue_ns - b.timings.flush_queue_ns).to_string(),
            m.timings.flush_queue_max_ns.to_string(),
            (m.timings.flush_build_ns - b.timings.flush_build_ns).to_string(),
            (m.timings.flush_commit_wait_ns - b.timings.flush_commit_wait_ns).to_string(),
            (m.timings.flush_commit_ns - b.timings.flush_commit_ns).to_string(),
            (m.timings.compaction_build_ns - b.timings.compaction_build_ns).to_string(),
            (m.timings.compaction_commit_wait_ns - b.timings.compaction_commit_wait_ns).to_string(),
            (m.timings.compaction_commit_ns - b.timings.compaction_commit_ns).to_string(),
            (s.read_amp.gets_total - self.baseline.read_amp.gets_total).to_string(),
            (s.read_amp.resolved_in_memtable - self.baseline.read_amp.resolved_in_memtable)
                .to_string(),
            (s.read_amp.resolved_in_immutable - self.baseline.read_amp.resolved_in_immutable)
                .to_string(),
            (s.read_amp.l0_tables_probed - self.baseline.read_amp.l0_tables_probed).to_string(),
            (s.read_amp.leveled_tables_probed - self.baseline.read_amp.leveled_tables_probed)
                .to_string(),
            (s.read_amp.resolved_not_found - self.baseline.read_amp.resolved_not_found).to_string(),
        ];
        for l in &s.levels {
            writeln!(
                self.levels,
                "{at},{},{},{},{}",
                csv(stage),
                l.level,
                l.tables,
                l.bytes
            )?;
        }
        for d in db.governor_trace().map_err(io::Error::other)? {
            if d.sequence <= self.last_sequence {
                continue;
            }
            let gap = d.sequence - self.last_sequence - 1;
            writeln!(
                self.decisions,
                "{at},{},{},{},{},{},{},{:?},{:?},{},{},{},{},{},{}",
                csv(stage),
                d.sequence,
                gap,
                d.pressure.l0,
                d.pressure.deep_debt,
                d.pressure.read,
                d.dominant,
                d.tier,
                csv(&format!("{:?}", d.chosen)),
                d.candidates_considered,
                d.predicted_rewrite_bytes,
                d.l0_relief,
                d.debt_relief,
                d.read_relief
            )?;
            self.last_sequence = d.sequence;
        }
        writeln!(
            self.samples,
            "{},{}",
            fields.join(","),
            cost.elapsed().as_nanos()
        )?;
        Ok(())
    }
    pub fn summary(
        &mut self,
        stage: &str,
        threads: usize,
        elapsed: f64,
        m: &Metrics,
    ) -> io::Result<()> {
        let ops = m.reads.count + m.writes.count;
        writeln!(
            self.summaries,
            "{},{threads},{elapsed},{ops},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv(stage),
            m.reads.count,
            m.writes.count,
            ops as f64 / elapsed,
            m.reads.count as f64 / elapsed,
            m.writes.count as f64 / elapsed,
            m.all().fields(),
            m.reads.fields(),
            m.writes.fields(),
            m.misses,
            m.app_bytes,
            m.syncs,
            m.all().max,
            optional((m.reads.count > 0).then_some(m.reads.max)),
            optional((m.writes.count > 0).then_some(m.writes.max))
        )?;
        eprintln!(
            "{stage}: {ops} operations in {elapsed:.3}s ({:.0} ops/s), read/write p99={} / {} ns",
            ops as f64 / elapsed,
            optional(m.reads.percentile(99, 100)),
            optional(m.writes.percentile(99, 100))
        );
        Ok(())
    }
}
pub fn write_config(c: &Config) -> io::Result<()> {
    let mut f = create(&c.output.join("config.csv"))?;
    writeln!(f, "parameter,value")?;
    for (k, v) in [
        ("schema_version", "1".into()),
        ("workload", c.workload.clone()),
        ("keys", c.keys.to_string()),
        ("working_set", c.working_set.to_string()),
        ("key_bytes", "8".into()),
        ("value_size", c.value_size.to_string()),
        ("value_pattern", c.value_pattern.clone()),
        ("operations", c.operations.to_string()),
        ("read_percent", c.read_percent.to_string()),
        ("threads", c.threads.to_string()),
        ("seed", c.seed.to_string()),
        ("duration", c.duration.to_string()),
        ("sample_ms", c.sample_ms.to_string()),
        ("sync_every_per_thread", c.sync_every.to_string()),
        ("preload_keys", c.preload_keys.to_string()),
        ("preload_mode", c.preload_mode.clone()),
        ("cache", c.cache.clone()),
        ("reuse", c.reuse.to_string()),
        ("db", c.db.display().to_string()),
        ("output", c.output.display().to_string()),
        ("engine_options", format!("{:?}", c.options)),
        (
            "estimated_working_set_payload_bytes",
            ((c.value_size as u128 + 8) * c.working_set as u128).to_string(),
        ),
        (
            "available_parallelism",
            std::thread::available_parallelism()
                .map(|n| n.to_string())
                .unwrap_or_default(),
        ),
        (
            "mem_total_bytes",
            optional(proc_number("/proc/meminfo", "MemTotal:", 1024)),
        ),
        (
            "mem_available_bytes",
            optional(proc_number("/proc/meminfo", "MemAvailable:", 1024)),
        ),
        (
            "filesystem",
            command(
                "findmnt",
                &[
                    "-T",
                    &c.db.display().to_string(),
                    "-o",
                    "TARGET,SOURCE,FSTYPE,OPTIONS",
                ],
            ),
        ),
        ("git_revision", command("git", &["rev-parse", "HEAD"])),
        ("git_status", command("git", &["status", "--porcelain"])),
        ("rustc", command("rustc", &["--version"])),
        ("uname", command("uname", &["-a"])),
        (
            "cgroup",
            std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default(),
        ),
        (
            "cpuinfo",
            std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default(),
        ),
        (
            "started_unix_s",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_string(),
        ),
        ("debug_assertions", cfg!(debug_assertions).to_string()),
    ] {
        writeln!(f, "{k},{}", csv(&v))?;
    }
    Ok(())
}
fn command(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().into())
        .unwrap_or_default()
}
