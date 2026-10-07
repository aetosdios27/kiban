mod backend;
#[path = "../../../examples/scale/cache.rs"]
pub mod cache;
#[path = "../../../examples/scale/config.rs"]
pub mod config;
#[path = "../../../examples/scale/metrics.rs"]
pub mod metrics;
#[path = "../../../examples/support/random.rs"]
pub mod random;
mod runner;
mod settings;
mod telemetry;
#[path = "../../../examples/scale/workload.rs"]
pub mod workload;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!(
            "Kiban/RocksDB comparison\n--engine kiban|rocksdb --profile matched|defaults --drain-seconds N --verify true|false --rocks-statistics true|false\n{}",
            config::HELP
        );
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--crash-probe") {
        let s = settings::Settings::parse(vec![
            "--engine".into(),
            args.get(1).ok_or("probe requires engine")?.clone(),
            "--db".into(),
            args.get(2).ok_or("probe requires DB path")?.clone(),
        ])?;
        let db = backend::Engine::open(&s)?;
        for key in 0u64..64 {
            db.put(&key.to_be_bytes(), b"durable", true)?;
        }
        db.flush()?;
        for key in 32u64..96 {
            db.put(&key.to_be_bytes(), b"new-durable", true)?;
        }
        for key in 0u64..8 {
            db.delete(&key.to_be_bytes())?;
        }
        db.sync()?;
        use std::io::Write;
        println!("READY");
        std::io::stdout().flush()?;
        loop {
            std::thread::park();
        }
    }
    let s = settings::Settings::parse(args)?;
    runner::run(s)?;
    Ok(())
}
