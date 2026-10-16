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

mod device_model {
    use crate::sys::{self, File};
    use crate::testutil::TempDir;
    use std::io::Write;
    use std::path::Path;

    fn write_file(path: &Path, bytes: &[u8], sync: bool) {
        let mut f = File::create_new(path).unwrap();
        f.write_all(bytes).unwrap();
        if sync {
            f.sync_all().unwrap();
        }
    }

    fn sync_dir(dir: &Path) {
        File::open_read(dir).unwrap().sync_all().unwrap();
    }

    /// Runs `f` on a fresh simulated device and always restores passthrough.
    fn on_device(label: &str, f: impl FnOnce(&Path)) {
        let dir = TempDir::new(label);
        sys::enable_device_sim();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(dir.path())));
        sys::disable_device_sim();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn create_without_directory_sync_is_lost() {
        on_device("ns-create", |dir| {
            let path = dir.join("a");
            write_file(&path, b"data", true);
            sys::power_loss();
            assert!(!sys::exists(&path));
        });
    }

    #[test]
    fn directory_sync_makes_entries_durable_but_not_unsynced_bytes() {
        on_device("ns-durable", |dir| {
            let path = dir.join("a");
            write_file(&path, b"data", false);
            sync_dir(dir);
            sys::power_loss();
            assert!(sys::exists(&path));
            assert_eq!(sys::read(&path).unwrap(), b"");
        });
    }

    #[test]
    fn rename_and_remove_revert_without_directory_sync() {
        on_device("ns-revert", |dir| {
            let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
            write_file(&a, b"one", true);
            write_file(&c, b"three", true);
            sync_dir(dir);
            sys::rename(&a, &b).unwrap();
            sys::remove_file(&c).unwrap();
            sys::power_loss();
            assert_eq!(sys::read(&a).unwrap(), b"one");
            assert!(!sys::exists(&b));
            assert_eq!(sys::read(&c).unwrap(), b"three", "unlink must resurrect");
        });
    }

    /// Replays one crash scenario with `seed`; returns what survived.
    fn seeded_outcome(seed: u64) -> (bool, Vec<u8>) {
        let mut out = (false, Vec::new());
        on_device("ns-seeded", |dir| {
            let path = dir.join("log");
            write_file(&path, b"synced:", true);
            sync_dir(dir);
            let mut f = File::open_rw(&path).unwrap();
            f.write_all(&[7u8; 2048]).unwrap();
            write_file(&dir.join("new"), b"x", true);
            sys::power_loss_seeded(seed);
            out = (sys::exists(&dir.join("new")), sys::read(&path).unwrap());
        });
        out
    }

    #[test]
    fn seeded_power_loss_is_deterministic_and_covers_every_tear() {
        let (mut none, mut all, mut partial, mut zeros, mut created, mut lost) =
            (false, false, false, false, false, false);
        for seed in 0..400 {
            let first = seeded_outcome(seed);
            assert_eq!(first, seeded_outcome(seed), "seed {seed} not reproducible");
            let (new_survived, data) = first;
            assert!(
                data.starts_with(b"synced:"),
                "synced bytes lost, seed {seed}"
            );
            let tail = &data[7..];
            assert!(tail.len() <= 2048 && tail.iter().all(|&b| b == 0 || b == 7));
            none |= tail.is_empty();
            all |= tail.len() == 2048 && tail.iter().all(|&b| b == 7);
            partial |= !tail.is_empty() && tail.len() < 2048;
            zeros |= tail.contains(&0);
            created |= new_survived;
            lost |= !new_survived;
        }
        assert!(none && all && partial && zeros && created && lost);
    }
}
