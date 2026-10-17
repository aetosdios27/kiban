//! Seeded crash simulation for direct and manually scheduled shared engines.

use super::*;
use crate::sys;
use crate::testutil::TempDir;
use std::collections::BTreeMap;

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

#[derive(Clone, Debug)]
enum Mutation {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

impl Mutation {
    fn apply(&self, model: &mut Model) {
        match self {
            Self::Put(key, value) => {
                model.insert(key.clone(), value.clone());
            }
            Self::Delete(key) => {
                model.remove(key);
            }
        }
    }
}

fn matching_prefix(
    base: &Model,
    acknowledged: &[Vec<Mutation>],
    durable: usize,
    recovered: &Model,
) -> Result<usize, String> {
    let mut model = base.clone();
    for prefix in 0..=acknowledged.len() {
        if prefix >= durable && model == *recovered {
            return Ok(prefix);
        }
        if let Some(operation) = acknowledged.get(prefix) {
            for mutation in operation {
                mutation.apply(&mut model);
            }
        }
    }
    let summarize = |state: &Model| {
        state
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    value.len(),
                    value[..value.len().min(24)].to_vec(),
                )
            })
            .collect::<Vec<_>>()
    };
    Err(format!(
        "no legal prefix in {durable}..={} acknowledged operations; recovered={:?}; final={:?}",
        acknowledged.len(),
        summarize(recovered),
        summarize(&model),
    ))
}

const SIM_KEYS: u64 = 16;

#[derive(Clone, Copy)]
enum EngineMode {
    Direct,
    Shared,
}

impl EngineMode {
    fn open(self, path: &Path, mut options: KibanOptions) -> Result<Engine, DbError> {
        match self {
            Self::Direct => {
                Kiban::open_with_options(path, options).map(|db| Engine::Direct(Box::new(db)))
            }
            Self::Shared => {
                options.write_buffer_bytes = 512;
                options.l0_write_stall_trigger = 4;
                SharedKiban::open_manual(path, options).map(Engine::Shared)
            }
        }
    }

    fn long_test(self) -> &'static str {
        match self {
            Self::Direct => "seeded_crash_long",
            Self::Shared => "shared_seeded_crash_long",
        }
    }
}

enum Engine {
    Direct(Box<Kiban>),
    Shared(SharedKiban),
}

impl Engine {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        match self {
            Self::Direct(db) => db.put(key, value),
            Self::Shared(db) => db.put(key, value),
        }
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        match self {
            Self::Direct(db) => db.delete(key),
            Self::Shared(db) => db.delete(key),
        }
    }

    fn write(&mut self, batch: WriteBatch) -> Result<(), DbError> {
        match self {
            Self::Direct(db) => db.write(batch),
            Self::Shared(db) => db.write(batch),
        }
    }

    fn sync(&mut self) -> Result<(), DbError> {
        match self {
            Self::Direct(db) => db.sync(),
            Self::Shared(db) => db.sync(),
        }
    }

    fn flush(&mut self) -> Result<(), DbError> {
        match self {
            Self::Direct(db) => db.flush(),
            Self::Shared(db) => db.flush(),
        }
    }

    fn checkpoint(&mut self) -> Result<(), DbError> {
        match self {
            Self::Direct(db) => db.checkpoint(),
            Self::Shared(db) => db.checkpoint(),
        }
    }

    fn step(&self) -> Result<bool, DbError> {
        match self {
            Self::Direct(_) => Ok(false),
            Self::Shared(db) => {
                let did_work = db.step();
                db.maintenance_error()
                    .map_or(Ok(did_work), |e| Err(DbError::Maintenance(e)))
            }
        }
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        match self {
            Self::Direct(db) => db.get(key),
            Self::Shared(db) => db.get(key),
        }
    }

    fn entries(&self) -> Result<Vec<ScanEntry>, DbError> {
        match self {
            Self::Direct(db) => db.iter().collect(),
            Self::Shared(db) => db.snapshot()?.iter().collect(),
        }
    }
}

type DeviceImage = BTreeMap<PathBuf, Vec<u8>>;

fn device_image(dir: &Path) -> DeviceImage {
    sys::simulated_files_under(dir)
        .into_iter()
        .map(|path| {
            (
                path.strip_prefix(dir).unwrap().to_path_buf(),
                sys::read(&path).unwrap(),
            )
        })
        .collect()
}

fn read_model(db: &Engine) -> Result<Model, String> {
    let entries = db.entries().map_err(|e| format!("full scan: {e}"))?;
    if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err("scan returned duplicate or unordered keys".into());
    }
    let model: Model = entries.into_iter().collect();
    for key in (0..SIM_KEYS as u8)
        .map(|key| vec![key])
        .chain([Vec::new(), vec![u8::MAX]])
    {
        let actual = db.get(&key).map_err(|e| format!("get {key:?}: {e}"))?;
        if actual.as_ref() != model.get(&key) {
            return Err(format!("get {key:?} disagrees with full scan"));
        }
    }
    Ok(model)
}

