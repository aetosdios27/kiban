//! Regression coverage for durability boundaries exposed by scale validation.
use super::*;
use crate::testutil::TempDir;

#[test]
fn sync_after_freeze_must_durably_cover_the_older_wal() {
    struct DeviceGuard;
    impl Drop for DeviceGuard {
        fn drop(&mut self) {
            sys::clear_fault();
            sys::disable_device_sim();
        }
    }
    let dir = TempDir::new("scale-freeze-sync");
    sys::enable_device_sim();
    let _guard = DeviceGuard;
    let mut db = Kiban::open(dir.path()).unwrap();
    db.put(b"acknowledged", b"must-survive").unwrap();
    // Exactly the automatic SharedKiban handoff, before the worker builds its
    // frozen table. The existing simulator is thread-local, so use the same
    // private pipeline operation here instead of an unrelated crash model.
    db.freeze().unwrap();
    db.sync().unwrap(); // Public contract: every previously accepted write durable.
    drop(db);
    sys::power_loss();
    let recovered = Kiban::open(dir.path()).unwrap();
    assert_eq!(
        recovered.get(b"acknowledged").unwrap(),
        Some(b"must-survive".to_vec()),
        "successful sync after freeze must cover the MANIFEST's older live WAL"
    );
}

#[test]
fn repeated_simulated_recovery_sweeps_orphans_in_the_device_namespace() {
    struct DeviceGuard;
    impl Drop for DeviceGuard {
        fn drop(&mut self) {
            sys::disable_device_sim();
        }
    }
    let dir = TempDir::new("scale-simulated-orphans");
    sys::enable_device_sim();
    let _device = DeviceGuard;
    drop(Kiban::open(dir.path()).unwrap());
    let sst = dir.path().join("999.sst");
    let wal = dir.path().join("888.wal");
    let temporary = dir.path().join(".orphan.kiban-tmp.1.1");
    atomic::commit_file(&sst, b"unreferenced table").unwrap();
    atomic::create_durably(&wal).unwrap();
    atomic::create_durably(&temporary).unwrap();
    let nested = dir.path().join("other-db");
    std::fs::create_dir(&nested).unwrap();
    let nested_wal = nested.join("777.wal");
    atomic::create_durably(&nested_wal).unwrap();
    sys::power_loss();
    let db = Kiban::open(dir.path()).unwrap();
    for p in [sst, wal, temporary] {
        assert!(!sys::exists(&p), "simulated orphan survived reopen: {p:?}");
    }
    assert!(
        sys::exists(&dir.path().join("1.wal")),
        "live WAL must remain"
    );
    assert!(
        sys::exists(&nested_wal),
        "sweeping one database must not touch nested namespaces"
    );
    assert!(db.get(b"missing").unwrap().is_none());
}

struct DeviceGuard;
impl DeviceGuard {
    fn new() -> Self {
        sys::enable_device_sim();
        Self
    }
}
impl Drop for DeviceGuard {
    fn drop(&mut self) {
        sys::clear_fault();
        sys::disable_device_sim();
    }
}

/// Sweep freeze, both generations' syncs, SST creation/fsync/rename,
/// MANIFEST installation, and WAL cleanup. Each failed syscall is a
/// deterministic crash boundary; no acknowledgement is inferred from errors.
#[test]
fn frozen_wal_barrier_survives_every_flush_fault_and_power_loss() {
    fn run(faults: &[usize]) -> usize {
        let dir = TempDir::new("frozen-wal-barrier-sweep");
        let _device = DeviceGuard::new();
        let mut db = Kiban::open(dir.path()).unwrap();
        db.put(b"baseline", b"durable").unwrap();
        db.put(b"deleted", b"old").unwrap();
        db.sync().unwrap();
        db.put(b"frozen", b"older-generation").unwrap();
        db.delete(b"deleted").unwrap();
        sys::install_faults(faults);
        let mut barrier = false;
        let result = (|| -> Result<(), DbError> {
            db.freeze()?;
            db.put(b"active", b"newer-generation")?;
            db.sync()?;
            barrier = true;
            let plan = db.plan_flush().unwrap();
            let output = plan.build()?;
            db.commit_flush(plan, output)?;
            Ok(())
        })();
        let count = sys::op_count();
        if faults.is_empty() {
            result.unwrap();
        }
        // No post-error retry/flush. Buffered bytes may reach the volatile
        // layer on drop, but must be discarded by the simulated power loss.
        drop(db);
        sys::clear_fault();
        sys::power_loss();
        let recovered = Kiban::open(dir.path()).unwrap();
        assert_eq!(
            recovered.get(b"baseline").unwrap(),
            Some(b"durable".to_vec()),
            "{faults:?}"
        );
        if barrier {
            assert_eq!(
                recovered.get(b"frozen").unwrap(),
                Some(b"older-generation".to_vec()),
                "{faults:?}"
            );
            assert_eq!(
                recovered.get(b"active").unwrap(),
                Some(b"newer-generation".to_vec()),
                "{faults:?}"
            );
            assert_eq!(
                recovered.get(b"deleted").unwrap(),
                None,
                "{faults:?}: deleted data resurrected"
            );
        } else {
            assert!(
                matches!(
                    recovered.get(b"deleted").unwrap().as_deref(),
                    None | Some(b"old")
                ),
                "{faults:?}"
            );
        }
        let manifest = Manifest::load(dir.path()).unwrap().unwrap();
        assert_eq!(
            manifest.wal_numbers.len(),
            1,
            "recovery consolidates the live generations"
        );
        for number in manifest.wal_numbers {
            assert!(sys::exists(
                &dir.path().join(file_name(number, WAL_EXTENSION))
            ));
        }
        count
    }
    let count = run(&[]);
    assert!(count > 20);
    for a in 0..count {
        run(&[a]);
        for b in a + 1..count {
            run(&[a, b]);
        }
    }
}

