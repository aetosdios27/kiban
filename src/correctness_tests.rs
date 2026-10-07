//! Logical visibility must survive changes in physical storage.
use super::*;
use crate::testutil::TempDir;

fn assert_snapshot(db: &Kiban, snap: &Snapshot, expected: Option<&[u8]>) {
    assert_eq!(db.get_at(snap, b"key").unwrap().as_deref(), expected);
    let scanned = db.scan_at(snap).unwrap();
    assert_eq!(
        scanned
            .iter()
            .find(|(k, _)| k == b"key")
            .map(|(_, v)| v.as_slice()),
        expected
    );
}

#[test]
fn snapshot_captures_unflushed_versions_across_rotation_flush_and_compaction() {
    let dir = TempDir::new("snapshot-physical-movement");
    let mut db =
        Kiban::open_with_options(dir.path(), super::compaction_tests::tiny_options()).unwrap();
    let empty = db.snapshot();
    db.put(b"key", b"v1").unwrap();
    let first = db.snapshot();
    db.put(b"key", b"v2").unwrap();
    let second = db.snapshot();
    db.delete(b"key").unwrap();
    let deleted = db.snapshot();
    db.put(b"key", b"v3").unwrap();
    db.freeze().unwrap();
    let frozen = db.snapshot();
    db.put(b"key", b"v4").unwrap();
    let active_and_frozen = db.snapshot();
    let check = |db: &Kiban| {
        assert_snapshot(db, &empty, None);
        assert_snapshot(db, &first, Some(b"v1"));
        assert_snapshot(db, &second, Some(b"v2"));
        assert_snapshot(db, &deleted, None);
        assert_snapshot(db, &frozen, Some(b"v3"));
        assert_snapshot(db, &active_and_frozen, Some(b"v4"));
    };
    check(&db);
    db.sync().unwrap();
    let plan = db.plan_flush().unwrap();
    let output = plan.build().unwrap();
    check(&db); // durable SST not yet installed
    db.commit_flush(plan, output).unwrap();
    check(&db);
    db.flush().unwrap();
    check(&db);
    // Snapshots now pin a table topology containing retained versions too.
    let tables = db.snapshot();
    for round in 0..8 {
        db.delete(b"key").unwrap();
        db.put(format!("other-{round}"), vec![round as u8; 1024])
            .unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        check(&db);
        assert_snapshot(&db, &tables, Some(b"v4"));
    }
    assert!(
        db.version.tables.iter().any(|t| t.level >= 1),
        "compaction must actually run"
    );
    for snap in [
        &empty,
        &first,
        &second,
        &deleted,
        &frozen,
        &active_and_frozen,
        &tables,
    ] {
        db.release_snapshot(snap);
    }
    assert!(db.active_snapshots.is_empty());
}

#[test]
fn snapshot_before_first_flush_preserves_old_value_and_visible_delete() {
    let dir = TempDir::new("snapshot-first-flush");
    let mut db = Kiban::open(dir.path()).unwrap();
    db.put(b"key", b"before").unwrap();
    db.sync().unwrap();
    let value = db.snapshot();
    db.delete(b"key").unwrap();
    let deleted = db.snapshot();
    db.put(b"key", b"after").unwrap();
    db.sync().unwrap();
    db.flush().unwrap();
    assert_snapshot(&db, &value, Some(b"before"));
    assert_snapshot(&db, &deleted, None);
    assert_eq!(db.get(b"key").unwrap(), Some(b"after".to_vec()));
}
