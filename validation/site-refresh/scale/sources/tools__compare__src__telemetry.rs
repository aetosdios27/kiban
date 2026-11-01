use crate::{
    backend::Engine,
    metrics::{Histogram, Metrics},
    settings::Settings,
};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    time::Instant,
};

pub fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
pub fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    writeln!(create(path)?, "{}", serde_json::to_string_pretty(value)?)
}
pub fn append(file: &mut File, value: &Value) -> io::Result<()> {
    writeln!(file, "{value}")?;
    file.flush()
}
pub fn disk_bytes(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        match entry.metadata() {
            Ok(m) if m.is_file() => total += m.len(),
            Ok(m) if m.is_dir() => total += disk_bytes(&entry.path())?,
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (), // concurrent obsolete-file retirement
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
            line.strip_prefix(field)?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()?
                .checked_mul(multiplier)
        })
}
pub fn resources() -> Value {
    json!({"rss_bytes":proc_number("/proc/self/status","VmRSS:",1024),"fds":std::fs::read_dir("/proc/self/fd").ok().map(|d|d.count()),
        "process_write_bytes":proc_number("/proc/self/io","write_bytes:",1),"process_read_bytes":proc_number("/proc/self/io","read_bytes:",1),
        "host_mem_available_bytes":proc_number("/proc/meminfo","MemAvailable:",1024),"host_swap_free_bytes":proc_number("/proc/meminfo","SwapFree:",1024)})
}
fn histogram(h: &Histogram) -> Value {
    json!({"samples":h.count,"p50_ns":h.percentile(50,100),"p95_ns":h.percentile(95,100),"p99_ns":h.percentile(99,100),"p999_ns":h.percentile(999,1000),"max_ns":h.max})
}
pub fn metrics(m: &Metrics, seconds: f64) -> Value {
    json!({"operations":m.reads.count+m.writes.count,"reads":m.reads.count,"writes":m.writes.count,"misses":m.misses,"app_bytes":m.app_bytes,"sync_calls":m.syncs,
        "total_ops_s":(m.reads.count+m.writes.count) as f64 /seconds.max(1e-9),"read_ops_s":m.reads.count as f64/seconds.max(1e-9),"write_ops_s":m.writes.count as f64/seconds.max(1e-9),
        "read_latency":histogram(&m.reads),"write_latency":histogram(&m.writes),"all_latency":histogram(&m.all())})
}
pub struct Reporter {
    samples: File,
    pub start: Instant,
    pub baseline: Value,
    pub peak_rss: u64,
    pub peak_fds: u64,
    pub max_l0: u64,
}
impl Reporter {
    pub fn new(s: &Settings, db: &Engine) -> io::Result<Self> {
        let baseline = json!({"engine":db.stats()?,"resources":resources(),"db_bytes":disk_bytes(&s.workload.db)?});
        Ok(Self {
            samples: create(&s.workload.output.join("samples.jsonl"))?,
            start: Instant::now(),
            baseline,
            peak_rss: 0,
            peak_fds: 0,
            max_l0: 0,
        })
    }
    pub fn sample(
        &mut self,
        s: &Settings,
        db: &Engine,
        stage: &str,
        interval: &Metrics,
        seconds: f64,
    ) -> io::Result<Value> {
        let cost = Instant::now();
        let engine = db.stats()?;
        let r = resources();
        if engine["background_errors"].as_u64().is_some_and(|n| n > 0) {
            return Err(io::Error::other("RocksDB background error"));
        }
        self.peak_rss = self.peak_rss.max(r["rss_bytes"].as_u64().unwrap_or(0));
        self.peak_fds = self.peak_fds.max(r["fds"].as_u64().unwrap_or(0));
        if let Some(l) = engine["levels"]
            .as_array()
            .and_then(|ls| ls.iter().find(|l| l["level"] == 0))
        {
            self.max_l0 = self.max_l0.max(l["files"].as_u64().unwrap_or(0));
        }
        let mut sample = json!({"elapsed_s":self.start.elapsed().as_secs_f64(),"stage":stage,"interval_s":seconds,"interval":metrics(interval,seconds),"engine":engine,"resources":r,"db_bytes":disk_bytes(&s.workload.db)?});
        sample["sample_cost_ns"] = json!(cost.elapsed().as_nanos() as u64);
        append(&mut self.samples, &sample)?;
        Ok(sample)
    }
    pub fn amplification(&self, last: &Value, app_bytes: u64) -> Value {
        let delta = |a: &Value, b: &Value| {
            a.as_u64()
                .zip(b.as_u64())
                .and_then(|(a, b)| a.checked_sub(b))
        };
        let sst = delta(
            &last["engine"]["sst_bytes_written"],
            &self.baseline["engine"]["sst_bytes_written"],
        );
        let io = delta(
            &last["resources"]["process_write_bytes"],
            &self.baseline["resources"]["process_write_bytes"],
        );
        json!({"sst_bytes_written":sst,"sst_write_amp":sst.filter(|_|app_bytes>0).map(|n|n as f64/app_bytes as f64),"process_write_bytes":io,
            "process_write_amp":io.filter(|_|app_bytes>0).map(|n|n as f64/app_bytes as f64),"app_bytes":app_bytes,
            "definitions":"SST bytes exclude WAL/MANIFEST; process write_bytes is Linux task I/O accounting, not device bytes; null means unobserved, not zero."})
    }
}
