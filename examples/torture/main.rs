mod config;
mod model;
mod probes;
mod process;
mod simulated;
#[path = "../support/random.rs"]
#[allow(dead_code)]
mod workload;
mod telemetry {
    use std::{
        fs::{File, OpenOptions},
        io,
        path::Path,
    };
    pub fn create(p: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(p)
    }
    pub fn csv(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
}
use std::io::{self, Write};
fn run(c: config::Config) -> io::Result<()> {
    if let Ok(cycle) = std::env::var("KIBAN_TORTURE_CHILD") {
        return process::child(&c, cycle.parse().map_err(io::Error::other)?);
    }
    std::fs::create_dir_all(&c.db)?;
    if std::fs::read_dir(&c.db)?.next().is_some() {
        return Err(io::Error::other(
            "torture requires a fresh empty database directory",
        ));
    }
    std::fs::create_dir_all(&c.output)?;
    let db = std::fs::canonicalize(&c.db)?;
    let out = std::fs::canonicalize(&c.output)?;
    if db.starts_with(&out) || out.starts_with(&db) {
        return Err(io::Error::other(
            "database/output directories must be separate",
        ));
    }
    let mut config = telemetry::create(&c.output.join("config.csv"))?;
    writeln!(
        config,
        "parameter,value\nconfig,{}\nengine_options,{}",
        telemetry::csv(&format!("{c:?}")),
        telemetry::csv(&format!("{:?}", model::options()))
    )?;
    let mut status = telemetry::create(&c.output.join("status.csv"))?;
    writeln!(
        status,
        "status,detail\nstarted,incomplete until complete record"
    )?;
    let result = match c.mode.as_str() {
        "simulated" => simulated::run(&c),
        "process" => process::run(&c),
        _ => probes::run(&c),
    };
    match &result {
        Ok(()) => writeln!(status, "complete,success")?,
        Err(e) => writeln!(status, "failed,{}", telemetry::csv(&e.to_string()))?,
    }
    result
}
fn main() {
    if std::env::args().any(|a| a == "--help") {
        println!("{}", config::HELP);
        return;
    }
    if let Err(e) = config::Config::parse()
        .map_err(io::Error::other)
        .and_then(run)
    {
        eprintln!("torture FAILED: {e}");
        std::process::exit(1);
    }
}
