//! Durable write groups: grouping, wakeup, error propagation, visibility and
//! concurrent crash recovery on the simulated device.

use super::*;
use crate::sys;
use crate::testutil::TempDir;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex as StdMutex};
use std::thread;
use std::time::{Duration, Instant};

type Map = BTreeMap<Vec<u8>, Vec<u8>>;

struct DeviceGuard;

impl DeviceGuard {
    fn new() -> Self {
        sys::enable_device_sim();
        DeviceGuard
    }
}

impl Drop for DeviceGuard {
    fn drop(&mut self) {
        sys::clear_fault();
        sys::disable_device_sim();
    }
}

fn wait_for_queue(
    queue: &GroupQueue<DurableOp, Result<(), DbError>>,
    wanted: usize,
    limit: Duration,
) {
    let deadline = Instant::now() + limit;
    while queue.queued() < wanted && Instant::now() < deadline {
        thread::yield_now();
    }
}

/// Group 0 (the first writer, alone) pauses until `followers` others queue,
/// so group 1 deterministically gathers them. `on_group` sees each later
/// group's `BeforeAppend` with its index, starting at 1.
fn gather_hook(
    db: &SharedKiban,
    followers: usize,
    on_group: impl Fn(usize, GroupStage, &[DurableOp]) + Send + Sync + 'static,
) -> StdArc<std::sync::atomic::AtomicBool> {
    let queue = db.group.clone();
    let seen = AtomicUsize::new(usize::MAX);
    let entered = StdArc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = entered.clone();
    db.set_group_hook(Some(StdArc::new(move |stage, ops| {
        if stage == GroupStage::BeforeAppend {
            let group = seen.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
            if group == 0 {
                signal.store(true, Ordering::SeqCst);
                wait_for_queue(&queue, followers, Duration::from_secs(30));
                assert!(queue.queued() >= followers, "followers never queued");
            }
        }
        let group = seen.load(Ordering::SeqCst);
        on_group(group, stage, ops);
    })));
    entered
}

fn tiny() -> KibanOptions {
    super::compaction_tests::tiny_options()
}

#[test]
fn durable_writes_roundtrip_and_survive_power_loss_without_sync() {
    let dir = TempDir::new("gc-roundtrip");
    let _device = DeviceGuard::new();
    {
        let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
        db.put_durable(b"a", b"1").unwrap();
        db.put_durable(b"b", b"2").unwrap();
        db.delete_durable(b"a").unwrap();
        let mut batch = WriteBatch::new();
        batch.put(b"c", b"3").put(b"d", b"4").delete(b"b");
        db.write_durable(batch).unwrap();
        db.write_durable(WriteBatch::new()).unwrap();
        assert_eq!(db.get(b"a").unwrap(), None);
        assert_eq!(db.get(b"c").unwrap(), Some(b"3".to_vec()));
        // A plain put after the last durable write is not covered.
        db.put(b"volatile", b"x").unwrap();
        sys::power_loss();
    }
    sys::power_loss();
    let db = Kiban::open_with_options(dir.path(), tiny()).unwrap();
    assert_eq!(db.get(b"a").unwrap(), None);
    assert_eq!(db.get(b"b").unwrap(), None);
    assert_eq!(db.get(b"c").unwrap(), Some(b"3".to_vec()));
    assert_eq!(db.get(b"d").unwrap(), Some(b"4".to_vec()));
    assert_eq!(db.get(b"volatile").unwrap(), None);
}

#[test]
fn durable_write_also_syncs_earlier_buffered_writes() {
    let dir = TempDir::new("gc-covers-buffered");
    let _device = DeviceGuard::new();
    {
        let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
        db.put(b"earlier", b"buffered").unwrap();
        db.put_durable(b"later", b"durable").unwrap();
        sys::power_loss();
    }
    sys::power_loss();
    let db = Kiban::open_with_options(dir.path(), tiny()).unwrap();
    assert_eq!(db.get(b"earlier").unwrap(), Some(b"buffered".to_vec()));
    assert_eq!(db.get(b"later").unwrap(), Some(b"durable".to_vec()));
}

