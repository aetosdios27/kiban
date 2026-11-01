//! Additional group-commit contract tests. No production hooks are added.
use super::*;
use std::sync::mpsc;

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for test rendezvous"
        );
        thread::yield_now();
    }
}

fn apply_op(op: &DurableOp, model: &mut Map) {
    match op {
        DurableOp::Put(k, v) => {
            model.insert(k.clone(), v.clone());
        }
        DurableOp::Delete(k) => {
            model.remove(k);
        }
        DurableOp::Batch(b) => {
            for (kind, k, v) in &b.ops {
                match kind {
                    Kind::Put => {
                        model.insert(k.clone(), v.clone());
                    }
                    Kind::Tombstone => {
                        model.remove(k);
                    }
                }
            }
        }
    }
}

#[test]
fn qa_live_wal_generations_sync_before_ack_and_survive_rotation() {
    let dir = TempDir::new("gc-qa-generations");
    let _device = DeviceGuard::new();
    let db = SharedKiban::open_manual(dir.path(), tiny()).unwrap();
    let mut expected = Map::new();
    for generation in 0..6 {
        let old = format!("old/{generation}").into_bytes();
        let new = format!("new/{generation}").into_bytes();
        db.put(&old, b"buffered-old").unwrap();
        expected.insert(old, b"buffered-old".to_vec());
        db.write_lock().unwrap().freeze().unwrap();
        assert!(db.write_lock().unwrap().immutable.is_some());
        db.put(&new, b"buffered-new").unwrap();
        expected.insert(new, b"buffered-new".to_vec());
        let key = format!("durable/{generation}").into_bytes();
        db.put_durable(&key, b"ack").unwrap();
        expected.insert(key, b"ack".to_vec());
        // Capture at the acknowledgement, before any later flush can repair
        // a missing sync. Recover on a separate device clone, then return.
        let live = sys::snapshot_device();
        sys::power_loss();
        let recovered = Kiban::open_with_options(dir.path(), tiny()).unwrap();
        assert_eq!(
            recovered.iter().collect::<Result<Map, _>>().unwrap(),
            expected
        );
        drop(recovered);
        sys::restore_device(live);
        db.maintenance_for_test().wake();
        assert!(
            db.step(),
            "drain the immutable generation before rotating again"
        );
    }
}

