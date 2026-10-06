//! Small real-filesystem reproductions for defects first found by the campaign.
//! These validate expected storage semantics and return failure on disagreement.
use super::{config::Config, model, telemetry};
use kiban::db::Kiban;
use std::io::{self, Write};
pub fn run(c: &Config) -> io::Result<()> {
    let mut out = telemetry::create(&c.output.join("probes.csv"))?;
    writeln!(out, "probe,expected,actual,passed")?;
    let mut failures = 0;
    {
        let mut db = Kiban::open_with_options(c.db.join("snapshot"), model::options())
            .map_err(io::Error::other)?;
        db.put(b"key", b"before")?;
        db.sync()?;
        let snap = db.snapshot();
        db.put(b"key", b"after")?;
        db.sync()?;
        db.flush().map_err(io::Error::other)?;
        let got = db.get_at(&snap, b"key").map_err(io::Error::other)?;
        let passed = got.as_deref() == Some(b"before".as_slice());
        writeln!(
            out,
            "snapshot-across-first-flush,before,{},{}",
            telemetry::csv(&format!("{got:?}")),
            passed
        )?;
        failures += usize::from(!passed);
        db.release_snapshot(&snap);
    }
    {
        let dir = c.db.join("versions");
        let mut db = Kiban::open_with_options(&dir, model::options()).map_err(io::Error::other)?;
        for i in 1..=40u64 {
            db.put(b"key", i.to_be_bytes())?;
        }
        db.sync()?;
        db.flush().map_err(io::Error::other)?;
        drop(db);
        let db = Kiban::open_with_options(&dir, model::options()).map_err(io::Error::other)?;
        let got = db.get(b"key").map_err(io::Error::other)?;
        let latest = got
            .as_ref()
            .map(|v| u64::from_be_bytes(v.as_slice().try_into().unwrap()));
        let scan = model::scan(&db)?;
        let scanned = scan
            .get(b"key".as_slice())
            .map(|v| u64::from_be_bytes(v.as_slice().try_into().unwrap()));
        let passed = latest == Some(40) && scanned == Some(40);
        writeln!(
            out,
            "multi-version-point-after-reopen,40,{},{}",
            telemetry::csv(&format!("get={latest:?};scan={scanned:?}")),
            passed
        )?;
        failures += usize::from(!passed);
    }
    if failures > 0 {
        Err(io::Error::other(format!(
            "{failures} storage correctness probes failed; see probes.csv"
        )))
    } else {
        Ok(())
    }
}
