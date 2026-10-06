use super::{
    config::Config,
    model::{self, Allowed, Model},
    telemetry, workload,
};
use kiban::{db::Kiban, sys};
use std::io::{self, Write};
use std::time::Instant;

pub fn run(c: &Config) -> io::Result<()> {
    sys::enable_device_sim();
    let result = campaign(c);
    sys::clear_fault();
    sys::disable_device_sim();
    result
}
fn campaign(c: &Config) -> io::Result<()> {
    let mut out = telemetry::create(&c.output.join("cycles.csv"))?;
    writeln!(
        out,
        "cycle,planned_ops,completed_ops,fault_index,checked_syscalls,fault_triggered,crash_elapsed_s,reopen_s,verify_s,live_keys,verification"
    )?;
    let mut floor = Model::new();
    for cycle in 0..c.cycles {
        let mut db = Kiban::open_with_options(&c.db, model::options()).map_err(io::Error::other)?;
        let mut live = floor.clone();
        let mut allowed = Allowed::new();
        let mut ambiguous_flush = false;
        // Pin a real snapshot across updates, deletes, flushes and compactions.
        let snapshot = c.snapshots.then(|| db.snapshot());
        let snapshot_model = live.clone();
        let crash_after = 1 + workload::mix(c.seed ^ cycle) % c.operations;
        let fault = if workload::mix(c.seed ^ cycle ^ 0xd3) % 100 < c.fault_percent {
            Some(
                (workload::mix(c.seed ^ cycle ^ 0xf1) % (crash_after.saturating_mul(3).max(1)))
                    as usize,
            )
        } else {
            None
        };
        sys::install_fault(fault.unwrap_or(usize::MAX));
        let start = Instant::now();
        let mut completed = 0u64;
        for i in 0..crash_after {
            let id = cycle * c.operations + i;
            let (k, v) = model::mutation(c.seed, id, c.keys);
            let result = match &v {
                Some(v) => db.put(&k, v),
                None => db.delete(&k),
            };
            if result.is_err() {
                break;
            }
            allowed.entry(k.clone()).or_default().push(v.clone());
            model::apply(&mut live, k, v);
            completed += 1;
            if completed.is_multiple_of(c.sync_every) {
                if db.sync().is_err() {
                    break;
                }
                floor = live.clone();
                allowed.clear();
                ambiguous_flush = false;
            }
            // Explicit flush needed for direct Kiban: it has no auto-freeze worker.
            if completed.is_multiple_of(31) {
                if db.flush().is_err() {
                    ambiguous_flush = true;
                    break;
                }
                floor = live.clone();
                allowed.clear();
                ambiguous_flush = false;
            }
        }
        let checked = sys::op_count();
        sys::clear_fault();
        if let Some(snapshot) = snapshot {
            let snap_scan: Model = db
                .scan_at(&snapshot)
                .map_err(io::Error::other)?
                .into_iter()
                .collect();
            if snap_scan != snapshot_model {
                return Err(io::Error::other(format!(
                    "cycle={cycle}: pinned snapshot changed (expected {} live keys, scanned {})",
                    snapshot_model.len(),
                    snap_scan.len()
                )));
            }
            for (k, v) in &snapshot_model {
                if db.get_at(&snapshot, k).map_err(io::Error::other)?.as_ref() != Some(v) {
                    return Err(io::Error::other("snapshot get/scan mismatch"));
                }
            }
            db.release_snapshot(&snapshot);
        }
        drop(db);
        sys::power_loss();
        let crash_elapsed = start.elapsed().as_secs_f64();
        let t = Instant::now();
        let recovered =
            Kiban::open_with_options(&c.db, model::options()).map_err(io::Error::other)?;
        let reopen = t.elapsed().as_secs_f64();
        let t = Instant::now();
        // Unsynced values may have been durably published by an ambiguous flush.
        // Enforce exact floor when no such values exist; otherwise enforce known per-key band.
        let verification = if !ambiguous_flush {
            allowed.clear();
            "exact"
        } else {
            "durable-floor-and-attempt-band"
        };
        floor = model::verify(&recovered, &floor, &allowed, c.keys)
            .map_err(|e| io::Error::other(format!("cycle={cycle} fault={fault:?}: {e}")))?;
        let verify = t.elapsed().as_secs_f64();
        drop(recovered);
        writeln!(
            out,
            "{cycle},{crash_after},{completed},{},{checked},{},{crash_elapsed},{reopen},{verify},{},{}",
            fault.map(|f| f.to_string()).unwrap_or_default(),
            fault.is_some_and(|f| checked > f),
            floor.len(),
            verification
        )?;
        if cycle.is_multiple_of(100) {
            eprintln!("simulated recovery cycle {cycle}/{} verified", c.cycles);
        }
    }
    Ok(())
}