#[test]
fn qa_failed_groups_refuse_all_retries_and_recover_exact_synced_prefix() {
    // Small frames fault in flush/fdatasync; large frames also fault inside
    // append. Every failed operation has a distinct key to expose leaked tails.
    let mut phases = [false; 2];
    for size in [16, 32768] {
        for fault in 0..8 {
            let dir = TempDir::new("gc-qa-prefix");
            let _device = DeviceGuard::new();
            let mut options = tiny();
            options.write_buffer_bytes = 1 << 24;
            let db = SharedKiban::open_manual(dir.path(), options.clone()).unwrap();
            db.put_durable(b"base", b"synced").unwrap();
            let entered = gather_hook(&db, 3, move |group, stage, _| {
                if group == 1 && stage == GroupStage::BeforeAppend {
                    sys::install_fault(fault);
                }
            });
            let device = sys::device_handle();
            let results = thread::scope(|scope| {
                let a = scope.spawn(|| {
                    device.clone().adopt();
                    db.put_durable(b"leader", b"synced")
                });
                until(|| entered.load(Ordering::SeqCst));
                let rest: Vec<_> = (0..3)
                    .map(|i| {
                        let (db, device) = (&db, device.clone());
                        scope.spawn(move || {
                            device.adopt();
                            let mut batch = WriteBatch::new();
                            batch.put(format!("failed/{i}/a"), vec![i; size]);
                            batch.delete(b"base");
                            batch.put(format!("failed/{i}/b"), b"tail");
                            let out = db.write_durable(batch);
                            sys::clear_fault();
                            out
                        })
                    })
                    .collect();
                a.join().unwrap().unwrap();
                rest.into_iter()
                    .map(|h| h.join().unwrap())
                    .collect::<Vec<_>>()
            });
            db.set_group_hook(None);
            let failed = results[0].is_err();
            assert!(
                results.iter().all(|r| r.is_err() == failed),
                "whole group: {results:?}"
            );
            let expected: Map;
            if failed {
                let phase = match &results[0] {
                    Err(DbError::Poisoned(PoisonCause::WalAppendFailed(_))) => 0,
                    Err(DbError::Poisoned(PoisonCause::WalSyncFailed(_))) => 1,
                    other => panic!("unexpected failure: {other:?}"),
                };
                phases[phase] = true;
                assert!(db.is_poisoned());
                expected = [
                    (b"base".to_vec(), b"synced".to_vec()),
                    (b"leader".to_vec(), b"synced".to_vec()),
                ]
                .into();
                assert_eq!(
                    db.snapshot()
                        .unwrap()
                        .iter()
                        .collect::<Result<Map, _>>()
                        .unwrap(),
                    expected
                );
                // Retrying on the poisoned handle must never repair the tail
                // accidentally, including through the non-durable API.
                let mut batch = WriteBatch::new();
                batch.put(b"retry", b"v");
                for out in [
                    db.put(b"retry", b"v"),
                    db.delete(b"base"),
                    db.write(WriteBatch {
                        ops: batch.ops.clone(),
                    }),
                    db.put_durable(b"retry", b"v"),
                    db.delete_durable(b"base"),
                    db.write_durable(batch),
                    db.sync(),
                ] {
                    assert!(matches!(out, Err(DbError::Poisoned(_))), "retry: {out:?}");
                }
                assert_eq!(db.write_lock().unwrap().last_sequence_for_test(), 2);
            } else {
                expected = db
                    .snapshot()
                    .unwrap()
                    .iter()
                    .collect::<Result<Map, _>>()
                    .unwrap();
            }
            // Snapshot BEFORE dropping the writer: BufWriter::drop must not
            // change what this strict power-loss test regards as persisted.
            let image = sys::snapshot_device();
            drop(db);
            sys::restore_device(image);
            sys::power_loss();
            let mut reopened = Kiban::open_with_options(dir.path(), options).unwrap();
            assert_eq!(
                reopened.iter().collect::<Result<Map, _>>().unwrap(),
                expected,
                "size={size} fault={fault}"
            );
            // Reopen is the boundary at which a retry becomes legal again.
            reopened.put(b"retry", b"v").unwrap();
            reopened.sync().unwrap();
            assert_eq!(reopened.get(b"retry").unwrap(), Some(b"v".to_vec()));
        }
    }
    assert_eq!(
        phases,
        [true, true],
        "must exercise append/flush and fdatasync failures"
    );
}

#[test]
fn qa_mixed_batches_tombstones_and_groups_match_wal_order_and_snapshots() {
    for seed in 0..32 {
        let dir = TempDir::new("gc-qa-order");
        let _device = DeviceGuard::new();
        let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
        let model = StdArc::new(StdMutex::new(Map::new()));
        let observed = model.clone();
        let entered = gather_hook(&db, 4, move |_, stage, ops| {
            if stage == GroupStage::AfterAppend {
                let mut m = observed.lock().unwrap();
                for op in ops {
                    apply_op(op, &mut m);
                }
            }
        });
        let device = sys::device_handle();
        let old = db.snapshot().unwrap();
        thread::scope(|scope| {
            scope.spawn(|| {
                device.clone().adopt();
                db.put_durable(b"same", b"first").unwrap();
            });
            until(|| entered.load(Ordering::SeqCst));
            for writer in 0..4u64 {
                let (db, device) = (&db, device.clone());
                scope.spawn(move || {
                    device.adopt();
                    let mut rng = sys::SplitMix64(seed * 71 + writer);
                    for round in 0..12 {
                        match rng.below(3) {
                            0 => db.delete_durable(b"same").unwrap(),
                            1 => db
                                .put_durable(b"same", format!("{writer}/{round}"))
                                .unwrap(),
                            _ => {
                                let mut batch = WriteBatch::new();
                                batch.put(b"same", b"overwritten").delete(b"same");
                                if rng.below(2) == 0 {
                                    batch.put(b"same", format!("b{writer}/{round}"));
                                }
                                batch.put(format!("marker/{writer}/{round}"), b"v");
                                db.write_durable(batch).unwrap();
                            }
                        }
                    }
                });
            }
        });
        db.set_group_hook(None);
        let expected = model.lock().unwrap().clone();
        assert_eq!(
            db.snapshot()
                .unwrap()
                .iter()
                .collect::<Result<Map, _>>()
                .unwrap(),
            expected,
            "seed={seed}"
        );
        assert_eq!(
            db.get(b"same").unwrap().as_ref(),
            expected.get(b"same".as_slice())
        );
        assert_eq!(old.get(b"same").unwrap(), None);
        drop(old);
        let image = sys::snapshot_device();
        drop(db);
        sys::restore_device(image);
        sys::power_loss();
        let recovered = Kiban::open_with_options(dir.path(), tiny()).unwrap();
        assert_eq!(
            recovered.iter().collect::<Result<Map, _>>().unwrap(),
            expected,
            "seed={seed}"
        );
    }
}