#[test]
fn frozen_wal_sync_failure_poisoning_covers_both_generations() {
    for fault in 0..4 {
        let dir = TempDir::new("frozen-wal-sync-failure");
        let _device = DeviceGuard::new();
        let mut db = Kiban::open(dir.path()).unwrap();
        db.put(b"old", b"v").unwrap();
        db.freeze().unwrap();
        db.put(b"new", b"v").unwrap();
        sys::install_fault(fault);
        assert!(
            db.sync().is_err(),
            "fault {fault} must hit a WAL write or sync"
        );
        assert!(db.is_poisoned());
        assert!(db.put(b"unsafe", b"ack").is_err());
        assert!(db.sync().is_err());
    }
}

#[test]
fn repeated_frozen_wal_generations_remain_durable_before_and_after_commit() {
    let dir = TempDir::new("frozen-wal-generations");
    let _device = DeviceGuard::new();
    for stop in 0..3 {
        let mut db = Kiban::open(dir.path()).unwrap();
        for round in 0..4 {
            let key = format!("{stop}-{round}");
            db.put(&key, b"durable").unwrap();
            db.freeze().unwrap();
            db.sync().unwrap();
            if round == 3 && stop == 0 {
                break; // crash with immutable + active WAL, before BUILD
            }
            let plan = db.plan_flush().unwrap();
            let output = plan.build().unwrap();
            if round == 3 && stop == 1 {
                break; // durable orphan SST, old WAL still MANIFEST-live
            }
            db.commit_flush(plan, output).unwrap();
        }
        drop(db);
        sys::power_loss();
        let recovered = Kiban::open(dir.path()).unwrap();
        for previous in 0..=stop {
            for round in 0..4 {
                assert_eq!(
                    recovered.get(format!("{previous}-{round}")).unwrap(),
                    Some(b"durable".to_vec())
                );
            }
        }
    }
}

#[test]
fn durable_acknowledgements_after_a_flush_error_remain_recoverable() {
    fn run(fault: Option<usize>) -> usize {
        let dir = TempDir::new("flush-error-then-ack");
        let _device = DeviceGuard::new();
        let mut db = Kiban::open(dir.path()).unwrap();
        db.put(b"baseline", b"durable").unwrap();
        db.sync().unwrap();
        sys::install_faults_with_read_opens(&fault.into_iter().collect::<Vec<_>>());
        let result = db.flush_without_compaction();
        let count = sys::op_count();
        sys::clear_fault();
        if fault.is_none() {
            result.as_ref().unwrap();
        }
        let acknowledged = !db.is_poisoned();
        if acknowledged {
            db.put(b"later", b"durable-after-error").unwrap();
            db.sync().unwrap();
        } else {
            assert!(result.is_err());
            assert!(db.put(b"later", b"unsafe").is_err());
            assert!(db.sync().is_err());
        }
        drop(db);
        sys::power_loss();
        let recovered = Kiban::open(dir.path()).unwrap();
        assert_eq!(
            recovered.get(b"baseline").unwrap(),
            Some(b"durable".to_vec()),
            "fault={fault:?}"
        );
        if acknowledged {
            assert_eq!(
                recovered.get(b"later").unwrap(),
                Some(b"durable-after-error".to_vec()),
                "fault={fault:?}: accepted durable write went to a retired WAL"
            );
        }
        count
    }
    let count = run(None);
    for fault in 0..count {
        run(Some(fault));
    }
}

#[test]
fn failed_flush_retry_never_reuses_a_cached_sst_identity() {
    fn run(fault: Option<usize>) -> usize {
        let dir = TempDir::new("flush-retry-cache-identity");
        let _device = DeviceGuard::new();
        let mut db = Kiban::open(dir.path()).unwrap();
        db.put(b"key", b"first").unwrap();
        db.sync().unwrap();
        sys::install_faults_with_read_opens(&fault.into_iter().collect::<Vec<_>>());
        let first = db.flush_without_compaction();
        let count = sys::op_count();
        sys::clear_fault();
        if !db.is_poisoned() {
            let next = db.next_file_number.load(Ordering::SeqCst);
            assert!(
                next >= 4,
                "even a failed attempt must reserve its file identities"
            );
            db.put(b"key", b"second").unwrap();
            db.sync().unwrap();
            db.flush_without_compaction().unwrap();
            assert_eq!(
                db.get(b"key").unwrap(),
                Some(b"second".to_vec()),
                "fault={fault:?}"
            );
            let scan: Vec<_> = db.iter().map(Result::unwrap).collect();
            assert_eq!(scan, vec![(b"key".to_vec(), b"second".to_vec())]);
            drop(db);
            sys::power_loss();
            let recovered = Kiban::open(dir.path()).unwrap();
            assert_eq!(
                recovered.get(b"key").unwrap(),
                Some(b"second".to_vec()),
                "fault={fault:?}"
            );
        } else {
            assert!(first.is_err());
        }
        count
    }
    let count = run(None);
    for fault in 0..count {
        run(Some(fault));
    }
}
