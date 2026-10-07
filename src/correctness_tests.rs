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

fn assert_current(db: &Kiban, expected: Option<&[u8]>) {
    assert_eq!(db.get(b"key").unwrap().as_deref(), expected);
    let scanned: Vec<_> = db.iter().map(Result::unwrap).collect();
    assert_eq!(
        scanned
            .iter()
            .find(|(k, _)| k == b"key")
            .map(|(_, v)| v.as_slice()),
        expected
    );
}

#[test]
fn point_and_scan_agree_on_many_versions_after_flush_and_reopen() {
    let dir = TempDir::new("point-many-versions");
    let mut db = Kiban::open(dir.path()).unwrap();
    for seq in 1..=40u64 {
        db.put(b"key", seq.to_be_bytes()).unwrap();
    }
    assert_current(&db, Some(&40u64.to_be_bytes()));
    db.sync().unwrap();
    db.flush().unwrap();
    assert_current(&db, Some(&40u64.to_be_bytes()));
    drop(db);
    let mut db = Kiban::open(dir.path()).unwrap();
    assert_current(&db, Some(&40u64.to_be_bytes()));
    let snap = db.snapshot(); // table-backed, not a captured memtable
    assert_snapshot(&db, &snap, Some(&40u64.to_be_bytes()));
    for _ in 0..40 {
        db.delete(b"key").unwrap();
    }
    db.flush().unwrap();
    assert_current(&db, None);
    assert_snapshot(&db, &snap, Some(&40u64.to_be_bytes()));
}

/// A valid MANIFEST carries file numbers and levels, not a guarantee
/// that their order sorts every version. Deliberately decouple both
/// from sequence order and compare point selection with the merge oracle.
#[test]
fn point_selection_uses_sequence_not_file_or_level_order() {
    let dir = TempDir::new("point-sequence-order");
    let mut db = Kiban::open(dir.path()).unwrap();
    let mut tables = Vec::new();
    for (number, level, seq, kind) in [
        (10, 0, 30u64, Kind::Put),
        (20, 0, 10, Kind::Put),
        (30, 1, 40, Kind::Tombstone),
        (40, 2, 20, Kind::Put),
    ] {
        let mut builder = TableBuilder::new();
        let value = seq.to_be_bytes();
        builder
            .add(
                kind,
                b"key",
                if kind == Kind::Put { &value } else { b"" },
                seq,
            )
            .unwrap();
        let path = dir.path().join(file_name(number, SST_EXTENSION));
        atomic::commit_file(&path, &builder.finish().unwrap()).unwrap();
        let table = SstTable::open(
            number,
            &path,
            db.cache.clone(),
            db.file_cache.clone(),
            false,
        )
        .unwrap();
        tables.push(StdArc::new(TableEntry {
            file_lifetime: StdArc::new(()),
            number,
            level,
            size: table.size_on_disk(),
            first_key: table.smallest_key().to_vec(),
            last_key: table.largest_key().to_vec(),
            table,
        }));
    }
    Manifest {
        next_file_number: 41,
        wal_numbers: vec![db.wal_number],
        last_sequence: 40,
        tables: tables
            .iter()
            .map(|t| TableRef {
                number: t.number,
                level: t.level,
            })
            .collect(),
    }
    .install(dir.path())
    .unwrap();
    db.version = StdArc::new(Version { id: 1, tables });
    db.last_sequence = 40;
    assert_current(&db, None); // newest visible version is a deeper tombstone
    for seq in 0..=40 {
        let snap = Snapshot {
            seq,
            memtable: Memtable::new(),
            immutable: None,
            version: db.version.clone(),
        };
        let expected = match seq {
            10..20 => Some(10u64.to_be_bytes()),
            20..30 => Some(20u64.to_be_bytes()),
            30..40 => Some(30u64.to_be_bytes()),
            _ => None,
        };
        assert_snapshot(&db, &snap, expected.as_ref().map(|v| v.as_slice()));
    }
    drop(db);
    let db = Kiban::open(dir.path()).unwrap();
    assert_current(&db, None);
    // Shared point/snapshot paths must use the same table resolver.
    drop(db);
    let shared = SharedKiban::open(dir.path()).unwrap();
    assert_eq!(shared.get(b"key").unwrap(), None);
    let snap = shared.snapshot().unwrap();
    assert_eq!(snap.get(b"key").unwrap(), None);
    assert!(snap.scan().unwrap().is_empty());
}