#[test]
fn qa_panic_boundaries_complete_followers_and_recover_synced_records() {
    for stage in [
        GroupStage::BeforeAppend,
        GroupStage::AfterAppend,
        GroupStage::AfterSync,
        GroupStage::Published,
    ] {
        let dir = TempDir::new("gc-qa-panic");
        let _device = DeviceGuard::new();
        let db = SharedKiban::open_manual(dir.path(), tiny()).unwrap();
        let entered = gather_hook(&db, 2, move |group, at, _| {
            if group == 1 && at == stage {
                panic!("injected leader panic at {stage:?}");
            }
        });
        let device = sys::device_handle();
        let (send, recv) = mpsc::channel();
        thread::scope(|scope| {
            let a = scope.spawn(|| {
                device.clone().adopt();
                db.put_durable(b"base", b"v")
            });
            until(|| entered.load(Ordering::SeqCst));
            for key in [b"b", b"c"] {
                let (db, device, send) = (&db, device.clone(), send.clone());
                scope.spawn(move || {
                    device.adopt();
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        db.put_durable(key, b"v")
                    }));
                    send.send(outcome).unwrap();
                });
            }
            a.join().unwrap().unwrap();
            let outcomes: Vec<_> = (0..2)
                .map(|_| recv.recv_timeout(Duration::from_secs(10)).unwrap())
                .collect();
            assert_eq!(
                outcomes.iter().filter(|r| r.is_err()).count(),
                1,
                "one leader panics"
            );
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|r| matches!(r, Ok(Err(DbError::Corrupt(_)))))
                    .count(),
                1,
                "follower completes with error"
            );
        });
        assert_eq!(db.group.queued(), 0);
        assert!(
            db.put_durable(b"retry", b"v").is_err(),
            "poisoned engine gate rejects retry"
        );
        assert_eq!(
            db.get(b"b").unwrap().is_some(),
            stage == GroupStage::Published
        );
        let image = sys::snapshot_device();
        drop(db);
        sys::restore_device(image);
        sys::power_loss();
        let reopened = Kiban::open_with_options(dir.path(), tiny()).unwrap();
        assert_eq!(reopened.get(b"base").unwrap(), Some(b"v".to_vec()));
        for key in [b"b", b"c"] {
            assert_eq!(
                reopened.get(key).unwrap().is_some(),
                matches!(stage, GroupStage::AfterSync | GroupStage::Published)
            );
        }
    }
}

#[test]
fn qa_grouped_batches_recover_atomically_at_each_crash_boundary() {
    for stage in [
        GroupStage::BeforeAppend,
        GroupStage::AfterAppend,
        GroupStage::AfterSync,
        GroupStage::Applied(0),
        GroupStage::Published,
    ] {
        let dir = TempDir::new("gc-qa-batch-crash");
        let _device = DeviceGuard::new();
        let db = SharedKiban::open_manual(dir.path(), tiny()).unwrap();
        let captured = StdArc::new(StdMutex::new(None));
        let expected = StdArc::new(StdMutex::new(Map::new()));
        let (image, model) = (captured.clone(), expected.clone());
        let entered = gather_hook(&db, 3, move |group, at, ops| {
            if at == GroupStage::BeforeAppend {
                let mut m = model.lock().unwrap();
                for op in ops {
                    apply_op(op, &mut m);
                }
            }
            if group == 1 && at == stage {
                *image.lock().unwrap() = Some(sys::snapshot_device());
            }
        });
        let device = sys::device_handle();
        thread::scope(|scope| {
            scope.spawn(|| {
                device.clone().adopt();
                db.put_durable(b"base", b"v").unwrap();
            });
            until(|| entered.load(Ordering::SeqCst));
            for writer in 0..3 {
                let (db, device) = (&db, device.clone());
                scope.spawn(move || {
                    device.adopt();
                    let mut b = WriteBatch::new();
                    b.put(format!("batch/{writer}/a"), b"first");
                    b.delete(b"base").put(b"same", b"discard").delete(b"same");
                    b.put(b"same", format!("winner/{writer}"));
                    b.put(format!("batch/{writer}/b"), b"last");
                    db.write_durable(b).unwrap();
                });
            }
        });
        db.set_group_hook(None);
        drop(db);
        sys::restore_device(
            captured
                .lock()
                .unwrap()
                .take()
                .expect("target group reached"),
        );
        sys::power_loss();
        let reopened = Kiban::open_with_options(dir.path(), tiny()).unwrap();
        let actual = reopened.iter().collect::<Result<Map, _>>().unwrap();
        let expected = if matches!(stage, GroupStage::BeforeAppend | GroupStage::AfterAppend) {
            [(b"base".to_vec(), b"v".to_vec())].into()
        } else {
            expected.lock().unwrap().clone()
        };
        assert_eq!(actual, expected, "crash at {stage:?}");
    }
}

