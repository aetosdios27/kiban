//! A separate robustness probe: unique, widely distributed starts within agents.
use super::{Config, Options, Result, backend, metrics, model, open, runner};
use heed3::{Database, Env, EnvOpenOptions, types::Bytes};
use kiban::db::{Kiban, KibanOptions};
use std::{collections::HashSet, ops::Bound, time::Instant};

type Records = Vec<model::Memory>;

enum Reader {
    Kiban(Box<Kiban>),
    Lmdb(Env, Database<Bytes, Bytes>),
}

impl Reader {
    fn range(&self, start: &[u8], end: &[u8], k: usize) -> Result<Records> {
        match self {
            #[cfg(feature = "remix")]
            Self::Kiban(db) => {
                db.range_values(start, end, k, |value| Ok(serde_json::from_slice(value)?))
            }
            #[cfg(not(feature = "remix"))]
            Self::Kiban(db) => db
                .range(start, end)
                .take(k)
                .map(|entry| {
                    let (_, value) = entry?;
                    Ok(serde_json::from_slice(&value)?)
                })
                .collect(),
            Self::Lmdb(env, records) => {
                let txn = env.read_txn()?;
                let bounds = (Bound::Included(start), Bound::Excluded(end));
                let result = records
                    .range(&txn, &bounds)?
                    .take(k)
                    .map(|entry| {
                        let (_, value) = entry?;
                        Ok(serde_json::from_slice(value)?)
                    })
                    .collect();
                txn.commit()?;
                result
            }
        }
    }
}

fn starts(c: &Config) -> Result<Vec<u64>> {
    if !c.n.is_multiple_of(model::AGENTS) || c.k == 0 || c.k as u64 > c.n / model::AGENTS {
        return Err("varied bounds require equal nonempty agent populations".into());
    }
    let slots = c.n / model::AGENTS - c.k as u64 + 1;
    let mut seen = HashSet::new();
    let ids = (0..c.operations)
        .map(|op| {
            let agent = op % model::AGENTS;
            let position = ((u128::from(op / model::AGENTS) * 7919
                + u128::from(super::random::mix(c.seed ^ agent)))
                % u128::from(slots)) as u64;
            agent + model::AGENTS * (position + c.k as u64 - 1)
        })
        .collect::<Vec<_>>();
    if !ids.iter().all(|id| seen.insert(*id)) {
        return Err("varied-bound starts repeated; reduce operations".into());
    }
    Ok(ids)
}

pub(super) fn run(o: &Options) -> Result<()> {
    let c = &o.suite;
    let ids = starts(c)?;
    {
        let mut verifier = open(o)?;
        runner::verify_initial(verifier.as_mut(), c)?;
    }
    let reader = if c.backend == "kiban" {
        Reader::Kiban(Box::new(Kiban::open_with_options(
            &c.db,
            KibanOptions {
                point_index: o.point_index,
                block_cache_bytes: o.cache_mib.map_or_else(
                    || KibanOptions::default().block_cache_bytes,
                    |mib| mib * 1024 * 1024,
                ),
                ..KibanOptions::default()
            },
        )?))
    } else {
        // This process exclusively owns the private fixture environment.
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024 * 1024)
                .max_dbs(200)
                .max_readers(200)
                .open(&c.db)?
        };
        backend::configure(&env, super::Mode::Fast)?;
        let txn = env.read_txn()?;
        let records = env
            .open_database(&txn, Some("memories"))?
            .ok_or("missing memories")?;
        txn.commit()?;
        Reader::Lmdb(env, records)
    };
    // Warm the same varied distribution once; this is a warm-read robustness probe.
    for &id in &ids {
        let start = model::key(id);
        let end = format!("agent/{:04}/memory0", id % model::AGENTS);
        check(reader.range(start.as_bytes(), end.as_bytes(), c.k)?, id, c)?;
    }
    let mut histogram = metrics::Histogram::default();
    let started = Instant::now();
    let mut ns = 0;
    for &id in &ids {
        let tick = Instant::now();
        let start = model::key(id);
        let end = format!("agent/{:04}/memory0", id % model::AGENTS);
        let records = reader.range(start.as_bytes(), end.as_bytes(), c.k)?;
        let elapsed = tick.elapsed().as_nanos();
        ns += elapsed;
        histogram.record(u64::try_from(elapsed)?);
        check(records, id, c)?;
    }
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&c.output)?;
    runner::emit(
        &mut out,
        &runner::row(
            "varied-bounds",
            &histogram,
            &histogram,
            &metrics::Histogram::default(),
            started,
            ns,
            0,
        ),
    )?;
    runner::emit(
        &mut out,
        &serde_json::json!({"type":"complete","errors":0,
        "unique_bounds":ids.len(),"warmup_calls":ids.len(),"k":c.k}),
    )?;
    drop(reader);
    let mut verifier = open(o)?;
    runner::verify_initial(verifier.as_mut(), c)?;
    Ok(())
}

fn check(records: Records, id: u64, c: &Config) -> Result<()> {
    if records.len() != c.k {
        return Err("varied bound result length mismatch".into());
    }
    for (memory, j) in records.into_iter().zip(0..c.k) {
        let expected = id - j as u64 * model::AGENTS;
        runner::check(&memory, expected, 0, c.seed)?;
    }
    Ok(())
}