#[test]
fn invalid_input_is_rejected_before_queueing_without_poisoning() {
    let dir = TempDir::new("gc-invalid");
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let huge = vec![0u8; crate::limits::MAX_KEY_BYTES + 1];
    assert!(matches!(
        db.put_durable(&huge, b"v"),
        Err(DbError::InvalidArgument(_))
    ));
    assert!(matches!(
        db.delete_durable(&huge),
        Err(DbError::InvalidArgument(_))
    ));
    assert!(!db.is_poisoned());
    assert_eq!(db.group.queued(), 0);
    db.put_durable(b"ok", b"v").unwrap();
}

#[test]
fn concurrent_writers_share_one_sync_and_all_wake_with_their_own_result() {
    const WRITERS: usize = 8;
    let dir = TempDir::new("gc-share");
    let _device = DeviceGuard::new();
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let group_sizes = StdArc::new(StdMutex::new(Vec::new()));
    let syncs = StdArc::new(AtomicUsize::new(0));
    let entered = {
        let (sizes, syncs) = (group_sizes.clone(), syncs.clone());
        gather_hook(&db, WRITERS - 1, move |_, stage, ops| match stage {
            GroupStage::BeforeAppend => sizes.lock().unwrap().push(ops.len()),
            GroupStage::AfterSync => {
                syncs.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        })
    };
    let device = sys::device_handle();
    let start = Barrier::new(2);
    thread::scope(|scope| {
        let first = scope.spawn(|| {
            device.clone().adopt();
            start.wait();
            db.put_durable(b"w0", b"v0")
        });
        start.wait();
        // The first writer is inside its group; the rest must queue behind it.
        while !entered.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        let rest: Vec<_> = (1..WRITERS)
            .map(|i| {
                let (db, device) = (&db, device.clone());
                scope.spawn(move || {
                    device.adopt();
                    db.put_durable(format!("w{i}"), format!("v{i}"))
                })
            })
            .collect();
        first.join().unwrap().unwrap();
        for handle in rest {
            handle.join().unwrap().unwrap();
        }
    });
    db.set_group_hook(None);
    assert_eq!(*group_sizes.lock().unwrap(), vec![1, WRITERS - 1]);
    assert_eq!(syncs.load(Ordering::SeqCst), 2, "one sync per group");
    for i in 0..WRITERS {
        assert_eq!(
            db.get(format!("w{i}")).unwrap(),
            Some(format!("v{i}").into_bytes())
        );
    }
    drop(db);
    sys::power_loss();
    let db = Kiban::open_with_options(dir.path(), tiny()).unwrap();
    for i in 0..WRITERS {
        assert!(db.get(format!("w{i}")).unwrap().is_some(), "w{i} lost");
    }
}

#[test]
fn group_writes_are_invisible_until_synced_then_appear_together() {
    let dir = TempDir::new("gc-visibility");
    let _device = DeviceGuard::new();
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let observed = StdArc::new(StdMutex::new(Vec::new()));
    {
        let reader = db.clone();
        let observed = observed.clone();
        db.set_group_hook(Some(StdArc::new(move |stage, _| {
            // Applied(_) holds the memtable write guard, so reads would block.
            if matches!(stage, GroupStage::Applied(_)) {
                return;
            }
            let seen = (reader.get(b"k0").unwrap(), reader.get(b"k1").unwrap());
            observed.lock().unwrap().push((stage, seen));
        })));
    }
    let mut batch = WriteBatch::new();
    batch.put(b"k0", b"v").put(b"k1", b"v");
    db.write_durable(batch).unwrap();
    db.set_group_hook(None);
    let observed = observed.lock().unwrap();
    let both = (Some(b"v".to_vec()), Some(b"v".to_vec()));
    for (stage, seen) in observed.iter() {
        match stage {
            GroupStage::Published => assert_eq!(*seen, both),
            _ => assert_eq!(*seen, (None, None), "visible before sync at {stage:?}"),
        }
    }
    assert!(observed.len() >= 4);
}

#[test]
fn readers_never_see_a_prefix_of_a_durable_batch() {
    let dir = TempDir::new("gc-batch-atomic");
    let _device = DeviceGuard::new();
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let device = sys::device_handle();
    let done = std::sync::atomic::AtomicBool::new(false);
    thread::scope(|scope| {
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let (db, device, done) = (&db, device.clone(), &done);
                scope.spawn(move || {
                    device.adopt();
                    while !done.load(Ordering::SeqCst) {
                        let snapshot = db.snapshot().unwrap();
                        let values: Vec<_> = (0..4)
                            .map(|j| snapshot.get(format!("b{j}")).unwrap())
                            .collect();
                        // Every batch sets all four keys to one generation.
                        assert!(
                            values.windows(2).all(|pair| pair[0] == pair[1]),
                            "torn batch visible: {values:?}"
                        );
                    }
                })
            })
            .collect();
        let writers: Vec<_> = (0..3)
            .map(|w| {
                let (db, device) = (&db, device.clone());
                scope.spawn(move || {
                    device.adopt();
                    for round in 0..40 {
                        let mut batch = WriteBatch::new();
                        for j in 0..4 {
                            batch.put(format!("b{j}"), format!("w{w}-r{round}"));
                        }
                        db.write_durable(batch).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        done.store(true, Ordering::SeqCst);
        for reader in readers {
            reader.join().unwrap();
        }
    });
}

/// Every fault position inside a gathered group: all of its members fail
/// together, the engine poisons, nothing from the group is visible, and a
/// later reopen still holds everything that was acknowledged.
#[test]
fn failed_group_sync_fails_every_member_and_poisons() {
    const WRITERS: usize = 5;
    let (mut append_poisons, mut sync_poisons, mut clean_runs) = (0, 0, 0);
    for fault in 0..48 {
        let dir = TempDir::new("gc-fault");
        let _device = DeviceGuard::new();
        let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
        let entered = gather_hook(&db, WRITERS - 1, move |group, stage, _| {
            if group == 1 && stage == GroupStage::BeforeAppend {
                sys::install_fault(fault);
            }
        });
        let device = sys::device_handle();
        let start = Barrier::new(2);
        let results: Vec<(usize, Result<(), DbError>)> = thread::scope(|scope| {
            let first = scope.spawn(|| {
                device.clone().adopt();
                start.wait();
                (0, db.put_durable(b"w0", b"v0"))
            });
            start.wait();
            while !entered.load(Ordering::SeqCst) {
                thread::yield_now();
            }
            let rest: Vec<_> = (1..WRITERS)
                .map(|i| {
                    let (db, device) = (&db, device.clone());
                    scope.spawn(move || {
                        device.adopt();
                        (i, db.put_durable(format!("w{i}"), format!("v{i}")))
                    })
                })
                .collect();
            let mut all = vec![first.join().unwrap()];
            all.extend(rest.into_iter().map(|h| h.join().unwrap()));
            all
        });
        db.set_group_hook(None);
        assert!(results[0].1.is_ok(), "group 0 ran before the fault");
        let failed: Vec<_> = results[1..].iter().filter(|(_, r)| r.is_err()).collect();
        if failed.is_empty() {
            clean_runs += 1;
            assert!(!db.is_poisoned());
        } else {
            assert_eq!(failed.len(), WRITERS - 1, "a group fails as a whole");
            assert!(db.is_poisoned());
            for (i, result) in &failed {
                match result {
                    Err(DbError::Poisoned(PoisonCause::WalAppendFailed(_))) => {}
                    Err(DbError::Poisoned(PoisonCause::WalSyncFailed(_))) => {}
                    other => panic!("writer {i}: unexpected {other:?}"),
                }
                assert_eq!(
                    db.get(format!("w{i}")).unwrap(),
                    None,
                    "unsynced write visible"
                );
            }
            let cause = match &failed[0].1 {
                Err(DbError::Poisoned(cause)) => cause.clone(),
                _ => unreachable!(),
            };
            match cause {
                PoisonCause::WalAppendFailed(_) => append_poisons += 1,
                PoisonCause::WalSyncFailed(_) => sync_poisons += 1,
                _ => unreachable!(),
            }
            assert!(matches!(
                db.put_durable(b"after", b"x"),
                Err(DbError::Poisoned(_))
            ));
            assert!(matches!(
                db.write_durable(WriteBatch::new()),
                Err(DbError::Poisoned(_))
            ));
        }
        sys::clear_fault();
        drop(db);
        sys::power_loss();
        let reopened = Kiban::open_with_options(dir.path(), tiny()).unwrap();
        assert_eq!(reopened.get(b"w0").unwrap(), Some(b"v0".to_vec()));
        for (i, result) in &results[1..] {
            if result.is_ok() {
                assert!(reopened.get(format!("w{i}")).unwrap().is_some());
            }
        }
        if clean_runs >= 3 {
            break;
        }
    }
    assert!(
        append_poisons > 0 && sync_poisons > 0,
        "sweep never reached both phases"
    );
}

// ---------------------------------------------------------------- crashes

#[derive(Clone, Debug)]
enum ThreadOp {
    Put(Vec<u8>, Vec<u8>),
    /// Puts new keys and may delete older ones, atomically. Every operation
    /// adds a fresh key, so a thread's state differs after each operation.
    Batch(Vec<(Vec<u8>, Option<Vec<u8>>)>),
}

impl ThreadOp {
    fn apply(&self, state: &mut Map) {
        match self {
            Self::Put(k, v) => {
                state.insert(k.clone(), v.clone());
            }
            Self::Batch(entries) => {
                for (k, v) in entries {
                    match v {
                        Some(v) => state.insert(k.clone(), v.clone()),
                        None => state.remove(k),
                    };
                }
            }
        }
    }
}

/// Keys are thread-private (`t{thread}/{op}/{j}`), so each thread's state is
/// an exact function of how many of its sequential operations survived.
fn plan_thread(rng: &mut sys::SplitMix64, thread: usize, count: usize) -> Vec<ThreadOp> {
    let mut live: Vec<Vec<u8>> = Vec::new();
    let value = |rng: &mut sys::SplitMix64, tag: &str| {
        let len = match rng.below(10) {
            0 => 3000,
            1 => 700,
            _ => 8 + rng.below(80) as usize,
        };
        let mut v = format!("{tag}:").into_bytes();
        v.resize(v.len() + len, rng.next() as u8);
        v
    };
    (0..count)
        .map(|op| {
            let key = |j: usize| format!("t{thread}/{op:04}/{j}").into_bytes();
            match rng.below(20) {
                0..=2 if !live.is_empty() => {
                    let at = rng.below(live.len() as u64) as usize;
                    let old = live.swap_remove(at);
                    live.push(key(0));
                    ThreadOp::Batch(vec![
                        (key(0), Some(value(rng, &format!("m{thread}.{op}")))),
                        (old, None),
                    ])
                }
                3..=8 => {
                    let entries: Vec<_> = (0..2 + rng.below(3) as usize)
                        .map(|j| (key(j), Some(value(rng, &format!("b{thread}.{op}.{j}")))))
                        .collect();
                    live.extend(entries.iter().map(|(k, _)| k.clone()));
                    let mut entries = entries;
                    let (key, value) = entries[0].clone();
                    entries.insert(0, (key.clone(), Some(b"overwritten".to_vec())));
                    entries.insert(1, (key.clone(), None));
                    entries.push((key, value));
                    ThreadOp::Batch(entries)
                }
                _ => {
                    live.push(key(0));
                    ThreadOp::Put(key(0), value(rng, &format!("p{thread}.{op}")))
                }
            }
        })
        .collect()
}

fn tag_of(key: &[u8]) -> (usize, usize) {
    let text = std::str::from_utf8(key).unwrap();
    let mut parts = text.trim_start_matches('t').split('/');
    let thread = parts.next().unwrap().parse().unwrap();
    let op = parts.next().unwrap().parse().unwrap();
    (thread, op)
}

fn first_key(op: &DurableOp) -> Option<&[u8]> {
    match op {
        DurableOp::Put(k, _) | DurableOp::Delete(k) => Some(k),
        DurableOp::Batch(batch) => batch.ops.first().map(|(_, k, _)| k.as_slice()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CrashAt {
    BeforeAppend,
    AfterAppend,
    AfterSync,
    DuringApply,
    Published,
}

impl CrashAt {
    const ALL: [CrashAt; 5] = [
        Self::BeforeAppend,
        Self::AfterAppend,
        Self::AfterSync,
        Self::DuringApply,
        Self::Published,
    ];

    fn matches(self, stage: GroupStage, group_len: usize) -> bool {
        match (self, stage) {
            (Self::BeforeAppend, GroupStage::BeforeAppend)
            | (Self::AfterAppend, GroupStage::AfterAppend)
            | (Self::AfterSync, GroupStage::AfterSync)
            | (Self::Published, GroupStage::Published) => true,
            (Self::DuringApply, GroupStage::Applied(i)) => i == group_len / 2,
            _ => false,
        }
    }
}

struct Captured {
    device: sys::DeviceSnapshot,
    acked: Vec<usize>,
    synced: Vec<usize>,
    started: Vec<usize>,
    log: Vec<(usize, usize)>,
    group_len: usize,
}

struct Crash {
    at: CrashAt,
    target_group: usize,
    groups: AtomicUsize,
    log: StdMutex<Vec<(usize, usize)>>,
    acked: Vec<AtomicUsize>,
    synced: Vec<AtomicUsize>,
    started: Vec<AtomicUsize>,
    captured: StdMutex<Option<Captured>>,
    biggest_group: AtomicUsize,
}

impl Crash {
    fn capture(&self, group_len: usize) {
        let mut slot = self.captured.lock().unwrap();
        if slot.is_some() {
            return;
        }
        // Durability counts first: anything counted here was synced before
        // this device copy. Starts last: anything not counted had not begun.
        let acked: Vec<_> = self
            .acked
            .iter()
            .map(|acked| acked.load(Ordering::SeqCst))
            .collect();
        let synced = self
            .synced
            .iter()
            .map(|s| s.load(Ordering::SeqCst))
            .collect();
        let log = self.log.lock().unwrap().clone();
        let device = sys::snapshot_device();
        let started: Vec<_> = self
            .started
            .iter()
            .map(|a| a.load(Ordering::SeqCst))
            .collect();
        *slot = Some(Captured {
            device,
            acked,
            synced,
            started,
            log,
            group_len,
        });
    }
}

struct CrashOutcome {
    group_len: usize,
    captured_live: bool,
}

fn run_crash(seed: u64) -> Result<CrashOutcome, String> {
    let dir = TempDir::new("gc-crash");
    let _device = DeviceGuard::new();
    let mut rng = sys::SplitMix64(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xC0FFEE);
    let writers = 2 + (seed % 5) as usize;
    let per_thread = 6 + (seed / 5 % 7) as usize;
    let mut options = tiny();
    if seed & 1 == 1 {
        // Rotations and flushes interleave with groups on odd seeds.
        options.write_buffer_bytes = 1500;
    }
    options.point_index = seed & 2 != 0;
    let plans: Vec<Vec<ThreadOp>> = (0..writers)
        .map(|t| plan_thread(&mut rng, t, per_thread))
        .collect();
    let crash = StdArc::new(Crash {
        at: CrashAt::ALL[(seed / 35 % 5) as usize],
        target_group: (seed / 175 % 3) as usize,
        groups: AtomicUsize::new(0),
        log: StdMutex::new(Vec::new()),
        acked: (0..writers).map(|_| AtomicUsize::new(0)).collect(),
        synced: (0..writers).map(|_| AtomicUsize::new(0)).collect(),
        started: (0..writers).map(|_| AtomicUsize::new(0)).collect(),
        captured: StdMutex::new(None),
        biggest_group: AtomicUsize::new(0),
    });
    let db = SharedKiban::open_with_options(dir.path(), options.clone())
        .map_err(|e| format!("open: {e}"))?;
    {
        let (crash, queue) = (crash.clone(), db.group.clone());
        let current = AtomicUsize::new(0);
        db.set_group_hook(Some(StdArc::new(move |stage, ops| {
            let group_len = ops.len();
            match stage {
                GroupStage::BeforeAppend => {
                    let group = crash.groups.fetch_add(1, Ordering::SeqCst);
                    current.store(group, Ordering::SeqCst);
                    crash.biggest_group.fetch_max(group_len, Ordering::SeqCst);
                    if group == 0 {
                        // Let followers pile up behind the first group.
                        wait_for_queue(&queue, 2, Duration::from_millis(20));
                    }
                }
                GroupStage::AfterAppend => {
                    let mut log = crash.log.lock().unwrap();
                    log.extend(ops.iter().filter_map(first_key).map(tag_of));
                }
                GroupStage::AfterSync => {
                    // Sync is the persistence boundary, even if this leader
                    // has not published or delivered any acknowledgement yet.
                    for (t, op) in ops.iter().filter_map(first_key).map(tag_of) {
                        crash.synced[t].fetch_max(op + 1, Ordering::SeqCst);
                    }
                }
                _ => {}
            }
            if current.load(Ordering::SeqCst) == crash.target_group
                && crash.at.matches(stage, group_len)
            {
                crash.capture(group_len);
            }
        })));
    }
    let device = sys::device_handle();
    let start = Barrier::new(writers);
    thread::scope(|scope| {
        for (t, plan) in plans.iter().enumerate() {
            let (db, crash, device, start) = (&db, &crash, device.clone(), &start);
            scope.spawn(move || {
                device.adopt();
                start.wait();
                for op in plan {
                    crash.started[t].fetch_add(1, Ordering::SeqCst);
                    let result = match op {
                        ThreadOp::Put(k, v) => db.put_durable(k, v),
                        ThreadOp::Batch(entries) => {
                            let mut batch = WriteBatch::new();
                            for (k, v) in entries {
                                match v {
                                    Some(v) => batch.put(k.clone(), v.clone()),
                                    None => batch.delete(k.clone()),
                                };
                            }
                            db.write_durable(batch)
                        }
                    };
                    result.unwrap_or_else(|e| panic!("writer {t}: {e}"));
                    crash.acked[t].fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    let captured_live = crash.captured.lock().unwrap().is_some();
    if !captured_live {
        // The target instant never occurred: crash after everything instead.
        crash.capture(0);
    }
    db.set_group_hook(None);
    drop(db);
    let captured = crash.captured.lock().unwrap().take().unwrap();
    sys::restore_device(captured.device);
    sys::power_loss_seeded(seed ^ 0xDEAD_BEEF);

    let reopened = Kiban::open_with_options(dir.path(), options)
        .map_err(|e| format!("recovery failed: {e}"))?;
    let recovered: Map = reopened
        .iter()
        .collect::<Result<_, _>>()
        .map_err(|e| format!("scan: {e}"))?;
    let mut survived = Vec::new();
    for (t, plan) in plans.iter().enumerate() {
        let prefix = format!("t{t}/").into_bytes();
        let mine: Map = recovered
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut state = Map::new();
        let mut matched = None;
        for p in 0..=plan.len() {
            if state == mine {
                matched = Some(p);
                break;
            }
            if let Some(op) = plan.get(p) {
                op.apply(&mut state);
            }
        }
        let Some(p) = matched else {
            return Err(format!(
                "thread {t}: recovered state is not any prefix of its operations (torn batch?)"
            ));
        };
        if p < captured.acked[t].max(captured.synced[t]) {
            return Err(format!(
                "thread {t}: acknowledged {} and synced {} operations but only {p} survived\nacked={:?} started={:?} log={:?} group_len={} recovered_keys={:?}\nplan0={:?}",
                captured.acked[t],
                captured.synced[t],
                captured.acked,
                captured.started,
                captured.log,
                captured.group_len,
                recovered
                    .keys()
                    .map(|k| String::from_utf8_lossy(k).to_string())
                    .collect::<Vec<_>>(),
                plan.first()
                    .map(|o| format!("{o:?}").chars().take(80).collect::<String>())
            ));
        }
        if p > captured.started[t] {
            return Err(format!(
                "thread {t}: {p} operations survived but only {} had started",
                captured.started[t]
            ));
        }
        survived.push(p);
    }
    // One WAL order: the surviving operations must be a prefix of it.
    let counts = |upto: usize| {
        let mut per_thread = vec![0usize; writers];
        for (t, _) in &captured.log[..upto] {
            per_thread[*t] += 1;
        }
        per_thread
    };
    if !(0..=captured.log.len()).any(|cut| counts(cut) == survived) {
        return Err(format!(
            "survivors {survived:?} are not a prefix of the WAL order {:?}",
            captured.log
        ));
    }
    Ok(CrashOutcome {
        group_len: captured.group_len,
        captured_live,
    })
}

fn crash_seed(seed: u64) -> CrashOutcome {
    run_crash(seed).unwrap_or_else(|e| {
        panic!(
            "seed {seed}: {e}\nrepro: KIBAN_SIM_SEED_START={seed} KIBAN_SIM_SEEDS=1 cargo test --lib group_commit_crash_long -- --ignored --nocapture"
        )
    })
}

/// 35 seeds per crash point (seed/35 selects it); seeds 175.. move to later
/// groups. 350 seeds cover every point at groups 0 to 1 and several sizes.
#[test]
fn concurrent_writers_crash_at_every_group_stage_recovers_acknowledged_prefix() {
    let mut multi_member = [false; 5];
    let mut live = [0usize; 5];
    for seed in 0..350 {
        let outcome = crash_seed(seed);
        let at = (seed / 35 % 5) as usize;
        multi_member[at] |= outcome.group_len >= 2;
        live[at] += outcome.captured_live as usize;
    }
    assert!(
        live.iter().all(|&n| n >= 20),
        "crash points rarely reached: {live:?}"
    );
    assert!(
        multi_member.iter().all(|&m| m),
        "no crash point ever caught a multi-writer group: {multi_member:?}"
    );
}

#[test]
#[ignore = "long group-commit crash sweep; configure KIBAN_SIM_SEEDS and KIBAN_SIM_SEED_START"]
fn group_commit_crash_long() {
    fn setting(name: &str, default: u64) -> u64 {
        std::env::var(name).map_or(default, |v| v.parse().unwrap_or(default))
    }
    let start = setting("KIBAN_SIM_SEED_START", 0);
    let count = setting("KIBAN_SIM_SEEDS", 1000);
    for seed in start..start + count {
        crash_seed(seed);
    }
    eprintln!("checked {count} group-commit crash seeds from {start}");
}

// ------------------------------------------------- failure-path coverage

/// Q1/Q2: a failed fdatasync after the frames were appended must not advance
/// `last_sequence`, must not publish, and must poison. The appended frames may
/// still replay on a process-level reopen (unacknowledged-but-survived).
#[test]
fn failed_group_sync_leaves_sequence_unadvanced_and_unacked_frames_may_replay() {
    let dir = TempDir::new("gc-scratch-q1");
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    db.put_durable(b"base", b"1").unwrap();
    let before = db.write_lock().unwrap().last_sequence_for_test();
    assert_eq!(before, 1);

    // Op 0 is the BufWriter flush of the appended frames, op 1 the fdatasync.
    sys::install_fault(1);
    let failed = db.put_durable(b"unacked", b"v");
    sys::clear_fault();
    match &failed {
        Err(DbError::Poisoned(PoisonCause::WalSyncFailed(_))) => {}
        other => panic!("expected WalSyncFailed poison, got {other:?}"),
    }
    assert_eq!(
        db.write_lock().unwrap().last_sequence_for_test(),
        before,
        "last_sequence must not advance for a failed group"
    );
    assert_eq!(db.get(b"unacked").unwrap(), None, "not published");
    assert!(db.is_poisoned());
    assert!(matches!(
        db.put_durable(b"after", b"x"),
        Err(DbError::Poisoned(_))
    ));
    drop(db);

    // Frames were flushed to the OS before the failed fdatasync; a reopen in
    // the same process replays them, which is the "not acknowledged, may
    // survive" branch of the contract.
    let mut reopened = Kiban::open_with_options(dir.path(), tiny()).unwrap();
    eprintln!(
        "reopened: last_sequence={} unacked={:?}",
        reopened.last_sequence_for_test(),
        reopened.get(b"unacked").unwrap()
    );
    let survived = reopened.get(b"unacked").unwrap();
    assert!(matches!(survived.as_deref(), None | Some(b"v")));
    if survived.is_some() {
        assert!(
            reopened.last_sequence_for_test() >= 2,
            "recovery advanced past the appended frame"
        );
    }
    let ls = reopened.last_sequence_for_test();
    reopened.put(b"next", b"x").unwrap();
    assert_eq!(
        reopened.last_sequence_for_test(),
        ls + 1,
        "no sequence reuse after surviving frames"
    );
}

/// Q1 (truncated branch): if the unacknowledged frames do not survive, their
/// sequence numbers are reused by later writes. That is safe because the
/// records are gone from disk and memory, and recovery never reuses the
/// surviving prefix's numbers.
#[test]
fn lost_unacked_tail_after_failed_sync_frees_its_sequence_numbers() {
    let dir = TempDir::new("gc-scratch-q1b");
    let wal_path = dir.path().join("1.wal");
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    db.put_durable(b"base", b"1").unwrap();
    let synced_len = std::fs::metadata(&wal_path).unwrap().len();

    sys::install_fault(1);
    let _ = db.put_durable(b"gone", b"v");
    sys::clear_fault();
    drop(db);

    // Simulate a torn/lost tail exactly as recovery would leave it.
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&wal_path)
        .unwrap();
    f.set_len(synced_len).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let mut reopened = Kiban::open_with_options(dir.path(), tiny()).unwrap();
    assert_eq!(reopened.last_sequence_for_test(), 1);
    assert_eq!(reopened.get(b"gone").unwrap(), None);
    reopened.put(b"next", b"v").unwrap();
    assert_eq!(
        reopened.last_sequence_for_test(),
        2,
        "2 is reused only after it was lost"
    );
}

/// Q6/M3 probe: a genuine WAL *append* failure (large record forces a write
/// syscall inside `wal.put`, not inside sync) must poison. The existing
/// `failed_group_sync_*` sweep only ever faults the sync phases, so this
/// branch of `commit_group` is otherwise untested.
#[test]
fn failed_group_append_poisons_and_publishes_nothing() {
    let dir = TempDir::new("gc-scratch-appendfail");
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    let big = vec![7u8; 32 * 1024];
    sys::install_fault(0); // the append's direct write syscall
    let result = db.put_durable(b"big", &big);
    sys::clear_fault();
    eprintln!(
        "append-failure result: {result:?}; poisoned={}",
        db.is_poisoned()
    );
    assert!(db.is_poisoned(), "append failure must poison the engine");
    assert!(matches!(result, Err(DbError::Poisoned(_))));
    assert_eq!(db.get(b"big").unwrap(), None);
}

/// Q6/M5 probe: two requests in one group touching the same key must be
/// published in arrival order (later request wins). No existing test builds a
/// group with a repeated key, so a reversed publication loop goes unnoticed.
#[test]
fn group_publishes_same_key_requests_in_arrival_order() {
    let dir = TempDir::new("gc-scratch-order");
    let _device = DeviceGuard::new();
    let db = SharedKiban::open_with_options(dir.path(), tiny()).unwrap();
    // Group 0 (writer A, alone) blocks until two followers queue; A then
    // finishes, B is promoted and takes group 1 containing B and C.
    let entered = gather_hook(&db, 2, |_, _, _| {});
    let device = sys::device_handle();
    thread::scope(|scope| {
        let (db_ref, device_a) = (&db, device.clone());
        let a = scope.spawn(move || {
            device_a.adopt();
            db_ref.put_durable(b"a", b"A")
        });
        while !entered.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        let b = {
            let (db_ref, device) = (&db, device.clone());
            scope.spawn(move || {
                device.adopt();
                db_ref.put_durable(b"k", b"B")
            })
        };
        while db.group.queued() < 1 {
            thread::yield_now();
        }
        let c = {
            let (db_ref, device) = (&db, device.clone());
            scope.spawn(move || {
                device.adopt();
                db_ref.put_durable(b"k", b"C")
            })
        };
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        c.join().unwrap().unwrap();
    });
    db.set_group_hook(None);
    assert_eq!(
        db.get(b"k").unwrap(),
        Some(b"C".to_vec()),
        "the later request in the group must win"
    );
}

/// A freeze failure after a group committed and synced must acknowledge the
/// group (its writes are durable and visible, so an error would invite a
/// duplicate retry) and fail later writes through the sticky maintenance
/// error instead, without poisoning.
#[test]
fn freeze_failure_after_a_committed_group_acknowledges_and_fails_later_writes() {
    let dir = TempDir::new("gc-freeze-fail");
    let mut opts = tiny();
    opts.write_buffer_bytes = 1;
    let db = SharedKiban::open_with_options(dir.path(), opts).unwrap();

    // Fresh db, one put: op 0 = WAL flush, op 1 = group fdatasync,
    // op 2 = freeze's own (redundant) fdatasync, op 3 = new WAL's create_new.
    sys::install_fault(3);
    let result = db.put_durable(b"k", b"v");
    sys::clear_fault();
    result.expect("the group committed, so it is acknowledged");
    assert!(
        !db.is_poisoned(),
        "freeze failure before the commit point must not poison"
    );
    assert_eq!(db.get(b"k").unwrap(), Some(b"v".to_vec()));
    assert_eq!(db.write_lock().unwrap().last_sequence_for_test(), 1);
    // The failure is sticky: the next write fails fast and is not applied.
    assert!(matches!(
        db.put_durable(b"k2", b"v2"),
        Err(DbError::Maintenance(_))
    ));
    assert_eq!(db.get(b"k2").unwrap(), None);
    assert_eq!(db.write_lock().unwrap().last_sequence_for_test(), 1);
}

#[path = "group_commit_qa_tests.rs"]
mod qa_tests;