#[test]
fn qa_stalled_group_wakes_on_progress_or_sticky_failure_and_reads_continue() {
    for fail in [false, true] {
        let dir = TempDir::new("gc-qa-stall");
        let backup = TempDir::new("gc-qa-stalled-backup");
        let mut options = tiny();
        options.l0_write_stall_trigger = 4;
        let db = SharedKiban::open_with_options(dir.path(), options).unwrap();
        db.maintenance_for_test().arm_before_build();
        for i in 0..4 {
            db.put(format!("seed/{i}"), b"v").unwrap();
            db.sync().unwrap();
            db.flush_sync_for_test().unwrap();
        }
        db.maintenance_for_test().wait_before_build_reached();
        assert_eq!(db.write_lock().unwrap().l0_count(), 4);
        thread::scope(|scope| {
            struct Release<'a>(&'a Maintenance);
            impl Drop for Release<'_> {
                fn drop(&mut self) {
                    self.0.release_before_build();
                }
            }
            let release = Release(db.maintenance_for_test());
            let first = scope.spawn(|| db.put_durable(b"writer/0", b"v"));
            until(|| db.maintenance_for_test().waiting_writers() == 1);
            let writers: Vec<_> = (1..4)
                .map(|i| {
                    let db = &db;
                    scope.spawn(move || db.put_durable(format!("writer/{i}"), b"v"))
                })
                .collect();
            until(|| db.maintenance_for_test().waiting_writers() == 1 && db.group.queued() == 3);
            assert_eq!(db.get(b"seed/0").unwrap(), Some(b"v".to_vec()));
            let snap = db.snapshot().unwrap();
            assert_eq!(snap.get(b"writer/0").unwrap(), None);
            db.backup(backup.path().join("copy")).unwrap();
            let copy = Kiban::open(backup.path().join("copy")).unwrap();
            assert_eq!(copy.get(b"writer/0").unwrap(), None);
            assert_eq!(copy.get(b"seed/0").unwrap(), Some(b"v".to_vec()));
            if fail {
                db.maintenance_for_test()
                    .fail("qa stalled worker failure".into());
            }
            drop(release);
            for h in std::iter::once(first).chain(writers) {
                let result = h.join().unwrap();
                if fail {
                    assert!(matches!(result, Err(DbError::Maintenance(_))));
                } else {
                    result.unwrap();
                }
            }
            assert_eq!(
                snap.get(b"writer/0").unwrap(),
                None,
                "old snapshot stays pinned"
            );
        });
        assert_eq!(db.group.queued(), 0);
        assert_eq!(db.maintenance_for_test().waiting_writers(), 0);
        if fail {
            let first = db.maintenance_error().unwrap().to_string();
            db.maintenance_for_test().fail("second failure".into());
            assert_eq!(db.maintenance_error().unwrap().to_string(), first);
            assert!(matches!(
                db.put(b"retry", b"v"),
                Err(DbError::Maintenance(_))
            ));
        } else {
            db.put_durable(b"retry", b"v").unwrap();
        }
    }
}

fn assert_pairs(snapshot: &SharedSnapshot) {
    for writer in 0..3 {
        assert_eq!(
            snapshot.get(format!("pair/{writer}/a")).unwrap(),
            snapshot.get(format!("pair/{writer}/b")).unwrap(),
            "torn pair for writer {writer}"
        );
    }
}

