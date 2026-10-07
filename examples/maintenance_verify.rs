//! Full post-run verification for the existing scale workload, outside its timed window.
#[path = "scale/config.rs"]
mod config;
#[path = "support/random.rs"]
mod random;
#[path = "scale/workload.rs"]
mod workload;

use kiban::db::Kiban;
use std::io;

fn verify(c: config::Config) -> io::Result<()> {
    if c.duration != 0.0 && c.preload_keys != c.keys {
        return Err(io::Error::other(
            "timed verification requires the entire key space to be preloaded",
        ));
    }
    let mut expected = vec![false; usize::try_from(c.keys).map_err(io::Error::other)?];
    expected[..c.preload_keys as usize].fill(true);
    let stages = workload::stages(&c);
    for (index, stage) in stages.iter().enumerate() {
        if c.duration != 0.0 {
            break;
        } // all preloaded keys retain deterministic per-key values
        let n = c.operations / stages.len() as u64
            + u64::from((index as u64) < c.operations % stages.len() as u64);
        println!(
            "stage={} threads={} operations={n}",
            stage.name, stage.threads
        );
        if stage.reads == 100 {
            continue;
        }
        for id in 0..n {
            let (read, key) = workload::operation(&c, stage, index as u64, id);
            if !read {
                expected[u64::from_be_bytes(key) as usize] = true;
            }
        }
    }
    let mut value = random::value(c.seed, c.value_size);
    let db = Kiban::open_with_options(&c.db, c.options).map_err(io::Error::other)?;
    let mut count = 0usize;
    let mut previous = None;
    for item in db.iter() {
        let (key, got) = item.map_err(io::Error::other)?;
        let bytes: [u8; 8] = key.as_slice().try_into().map_err(io::Error::other)?;
        let k = usize::try_from(u64::from_be_bytes(bytes)).map_err(io::Error::other)?;
        if c.value_pattern == "keyed" {
            random::keyed_value(c.seed, k as u64, &mut value);
        }
        if k >= expected.len() || !expected[k] || previous.is_some_and(|p| p >= k) || got != value {
            return Err(io::Error::other("post-run key/value/ordering mismatch"));
        }
        if count.is_multiple_of(4096)
            && db.get(&key).map_err(io::Error::other)?.as_deref() != Some(value.as_slice())
        {
            return Err(io::Error::other("post-run point/scan mismatch"));
        }
        previous = Some(k);
        count += 1;
    }
    let wanted = expected.into_iter().filter(|present| *present).count();
    if count != wanted {
        return Err(io::Error::other("post-run missing keys"));
    }
    println!("verified_keys={count}");
    Ok(())
}
fn main() {
    if std::env::args().any(|a| a == "--help") {
        println!("{}", config::HELP);
        return;
    }
    if let Err(e) = config::Config::parse()
        .map_err(io::Error::other)
        .and_then(verify)
    {
        eprintln!("maintenance verification FAILED: {e}");
        std::process::exit(1);
    }
}
