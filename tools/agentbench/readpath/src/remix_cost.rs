//! Diagnostic separation of routing, JSON decode, and memtable reconciliation.
use super::*;

pub(super) fn run(o: &Options) -> Result<()> {
    let c = &o.suite;
    let mut db = Kiban::open_with_options(
        &c.db,
        KibanOptions {
            point_index: true,
            scan_prefix_len: 18,
            block_cache_bytes: o.cache_mib.map_or_else(
                || KibanOptions::default().block_cache_bytes,
                |m| m * 1024 * 1024,
            ),
            ..KibanOptions::default()
        },
    )?;
    let tick = Instant::now();
    let ready = db.prepare_range_index();
    let ns = tick.elapsed().as_nanos();
    println!(
        "{}",
        json!({"type":"scan_build","ready":ready,"elapsed_ns":ns,"stats":format!("{:?}",db.range_index_stats())})
    );
    assert!(ready);
    let mut floor = Vec::new();
    for agent in 0..model::AGENTS {
        let start = format!("agent/{agent:04}/memory/");
        let end = format!("agent/{agent:04}/memory0");
        floor.push(
            db.range(start.as_bytes(), end.as_bytes())
                .take(c.k)
                .map(|r| r.map(|(_, v)| v))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
    }
    for phase in [0, 1, 10] {
        if phase != 0 {
            for agent in 0..model::AGENTS {
                for j in 0..phase {
                    let id = c.n - 1 - (c.n - 1 - agent) % model::AGENTS - j * model::AGENTS;
                    let memory = model::Memory::new(id, 0, c.seed);
                    db.put(memory.key(), serde_json::to_vec(&memory)?)?;
                }
            }
        }
        for mode in [
            "bounds",
            "prefix-bounds",
            "routing",
            "prefix-routing",
            "decoded",
            "prefix-decoded",
            "floor",
            "prefix-floor",
        ] {
            if phase != 0 && matches!(mode, "floor" | "prefix-floor" | "bounds" | "prefix-bounds") {
                continue;
            }
            let mut histogram = metrics::Histogram::default();
            let tick = Instant::now();
            let mut ns = 0;
            for op in 0..c.operations {
                let agent = random::mix(op ^ c.seed) % model::AGENTS;
                let started = Instant::now();
                let start = format!("agent/{agent:04}/memory/");
                let end =
                    (!mode.starts_with("prefix")).then(|| format!("agent/{agent:04}/memory0"));
                let records: Vec<model::Memory> = if mode.ends_with("bounds") {
                    black_box((&start, &end));
                    Vec::new()
                } else if mode == "prefix-routing" {
                    let records = db.prefix_values(start.as_bytes(), c.k, |value| {
                        Ok::<_, Box<dyn std::error::Error>>(black_box(value).len())
                    })?;
                    assert_eq!(black_box(records).len(), c.k);
                    Vec::new()
                } else if mode == "routing" {
                    let records = db.range_values(
                        start.as_bytes(),
                        end.as_ref().unwrap().as_bytes(),
                        c.k,
                        |value| Ok::<_, Box<dyn std::error::Error>>(black_box(value).len()),
                    )?;
                    assert_eq!(black_box(records).len(), c.k);
                    Vec::new()
                } else if mode == "floor" || mode == "prefix-floor" {
                    black_box((&start, &end));
                    floor[agent as usize]
                        .iter()
                        .map(|v| Ok::<_, Box<dyn std::error::Error>>(serde_json::from_slice(v)?))
                        .collect::<Result<Vec<_>>>()?
                } else if mode == "prefix-decoded" {
                    db.prefix_values(start.as_bytes(), c.k, |v| {
                        Ok::<_, Box<dyn std::error::Error>>(serde_json::from_slice(v)?)
                    })?
                } else {
                    db.range_values(
                        start.as_bytes(),
                        end.as_ref().unwrap().as_bytes(),
                        c.k,
                        |v| Ok::<_, Box<dyn std::error::Error>>(serde_json::from_slice(v)?),
                    )?
                };
                let elapsed = started.elapsed().as_nanos();
                ns += elapsed;
                histogram.record(elapsed as u64);
                if !matches!(
                    mode,
                    "routing" | "prefix-routing" | "bounds" | "prefix-bounds"
                ) {
                    assert_eq!(records.len(), c.k);
                    for (memory, id) in
                        records
                            .iter()
                            .zip(model::recent_ids(c.n, agent as u32, c.k))
                    {
                        runner::check(memory, id, 0, c.seed)?;
                    }
                }
            }
            println!(
                "{}",
                runner::row(
                    &format!("{mode}-mem-{phase}"),
                    &histogram,
                    &histogram,
                    &metrics::Histogram::default(),
                    tick,
                    ns,
                    0
                )
            );
        }
    }
    Ok(())
}