#[test]
fn qa_checkpoint_close_backup_and_snapshots_with_grouped_batches() {
    let dir = TempDir::new("gc-qa-protocols");
    let copies = TempDir::new("gc-qa-protocol-copies");
    let mut options = tiny();
    options.write_buffer_bytes = 1000;
    options.l0_write_stall_trigger = 4;
    let db = SharedKiban::open_with_options(dir.path(), options.clone()).unwrap();
    let start = Barrier::new(4);
    let remaining = AtomicUsize::new(3);
    thread::scope(|scope| {
        for writer in 0..3 {
            let (db, start, remaining) = (&db, &start, &remaining);
            scope.spawn(move || {
                start.wait();
                for round in 0..60 {
                    let mut batch = WriteBatch::new();
                    for suffix in ["a", "b"] {
                        let key = format!("pair/{writer}/{suffix}");
                        if round % 5 == 0 {
                            batch.delete(key);
                        } else {
                            batch.put(key, format!("value/{round}"));
                        }
                    }
                    db.write_durable(batch).unwrap();
                }
                remaining.fetch_sub(1, Ordering::SeqCst);
            });
        }
        scope.spawn(|| {
            start.wait();
            for i in 0..8 {
                db.checkpoint().unwrap();
                db.clone().close().unwrap();
                let path = copies.path().join(format!("copy-{i}"));
                db.backup(&path).unwrap();
                let copy = Kiban::open_with_options(&path, options.clone()).unwrap();
                for writer in 0..3 {
                    assert_eq!(
                        copy.get(format!("pair/{writer}/a")).unwrap(),
                        copy.get(format!("pair/{writer}/b")).unwrap()
                    );
                }
                // Each export must be independently durable too.
                drop(copy);
                assert_pairs(&db.snapshot().unwrap());
            }
        });
        let mut samples = 0;
        while samples < 40 || remaining.load(Ordering::SeqCst) != 0 {
            assert_pairs(&db.snapshot().unwrap());
            samples += 1;
        }
    });
    db.clone().close().unwrap();
    db.put_durable(b"after-close", b"remaining clone works")
        .unwrap();
    drop(db);
    let reopened = Kiban::open_with_options(dir.path(), options).unwrap();
    for writer in 0..3 {
        assert_eq!(
            reopened.get(format!("pair/{writer}/a")).unwrap(),
            Some(b"value/59".to_vec())
        );
        assert_eq!(
            reopened.get(format!("pair/{writer}/b")).unwrap(),
            Some(b"value/59".to_vec())
        );
    }
    assert!(reopened.get(b"after-close").unwrap().is_some());
}

