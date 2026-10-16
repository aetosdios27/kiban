//! Regressions for the October 2026 audit findings.
use super::*;
use crate::testutil::TempDir;

#[test]
fn atomic_batch_never_exposes_a_prefix_to_gate_free_reads() {
    let dir = TempDir::new("atomic-publication");
    let db = SharedKiban::open(dir.path()).unwrap();
    db.put(b"key", b"stable").unwrap();
    let checkpoint = db.lock().unwrap().batch_checkpoint.clone();
    checkpoint.arm(1);
    let writer = db.clone();
    let write = std::thread::spawn(move || {
        let mut batch = WriteBatch::new();
        batch
            .put(b"key".to_vec(), b"intermediate".to_vec())
            .delete(b"key".to_vec())
            .put(b"key".to_vec(), b"stable".to_vec())
            .put(b"other".to_vec(), b"final".to_vec());
        writer.write(batch).unwrap();
    });
    checkpoint.wait_reached();
    let reader = db.clone();
    let (started, ready) = std::sync::mpsc::channel();
    let (tx, rx) = std::sync::mpsc::channel();
    let read = std::thread::spawn(move || {
        started.send(()).unwrap();
        tx.send(reader.get(b"key")).unwrap();
    });
    ready.recv().unwrap();
    let premature = rx.recv_timeout(std::time::Duration::from_millis(50)).ok();
    checkpoint.release();
    write.join().unwrap();
    let value =
        premature.unwrap_or_else(|| rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap());
    read.join().unwrap();
    assert_eq!(value.unwrap(), Some(b"stable".to_vec()));
    assert_eq!(db.get(b"other").unwrap(), Some(b"final".to_vec()));
}

#[test]
fn owner_lock_prevents_second_open_and_snapshot_reclamation() {
    let dir = TempDir::new("ownership");
    let mut db = Kiban::open(dir.path()).unwrap();
    db.put(b"key", b"value").unwrap();
    db.checkpoint().unwrap();
    assert!(matches!(Kiban::open(dir.path()), Err(DbError::Locked)));
    let snapshot = db.snapshot();
    let clone = snapshot.clone();
    assert!(StdArc::ptr_eq(&snapshot.memtable, &clone.memtable));
    drop(db);
    assert!(matches!(Kiban::open(dir.path()), Err(DbError::Locked)));
    drop(snapshot);
    assert!(matches!(Kiban::open(dir.path()), Err(DbError::Locked)));
    drop(clone);
    assert_eq!(
        Kiban::open(dir.path()).unwrap().get(b"key").unwrap(),
        Some(b"value".to_vec())
    );
}

#[test]
fn maximum_sequence_is_preserved_by_compaction_without_a_sentinel_collision() {
    let dir = TempDir::new("maximum-sequence");
    let mut db =
        Kiban::open_with_options(dir.path(), super::compaction_tests::tiny_options()).unwrap();
    db.last_sequence = u64::MAX - 3;
    db.put(b"key", b"old").unwrap();
    db.flush().unwrap();
    db.last_sequence = u64::MAX - 1;
    db.put(b"key", b"new").unwrap();
    db.checkpoint().unwrap();
    assert_eq!(db.get(b"key").unwrap(), Some(b"new".to_vec()));
    assert!(matches!(
        db.put(b"key", b"later"),
        Err(DbError::SequenceExhausted)
    ));
    drop(db);
    assert_eq!(
        Kiban::open(dir.path()).unwrap().get(b"key").unwrap(),
        Some(b"new".to_vec())
    );
}

#[test]
fn pinned_snapshot_preserves_old_value_without_retaining_intermediate_sst_versions() {
    let dir = TempDir::new("snapshot-gc");
    let mut db =
        Kiban::open_with_options(dir.path(), super::compaction_tests::tiny_options()).unwrap();
    db.put(b"key", b"original").unwrap();
    db.checkpoint().unwrap();
    let snapshot = db.snapshot();
    for _ in 0..100 {
        db.put(b"key", [b'v'; 64]).unwrap();
        db.flush().unwrap();
    }
    let versions: usize = db
        .version
        .tables
        .iter()
        .map(|t| t.table.iter_from(b"").count())
        .sum();
    assert!(versions <= 2, "retained {versions} intermediate versions");
    assert_eq!(
        db.get_at(&snapshot, b"key").unwrap(),
        Some(b"original".to_vec())
    );
    assert_eq!(db.get(b"key").unwrap(), Some(vec![b'v'; 64]));
}

#[test]
fn direct_snapshot_release_is_identity_aware_and_drop_is_automatic() {
    let dir = TempDir::new("snapshot-identity");
    let mut db = Kiban::open(dir.path()).unwrap();
    let a = db.snapshot();
    let clone = a.clone();
    let b = db.snapshot(); // same sequence, independent identity
    db.release_snapshot(&a);
    db.release_snapshot(&clone);
    assert_eq!(db.active_snapshots.len(), 1);
    drop(b);
    assert_eq!(db.active_snapshots.len(), 0);
    let a = db.snapshot();
    let clone = a.clone();
    drop(a);
    assert_eq!(db.active_snapshots.len(), 1);
    drop(clone);
    assert_eq!(db.active_snapshots.len(), 0);
}

