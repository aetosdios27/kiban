#[path = "../examples/torture/config.rs"]
#[allow(dead_code)]
mod config;
#[path = "../examples/torture/model.rs"]
mod model;
#[path = "../examples/torture/simulated.rs"]
mod simulated;
#[path = "../examples/support/random.rs"]
#[allow(dead_code)]
mod workload;
mod telemetry {
    use std::{
        fs::{File, OpenOptions},
        io,
        path::Path,
    };
    pub fn csv(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    pub fn create(p: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(p)
    }
}
#[test]
fn repeated_wal_power_loss_preserves_durable_state() {
    let root = std::env::temp_dir().join(format!("kiban-repeated-recovery-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let db = root.join("db");
    let output = root.join("results");
    std::fs::create_dir(&db).unwrap();
    std::fs::create_dir(&output).unwrap();
    let c = config::Config {
        mode: "simulated".into(),
        db,
        output: output.clone(),
        seed: 20261006,
        cycles: 40,
        operations: 20,
        keys: 128,
        sync_every: 7,
        fault_percent: 80,
        threads: 1,
        snapshots: false,
    };
    simulated::run(&c).unwrap();
    let csv = std::fs::read_to_string(output.join("cycles.csv")).unwrap();
    assert_eq!(csv.lines().count(), 41);
    assert!(
        csv.lines().skip(1).any(|l| l.contains(",true,")),
        "fault campaign must trigger faults"
    );
    assert!(
        csv.lines().skip(1).any(|l| l.ends_with(",exact")),
        "must exercise exact power-loss verification"
    );
}

#[test]
fn snapshots_survive_repeated_recovery_and_faults() {
    let root =
        std::env::temp_dir().join(format!("kiban-snapshot-detection-{}", std::process::id()));
    let db = root.join("db");
    let output = root.join("results");
    std::fs::create_dir_all(&db).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let c = config::Config {
        mode: "simulated".into(),
        db,
        output,
        seed: 20261006,
        cycles: 20,
        operations: 100,
        keys: 64,
        sync_every: 7,
        fault_percent: 80,
        threads: 1,
        snapshots: true,
    };
    let result = simulated::run(&c);
    std::fs::remove_dir_all(root).unwrap();
    result.expect("every pinned snapshot must retain its captured logical view");
}

#[test]
fn point_reads_match_scans_through_repeated_recovery_and_faults() {
    let root =
        std::env::temp_dir().join(format!("kiban-point-scan-detection-{}", std::process::id()));
    let db = root.join("db");
    let output = root.join("results");
    std::fs::create_dir_all(&db).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let c = config::Config {
        mode: "simulated".into(),
        db,
        output,
        seed: 20261006,
        cycles: 20,
        operations: 100,
        keys: 64,
        sync_every: 7,
        fault_percent: 80,
        threads: 1,
        snapshots: false,
    };
    let result = simulated::run(&c);
    std::fs::remove_dir_all(root).unwrap();
    result.expect("point reads and scans must agree after every recovery");
}
