mod backend;
#[path = "../../../examples/scale/metrics.rs"]
#[allow(dead_code)]
mod metrics;
mod model;
#[path = "../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;
mod runner;

use backend::{Embedded, Helix, Result};
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
    if c.backend == "helix-floor" {
        runner::floor(&Helix::new(&c.endpoint)?, &c, &mut output)?;
    } else {
        let mut backend: Box<dyn backend::Backend> = match c.backend.as_str() {
            "kiban" => Box::new(Embedded::open(&c.db)?),
            "helix" => Box::new(Helix::new(&c.endpoint)?),
            _ => return Err("unknown backend".into()),
        };
        if c.verify_only {
            runner::verify_persisted(backend.as_mut(), &c, &mut output)?;
        } else {
            runner::suite(backend.as_mut(), &c, &mut output)?;
        }
    }
    output.flush()?;
    Ok(())
}