#[test]
fn qa_readers_and_snapshots_wait_for_whole_group_publication() {
    let dir = TempDir::new("gc-qa-reader-gate");
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let old = db.snapshot().unwrap();
    let (paused_send, paused_recv) = mpsc::channel();
    let (release_send, release_recv) = mpsc::channel();
    let release_recv = StdMutex::new(release_recv);
    let entered = gather_hook(&db, 2, move |group, stage, ops| {
        if group == 1 && stage == GroupStage::Applied(0) {
            assert_eq!(ops.len(), 2);
            paused_send.send(()).unwrap();
            release_recv
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
    });
    thread::scope(|scope| {
        let first = scope.spawn(|| db.put_durable(b"base", b"v"));
        until(|| entered.load(Ordering::SeqCst));
        let writers: Vec<_> = (0..2)
            .map(|writer| {
                let db = &db;
                scope.spawn(move || {
                    let mut batch = WriteBatch::new();
                    batch.put(format!("pair/{writer}/a"), b"v");
                    batch.put(format!("pair/{writer}/b"), b"v");
                    db.write_durable(batch)
                })
            })
            .collect();
        paused_recv.recv_timeout(Duration::from_secs(10)).unwrap();
        let (ready_send, ready_recv) = mpsc::channel();
        let (read_send, read_recv) = mpsc::channel();
        let ready = ready_send.clone();
        let reader_db = &db;
        let direct = scope.spawn(move || {
            ready.send(()).unwrap();
            read_send.send(reader_db.get(b"pair/1/b")).unwrap();
        });
        let (snap_send, snap_recv) = mpsc::channel();
        let snapshot_db = &db;
        let snapshot = scope.spawn(move || {
            ready_send.send(()).unwrap();
            snap_send.send(snapshot_db.snapshot()).unwrap();
        });
        for _ in 0..2 {
            ready_recv.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        let read_blocked = matches!(
            read_recv.recv_timeout(Duration::from_millis(30)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        let snapshot_blocked = matches!(
            snap_recv.recv_timeout(Duration::from_millis(30)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        // Release before asserting so a failed assertion cannot strand writers.
        release_send.send(()).unwrap();
        assert!(
            read_blocked,
            "point reader escaped the mirror guard mid-group"
        );
        assert!(
            snapshot_blocked,
            "snapshot escaped the engine guard mid-group"
        );
        assert_eq!(
            read_recv
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap(),
            Some(b"v".to_vec())
        );
        let snap = snap_recv
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        for writer in 0..2 {
            assert_eq!(
                snap.get(format!("pair/{writer}/a")).unwrap(),
                Some(b"v".to_vec())
            );
            assert_eq!(
                snap.get(format!("pair/{writer}/b")).unwrap(),
                Some(b"v".to_vec())
            );
        }
        direct.join().unwrap();
        snapshot.join().unwrap();
        first.join().unwrap().unwrap();
        for h in writers {
            h.join().unwrap().unwrap();
        }
    });
    db.set_group_hook(None);
    assert_eq!(old.get(b"base").unwrap(), None);
}

#[test]
fn qa_invalid_batch_is_rejected_before_queue_and_corrected_retry_succeeds() {
    let dir = TempDir::new("gc-qa-invalid-batch");
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(b"valid-prefix", b"must-not-appear");
    batch.delete(vec![0; crate::limits::MAX_KEY_BYTES + 1]);
    assert!(matches!(
        db.write_durable(batch),
        Err(DbError::InvalidArgument(_))
    ));
    assert_eq!(db.get(b"valid-prefix").unwrap(), None);
    assert_eq!(db.group.queued(), 0);
    assert_eq!(db.write_lock().unwrap().last_sequence_for_test(), 0);
    assert!(!db.is_poisoned());
    let mut retry = WriteBatch::new();
    retry.put(b"valid-prefix", b"corrected");
    db.write_durable(retry).unwrap();
    assert_eq!(
        db.get(b"valid-prefix").unwrap(),
        Some(b"corrected".to_vec())
    );
}

#[test]
fn qa_sequence_exhaustion_rejects_whole_batch_and_preserves_other_group_members() {
    let dir = TempDir::new("gc-qa-exhaustion");
    let _device = DeviceGuard::new();
    let db = SharedKiban::open_manual(dir.path(), tiny()).unwrap();
    let mut too_large = WriteBatch::new();
    too_large
        .put(b"invalid/a", b"v")
        .put(b"invalid/b", b"v")
        .delete(b"last");
    let ops = [
        DurableOp::Put(b"last".to_vec(), b"v".to_vec()),
        DurableOp::Batch(too_large),
        DurableOp::Delete(b"last".to_vec()),
        DurableOp::Batch(WriteBatch::new()),
        DurableOp::Put(b"overflow".to_vec(), b"v".to_vec()),
    ];
    {
        let mut guard = db.write_lock().unwrap();
        guard.last_sequence = u64::MAX - 2;
        let outcomes = guard.commit_group(&ops).unwrap();
        assert!(outcomes[0].is_ok());
        assert!(matches!(outcomes[1], Err(DbError::SequenceExhausted)));
        assert!(outcomes[2].is_ok());
        assert!(outcomes[3].is_ok());
        assert!(matches!(outcomes[4], Err(DbError::SequenceExhausted)));
        assert_eq!(guard.last_sequence_for_test(), u64::MAX);
    }
    assert!(db.snapshot().unwrap().iter().next().is_none());
    assert!(!db.is_poisoned());
    assert!(matches!(
        db.put_durable(b"retry", b"v"),
        Err(DbError::SequenceExhausted)
    ));
    db.write_durable(WriteBatch::new()).unwrap();
    let image = sys::snapshot_device();
    drop(db);
    sys::restore_device(image);
    sys::power_loss();
    let reopened = Kiban::open_with_options(dir.path(), tiny()).unwrap();
    assert_eq!(reopened.last_sequence_for_test(), u64::MAX);
    assert!(reopened.iter().next().is_none());
}