fn operation(rng: &mut sys::SplitMix64, id: u64) -> Vec<Mutation> {
    let count = if rng.below(5) == 0 {
        2 + rng.below(4)
    } else {
        1
    };
    (0..count)
        .map(|index| {
            let key = vec![rng.below(SIM_KEYS) as u8];
            if rng.below(4) == 0 {
                Mutation::Delete(key)
            } else {
                let len = match rng.below(12) {
                    0 => 0,
                    1 => 10_000,
                    2 => 4096,
                    3 => 700,
                    _ => 32 + rng.below(96) as usize,
                };
                let mut value = vec![rng.next() as u8; len];
                for (to, from) in value
                    .iter_mut()
                    .zip(id.to_le_bytes().into_iter().chain(index.to_le_bytes()))
                {
                    *to = from;
                }
                Mutation::Put(key, value)
            }
        })
        .collect()
}

fn write_operation(db: &mut Engine, operation: &[Mutation]) -> Result<(), DbError> {
    if let [mutation] = operation {
        return match mutation {
            Mutation::Put(key, value) => db.put(key, value),
            Mutation::Delete(key) => db.delete(key),
        };
    }
    let mut batch = WriteBatch::new();
    for mutation in operation {
        match mutation {
            Mutation::Put(key, value) => batch.put(key.clone(), value.clone()),
            Mutation::Delete(key) => batch.delete(key.clone()),
        };
    }
    db.write(batch)
}

fn wal_creation_probe(dir: &Path, rng: &mut sys::SplitMix64) -> Result<(), String> {
    let path = dir.join("probe.wal");
    let mut memtable = Memtable::new();
    let (mut wal, _) = Wal::open(&path, &mut memtable).map_err(|e| e.to_string())?;
    wal.put(1, b"durable", b"wal creation must survive")
        .map_err(|e| e.to_string())?;
    wal.sync().map_err(|e| format!("{e:?}"))?;
    drop(wal);
    sys::power_loss_seeded(rng.next());
    let (wal, _) = Wal::open(&path, &mut memtable).map_err(|e| e.to_string())?;
    drop(wal);
    if memtable.get(b"durable").as_deref() != Some(b"wal creation must survive".as_slice()) {
        return Err("WAL creation lost a synced write before any MANIFEST directory sync".into());
    }
    Ok(())
}

fn run_seed(seed: u64, mode: EngineMode) -> Result<Vec<DeviceImage>, String> {
    struct DeviceGuard;
    impl Drop for DeviceGuard {
        fn drop(&mut self) {
            sys::clear_fault();
            sys::disable_device_sim();
        }
    }

    let dir = TempDir::new("seeded-prefix");
    sys::enable_device_sim();
    let _device = DeviceGuard;
    let run = || -> Result<Vec<DeviceImage>, String> {
        let mut rng = sys::SplitMix64(seed);
        let probe_dir = TempDir::new("seeded-wal");
        wal_creation_probe(probe_dir.path(), &mut rng).map_err(|e| format!("WAL probe: {e}"))?;
        let mut options = super::compaction_tests::tiny_options();
        options.point_index = seed & 1 != 0;
        let mut db = mode
            .open(dir.path(), options.clone())
            .map_err(|e| format!("initial open: {e}"))?;
        let mut base = Model::new();
        let mut images = Vec::new();
        for round in 0..3 {
            let mut acknowledged = Vec::new();
            let mut durable = 0;
            let steps = if round == 0 {
                1 + rng.below(12)
            } else {
                32 + rng.below(48)
            };
            for step in 0..steps {
                let operation = operation(&mut rng, (round << 32) | step);
                write_operation(&mut db, &operation)
                    .map_err(|e| format!("round {round}, write {step}: {e}"))?;
                acknowledged.push(operation);
                // Sixteen writes without explicit steps exercise inline backpressure;
                // the following burst varies between zero and four whole jobs.
                if matches!(mode, EngineMode::Shared) && step % 24 >= 16 {
                    for _ in 0..rng.below(5) {
                        db.step()
                            .map_err(|e| format!("round {round}, step after write {step}: {e}"))?;
                    }
                }
                if step == 0 || rng.below(8) == 0 {
                    db.sync().map_err(|e| format!("sync: {e}"))?;
                    durable = acknowledged.len();
                }
                if round > 0 && step % 8 == 7 {
                    if rng.below(2) == 0 {
                        db.checkpoint().map_err(|e| format!("checkpoint: {e}"))?;
                    } else {
                        db.flush().map_err(|e| format!("flush: {e}"))?;
                    }
                    durable = acknowledged.len();
                }
                if round > 0 && rng.below(32) == 0 {
                    drop(db);
                    db = mode
                        .open(dir.path(), options.clone())
                        .map_err(|e| format!("graceful reopen: {e}"))?;
                    let recovered = read_model(&db)?;
                    matching_prefix(&base, &acknowledged, acknowledged.len(), &recovered)?;
                    durable = acknowledged.len();
                }
            }
            let point = rng.below(80) as usize;
            let checkpoint = rng.below(2) == 0;
            let crash_seed = rng.next();
            let context = format!(
                "round {round}, steps {steps}, durable {durable}, crash seed {crash_seed}, syscall {point}"
            );
            if round == 0 {
                sys::capture_crash(0, || drop(db));
            } else {
                let (result, interrupted) = sys::capture_crash(point, || {
                    let result = if checkpoint {
                        db.checkpoint()
                    } else {
                        db.flush().and_then(|()| {
                            while db.step()? {}
                            Ok(())
                        })
                    };
                    drop(db);
                    result
                });
                if !interrupted {
                    result.map_err(|e| format!("{context}, maintenance: {e}"))?;
                    durable = acknowledged.len();
                }
            }
            sys::power_loss_seeded(crash_seed);
            // Exercise recovery with both write policies and mixed existing SSTs.
            options.point_index = !options.point_index;
            db = mode
                .open(dir.path(), options.clone())
                .map_err(|e| format!("{context}, crash reopen: {e}"))?;
            let recovered = read_model(&db).map_err(|e| format!("{context}: {e}"))?;
            matching_prefix(&base, &acknowledged, durable, &recovered)
                .map_err(|e| format!("{context}: {e}"))?;
            base = recovered;
            images.push(device_image(dir.path()));
        }
        Ok(images)
    };
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic");
            Err(format!("engine panicked: {message}"))
        });
    result.map_err(|e| {
        format!(
            "seed {seed}: {e}\nrepro: KIBAN_SIM_SEED_START={seed} KIBAN_SIM_SEEDS=1 cargo test --lib {} -- --ignored --nocapture",
            mode.long_test(),
        )
    })
}