#[test]
fn direct_snapshot_reads_reject_another_engine_owner() {
    let a = TempDir::new("snapshot-owner-a");
    let b = TempDir::new("snapshot-owner-b");
    let mut a = Kiban::open(a.path()).unwrap();
    let mut b = Kiban::open(b.path()).unwrap();
    a.put(b"key", b"a").unwrap();
    b.put(b"key", b"b").unwrap();
    let snapshot = a.snapshot();
    assert!(matches!(
        b.get_at(&snapshot, b"key"),
        Err(DbError::InvalidArgument(_))
    ));
    assert!(matches!(
        b.scan_at(&snapshot),
        Err(DbError::InvalidArgument(_))
    ));
    b.release_snapshot(&snapshot);
    assert_eq!(a.active_snapshots.len(), 1);
    assert_eq!(a.get_at(&snapshot, b"key").unwrap(), Some(b"a".to_vec()));
}

#[test]
fn hot_updates_remain_bounded_and_copied_snapshots_preserve_values() {
    let dir = TempDir::new("bounded-versions");
    let mut db = Kiban::open(dir.path()).unwrap();
    db.put(b"key", b"before").unwrap();
    let snapshot = db.snapshot();
    for _ in 0..10_000 {
        db.put(b"key", b"after").unwrap();
    }
    assert_eq!(db.memtable.iter_all_versions().count(), 1);
    assert_eq!(
        db.shared_memtable
            .read()
            .unwrap()
            .iter_all_versions()
            .count(),
        1
    );
    db.checkpoint().unwrap();
    assert_eq!(
        db.get_at(&snapshot, b"key").unwrap(),
        Some(b"before".to_vec())
    );
}

#[test]
fn shared_hot_updates_flush_by_write_volume_even_with_one_resident_key() {
    let dir = TempDir::new("hot-wal-bound");
    let db = SharedKiban::open_with_options(
        dir.path(),
        KibanOptions {
            write_buffer_bytes: 16 * 1024,
            ..KibanOptions::default()
        },
    )
    .unwrap();
    for _ in 0..3_000 {
        db.put(b"key", [b'v'; 64]).unwrap();
    }
    db.checkpoint().unwrap();
    let stats = db.stats().unwrap();
    assert!(stats.maintenance.flushes_completed > 1);
    assert_eq!(db.get(b"key").unwrap(), Some(vec![b'v'; 64]));
}

#[test]
fn invalid_sizes_and_exhausted_sequences_do_not_poison_or_wrap() {
    let dir = TempDir::new("invalid-input");
    let mut db = Kiban::open(dir.path()).unwrap();
    let key = vec![0; crate::limits::MAX_KEY_BYTES + 1];
    assert!(matches!(
        db.put(&key, b"v"),
        Err(DbError::InvalidArgument(_))
    ));
    assert!(!db.is_poisoned());
    assert_eq!(db.last_sequence, 0);
    db.last_sequence = u64::MAX;
    assert!(matches!(
        db.put(b"k", b"v"),
        Err(DbError::SequenceExhausted)
    ));
    let mut batch = WriteBatch::new();
    batch.put(b"k".to_vec(), b"v".to_vec());
    assert!(matches!(db.write(batch), Err(DbError::SequenceExhausted)));
    assert!(db.memtable.is_empty());
}

#[test]
fn recovered_level_overlap_cannot_hide_behind_another_level() {
    let dir = TempDir::new("hidden-overlap");
    for (number, keys) in [(2, [b"a", b"z"]), (3, [b"b", b"c"]), (4, [b"m", b"n"])] {
        let mut builder = TableBuilder::new();
        for key in keys {
            builder.add(Kind::Put, key, b"v", 1).unwrap();
        }
        atomic::commit_file(
            &dir.path().join(file_name(number, SST_EXTENSION)),
            &builder.finish().unwrap(),
        )
        .unwrap();
    }
    atomic::create_durably(&dir.path().join("1.wal")).unwrap();
    Manifest {
        next_file_number: 5,
        wal_numbers: vec![1],
        last_sequence: 1,
        tables: vec![
            TableRef {
                level: 1,
                number: 2,
            },
            TableRef {
                level: 1,
                number: 4,
            },
            TableRef {
                level: 2,
                number: 3,
            },
        ],
    }
    .install(dir.path())
    .unwrap();
    assert!(matches!(Kiban::open(dir.path()), Err(DbError::Corrupt(_))));
}

#[test]
fn streaming_snapshot_range_and_backup_match_captured_state() {
    let dir = TempDir::new("snapshot-backup");
    let db = SharedKiban::open(dir.path()).unwrap();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    let snapshot = db.snapshot().unwrap();
    db.delete(b"a").unwrap();
    db.put(b"c", b"3").unwrap();
    let ranged: Vec<_> = snapshot
        .range(b"a", b"b")
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ranged, vec![(b"a".to_vec(), b"1".to_vec())]);
    let destination = dir.path().join("backup");
    snapshot.backup(&destination).unwrap();
    let backup = Kiban::open(&destination).unwrap();
    assert_eq!(
        backup.iter().collect::<Result<Vec<_>, _>>().unwrap(),
        snapshot.scan().unwrap()
    );
    assert!(snapshot.backup(&destination).is_err());
    drop(backup);
    drop(snapshot);
    db.close().unwrap();
    assert!(Kiban::open(dir.path()).is_ok());
}

#[test]
fn failed_obsolete_unlink_is_retained_for_retry() {
    let dir = TempDir::new("reclaim-retry");
    let mut db =
        Kiban::open_with_options(dir.path(), super::compaction_tests::tiny_options()).unwrap();
    db.put(b"key", b"old").unwrap();
    db.flush().unwrap();
    let snapshot = db.snapshot();
    db.put(b"key", b"new").unwrap();
    db.flush().unwrap();
    assert!(!db.obsolete.is_empty());
    drop(snapshot);
    crate::sys::install_fault(0);
    db.reclaim_obsolete();
    crate::sys::clear_fault();
    assert!(!db.obsolete.is_empty());
    db.reclaim_obsolete();
    assert!(db.obsolete.is_empty());
}
