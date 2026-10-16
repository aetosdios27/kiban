//! Crash simulation across the engine's own worker threads.

use super::*;
use crate::sys;
use crate::testutil::TempDir;
use std::collections::BTreeMap;

/// The background flush/compaction worker must run on the same simulated
/// disk as the caller, otherwise power loss only covers foreground I/O.
#[test]
fn shared_engine_worker_runs_on_the_simulated_device() {
    let dir = TempDir::new("sim-shared");
    sys::enable_device_sim();
    let mut synced: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    {
        let db =
            SharedKiban::open_with_options(dir.path(), super::compaction_tests::tiny_options())
                .unwrap();
        for round in 0..6u32 {
            for i in 0..40u32 {
                let k = format!("k{i:03}").into_bytes();
                let v = format!("r{round}-{i}").into_bytes();
                db.put(&k, &v).unwrap();
                synced.insert(k, v);
            }
            // Flush plus compaction run on the worker thread.
            db.checkpoint().unwrap();
        }
        assert!(db.maintenance_error().is_none());
        // Written after the last durable point: must vanish on power loss.
        for i in 0..10u32 {
            db.put(format!("tail{i}"), b"unsynced").unwrap();
        }
        assert!(
            !sys::simulated_files_under(dir.path()).is_empty(),
            "no file reached the simulated device"
        );
        sys::power_loss();
    }
    sys::power_loss();
    let db = Kiban::open_with_options(dir.path(), super::compaction_tests::tiny_options()).unwrap();
    let recovered: BTreeMap<Vec<u8>, Vec<u8>> = db.iter().map(|r| r.unwrap()).collect();
    drop(db);
    sys::disable_device_sim();
    assert_eq!(recovered, synced);
}