#[test]
fn point_scan_and_snapshots_agree_through_overlapping_generations_and_compaction() {
    let dir = TempDir::new("point-scan-transitions");
    let mut options = super::compaction_tests::tiny_options();
    options.l0_compaction_trigger = 4;
    options.l0_write_stall_trigger = 8;
    let mut db = Kiban::open_with_options(dir.path(), options).unwrap();
    let mut snapshots = Vec::new();
    let mut expected = None;
    for round in 0..8 {
        for i in 0..40 {
            if i % 11 == 0 {
                db.delete(b"key").unwrap();
                expected = None;
            } else {
                let value = format!("{round}-{i}").into_bytes();
                db.put(b"key", &value).unwrap();
                expected = Some(value);
            }
            assert_current(&db, expected.as_deref());
        }
        snapshots.push((db.snapshot(), expected.clone()));
        db.freeze().unwrap();
        assert_current(&db, expected.as_deref());
        // New active tombstone must hide an immutable value; old
        // snapshots must ignore this future tombstone.
        db.delete(b"key").unwrap();
        expected = None;
        assert_current(&db, None);
        db.sync().unwrap();
        let plan = db.plan_flush().unwrap();
        let output = plan.build().unwrap();
        db.commit_flush(plan, output).unwrap();
        assert_current(&db, None);
        // Flush the new tombstone as a separate, overlapping L0 file.
        db.flush_without_compaction().unwrap();
        assert_current(&db, None);
        for (snap, want) in &snapshots {
            assert_snapshot(&db, snap, want.as_deref());
        }
        db.maybe_compact().unwrap();
        assert_current(&db, None);
        for (snap, want) in &snapshots {
            assert_snapshot(&db, snap, want.as_deref());
        }
        snapshots.push((db.snapshot(), None));
    }
    assert!(db.version.tables.iter().any(|t| t.level >= 1));
    for (snap, _) in &snapshots {
        db.release_snapshot(snap);
    }
    drop(snapshots);
    drop(db);
    let db = Kiban::open(dir.path()).unwrap();
    assert_current(&db, expected.as_deref());
}

#[test]
fn snapshots_pin_the_same_sst_across_repeated_metadata_moves_and_rewrite() {
    let dir = TempDir::new("snapshot-moved-file-lifetime");
    let options = KibanOptions {
        block_cache_bytes: 0,
        max_open_table_files: 1,
        l0_compaction_trigger: 1000,
        l0_write_stall_trigger: 2000,
        ..KibanOptions::default()
    };
    let mut db = Kiban::open_with_options(dir.path(), options).unwrap();
    db.put(b"key", b"before").unwrap();
    db.flush_without_compaction().unwrap();
    let plan = db.plan_compaction_at_level(0).unwrap();
    let output = plan.build().unwrap();
    db.commit_compaction(plan, output).unwrap();
    let number = db.version.tables[0].number;
    let path = dir.path().join(file_name(number, SST_EXTENSION));
    let first = db.snapshot();
    let plan = db.plan_trivial_move(&[number], 1).unwrap();
    let output = plan.build().unwrap();
    db.commit_compaction(plan, output).unwrap();
    let second = db.snapshot();
    let plan = db.plan_trivial_move(&[number], 2).unwrap();
    let output = plan.build().unwrap();
    db.commit_compaction(plan, output).unwrap();
    let plan = db.plan_compaction_at_level(3).unwrap(); // actual rewrite retires old file
    let output = plan.build().unwrap();
    db.commit_compaction(plan, output).unwrap();
    assert!(
        path.exists(),
        "old metadata views still pin the physical SST"
    );
    assert_snapshot(&db, &first, Some(b"before"));
    assert_snapshot(&db, &second, Some(b"before"));
    db.delete(b"key").unwrap();
    db.flush_without_compaction().unwrap();
    assert_current(&db, None);
    assert_snapshot(&db, &first, Some(b"before"));
    assert_snapshot(&db, &second, Some(b"before"));
    db.release_snapshot(&first);
    drop(first);
    db.reclaim_obsolete();
    assert!(
        path.exists(),
        "second snapshot must independently pin the same file"
    );
    db.release_snapshot(&second);
    drop(second);
    db.reclaim_obsolete();
    assert!(
        !path.exists(),
        "reclamation proceeds once all metadata aliases are unpinned"
    );
    assert_current(&db, None);
}
