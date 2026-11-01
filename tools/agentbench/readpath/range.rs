//! Identical range probe linked against archived main and final engine libraries.
#[path = "../src/model.rs"]
mod model;
#[path = "../../../examples/support/random.rs"]
mod random;
use kiban::db::{Kiban, KibanOptions};
use std::{hint::black_box, time::Instant};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3);
    assert!(
        std::fs::read_to_string("/proc/self/status")?
            .lines()
            .any(|line| line == "Cpus_allowed_list:\t8")
    );
    let mut options = KibanOptions::default();
    if args[2] != "default" {
        options.block_cache_bytes = args[2].parse::<usize>()? * 1024 * 1024;
    }
    let cache_bytes = options.block_cache_bytes;
    let db = Kiban::open_with_options(&args[1], options)?;
    let n = 1_000_000u64;
    // Identical warmup on both libraries, without timing it.
    for id in 0..n {
        black_box(db.get(model::key(id))?.ok_or("missing warmup value")?);
    }
    for count in [10usize, 256] {
        let mut samples = Vec::new();
        for op in 0..1000u64 {
            let agent = op % model::AGENTS;
            let newest = n - 1 - (n - 1 - agent) % model::AGENTS;
            let first = newest - (random::mix(op) % 1000) * model::AGENTS;
            let start = model::key(first);
            let end = format!("agent/{agent:04}/memory0");
            let tick = Instant::now();
            let rows = db
                .range(start.as_bytes(), end.as_bytes())
                .take(count)
                .collect::<Result<Vec<_>, _>>()?;
            samples.push(tick.elapsed().as_nanos() as u64);
            assert_eq!(rows.len(), count);
            for (i, (key, value)) in rows.iter().enumerate() {
                let id = first - i as u64 * model::AGENTS;
                assert_eq!(key, model::key(id).as_bytes());
                let memory: model::Memory = serde_json::from_slice(value)?;
                assert_eq!(memory, model::Memory::new(id, 0, 20261016));
            }
            black_box(rows);
        }
        samples.sort_unstable();
        println!(
            "{}",
            serde_json::json!({"type":"range","count":count,
            "operations":samples.len(),"verified_records":samples.len()*count,
            "p50_ns":samples[500],"p99_ns":samples[990],
            "cache_bytes":cache_bytes,"errors":0})
        );
    }
    Ok(())
}
