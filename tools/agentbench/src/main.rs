mod backend;
#[path = "../../../examples/scale/metrics.rs"]
#[allow(dead_code)]
mod metrics;
mod model;
#[path = "../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;
mod runner;

use backend::{Embedded, Result};
use serde::Deserialize;
use std::{io::Write, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    backend: String,
    db: PathBuf,
    endpoint: String,
    n: u64,
    operations: u64,
    k: usize,
    seed: u64,
    output: PathBuf,
    verify_only: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("INVALID: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: kiban-agentbench CONFIG.json")?;
    let c: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    if c.n < model::AGENTS || c.n > 100_000_000 || c.operations == 0 || c.k == 0 || c.k > 1000 {
        return Err("require 64 <= n <= 100M, operations > 0, 1 <= k <= 1000".into());
    }
    let mut output = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&c.output)?,
    );
    let _ = &c.endpoint;
    if c.backend != "kiban" {
        return Err("backend not implemented".into());
    }
    let mut backend = Embedded::open(&c.db)?;
    if c.verify_only {
        runner::verify_persisted(&mut backend, &c, &mut output)?;
    } else {
        runner::suite(&mut backend, &c, &mut output)?;
    }
    output.flush()?;
    Ok(())
}
