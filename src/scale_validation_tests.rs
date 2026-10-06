//! Explicit regression probe for a durability boundary not exercised by the
//! existing flush fault sweep (that sweep syncs BEFORE every freeze).
//! Kept as a failing reproducer: scale validation does not repair engine policy.
use super::*;
use crate::testutil::TempDir;

#[test]
#[ignore = "Explicit scale probe: run separately to check sync after an unsynced WAL freeze"]
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