#[test]
fn seeded_crash_fixed_corpus() {
    for seed in 0..64 {
        run_seed(seed, EngineMode::Direct).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
#[ignore = "long seeded crash sweep; configure KIBAN_SIM_SEEDS and KIBAN_SIM_SEED_START"]
fn seeded_crash_long() {
    long_run(EngineMode::Direct);
}

#[test]
fn shared_seeded_crash_fixed_corpus() {
    for seed in 0..64 {
        run_seed(seed, EngineMode::Shared).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn seeded_crash_device_state_is_deterministic() {
    assert_deterministic(EngineMode::Direct);
}

#[test]
fn shared_seeded_crash_device_state_is_deterministic() {
    assert_deterministic(EngineMode::Shared);
}

fn assert_deterministic(mode: EngineMode) {
    for seed in 0..8 {
        let first = run_seed(seed, mode).unwrap_or_else(|e| panic!("{e}"));
        let second = run_seed(seed, mode).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first, second, "seed {seed} produced different device bytes");
    }
}

#[test]
#[ignore = "long shared seeded crash sweep; configure KIBAN_SIM_SEEDS and KIBAN_SIM_SEED_START"]
fn shared_seeded_crash_long() {
    long_run(EngineMode::Shared);
}

fn long_run(mode: EngineMode) {
    fn setting(name: &str, default: u64) -> u64 {
        std::env::var(name).map_or(default, |value| {
            value
                .parse()
                .unwrap_or_else(|_| panic!("invalid {name}: {value}"))
        })
    }
    let start = setting("KIBAN_SIM_SEED_START", 0);
    let count = setting("KIBAN_SIM_SEEDS", 1000);
    assert!(count > 0, "KIBAN_SIM_SEEDS must be positive");
    let end = start.checked_add(count).expect("seed range overflow");
    let started = std::time::Instant::now();
    for seed in start..end {
        run_seed(seed, mode).unwrap_or_else(|e| panic!("{e}"));
    }
    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "checked {count} seeds ({start}..{end}), {} crashes in {elapsed:.3}s ({:.0} seeds/hour)",
        count * 4,
        count as f64 * 3600.0 / elapsed,
    );
}

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
    use super::Kiban;
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

    #[test]
    fn seeded_torn_wal_payload_preserves_synced_prefix() {
        for seed in 0..16 {
            on_device("wal-torn-payload", |dir| {
                let mut db = Kiban::open(dir).unwrap();
                db.put(b"a", b"synced").unwrap();
                db.sync().unwrap();
                db.put(b"b", [7; 10_000]).unwrap();
                drop(db);
                sys::power_loss_seeded(seed);
                let db = Kiban::open(dir)
                    .unwrap_or_else(|e| panic!("legitimate crash seed {seed}: {e}"));
                assert_eq!(db.get(b"a").unwrap(), Some(b"synced".to_vec()));
                let tail = db.get(b"b").unwrap();
                assert!(tail.is_none() || tail == Some(vec![7; 10_000]));
            });
        }
    }
}
