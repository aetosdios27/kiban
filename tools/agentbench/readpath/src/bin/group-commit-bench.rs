//! Concurrent durable-write benchmark: T writer threads each insert fresh
//! JSON memories, one acknowledged-durable write at a time, into a 10k base.
//!
//!   group-commit-bench prepare ENGINE DIR [N]
//!   group-commit-bench run ENGINE MODE THREADS OPS_PER_THREAD DIR OUT.json [N]
//!
//! Engines: kiban (modes `sync` = put+sync, `group` = put_durable), lmdb (one
//! write txn per mutation, default sync), sqlite (WAL, synchronous=FULL, one
//! shared connection behind a mutex), rocksdb (sync=true, its own write
//! groups). Every run verifies all writes after a close and reopen.
#[path = "../../../src/model.rs"]
#[allow(dead_code)]
mod model;
#[path = "../../../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;

use model::Memory;
use std::{
    error::Error,
    ffi::{c_char, c_void},
    path::Path,
    sync::{Arc, Barrier, Mutex},
    thread,
    time::Instant,
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SEED: u64 = 20261019;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// Syscall counters from the optional LD_PRELOAD probe (zeros without it).
struct Probe {
    reset: Option<unsafe extern "C" fn()>,
    read: Option<unsafe extern "C" fn(*mut u64)>,
}

impl Probe {
    fn find() -> Probe {
        // SAFETY: dlsym with RTLD_DEFAULT (null) and NUL-terminated names;
        // a found symbol has the C signatures declared in the probe source.
        unsafe {
            let reset = dlsym(std::ptr::null_mut(), c"gcp_reset".as_ptr());
            let read = dlsym(std::ptr::null_mut(), c"gcp_read".as_ptr());
            Probe {
                reset: (!reset.is_null()).then(|| std::mem::transmute(reset)),
                read: (!read.is_null()).then(|| std::mem::transmute(read)),
            }
        }
    }
    fn reset(&self) {
        if let Some(f) = self.reset {
            // SAFETY: probe function takes no arguments.
            unsafe { f() }
        }
    }
    fn read(&self) -> [u64; 6] {
        let mut out = [0u64; 6];
        if let Some(f) = self.read {
            // SAFETY: probe writes exactly six u64 values.
            unsafe { f(out.as_mut_ptr()) }
        }
        out
    }
}

trait Engine: Send + Sync {
    fn insert(&self, memory: &Memory) -> Result<()>;
    fn get(&self, id: u64) -> Result<Option<Vec<u8>>>;
}

fn encode(memory: &Memory) -> Result<(String, Vec<u8>)> {
    Ok((memory.key(), serde_json::to_vec(memory)?))
}

// ------------------------------------------------------------------ kiban

struct KibanEngine {
    db: kiban::db::SharedKiban,
    group: bool,
}

impl Engine for KibanEngine {
    fn insert(&self, memory: &Memory) -> Result<()> {
        let (key, value) = encode(memory)?;
        if self.group {
            #[cfg(feature = "group")]
            self.db.put_durable(key, value)?;
            #[cfg(not(feature = "group"))]
            return Err("built without the group feature".into());
        } else {
            self.db.put(key, value)?;
            self.db.sync()?;
        }
        Ok(())
    }
    fn get(&self, id: u64) -> Result<Option<Vec<u8>>> {
        Ok(self.db.get(model::key(id))?)
    }
}

fn open_kiban(path: &Path, group: bool) -> Result<KibanEngine> {
    Ok(KibanEngine {
        db: kiban::db::SharedKiban::open_with_options(path, kiban::db::KibanOptions::default())?,
        group,
    })
}

fn prepare_kiban(path: &Path, n: u64) -> Result<()> {
    let db = kiban::db::SharedKiban::open(path)?;
    for start in (0..n).step_by(10000) {
        let mut batch = kiban::db::WriteBatch::new();
        for id in start..(start + 10000).min(n) {
            let (key, value) = encode(&Memory::new(id, 0, SEED))?;
            batch.put(key.into_bytes(), value);
        }
        db.write(batch)?;
    }
    db.close()?;
    Ok(())
}

// ------------------------------------------------------------------- lmdb

struct LmdbEngine {
    env: Option<heed3::Env>,
    records: heed3::Database<heed3::types::Bytes, heed3::types::Bytes>,
}

fn lmdb_env(path: &Path) -> Result<heed3::Env> {
    std::fs::create_dir_all(path)?;
    // SAFETY: this process exclusively owns the benchmark environment.
    Ok(unsafe {
        heed3::EnvOpenOptions::new()
            .map_size(10 << 30)
            .max_dbs(4)
            .max_readers(64)
            .open(path)?
    })
}

impl Drop for LmdbEngine {
    // LMDB refuses a second open of the same environment in one process
    // until the first is fully closed; the verification pass reopens it.
    fn drop(&mut self) {
        if let Some(env) = self.env.take() {
            env.prepare_for_closing().wait();
        }
    }
}

fn open_lmdb(path: &Path) -> Result<LmdbEngine> {
    let env = lmdb_env(path)?;
    let txn = env.read_txn()?;
    let records = env
        .open_database(&txn, Some("memories"))?
        .ok_or("missing LMDB database")?;
    drop(txn);
    Ok(LmdbEngine { env: Some(env), records })
}

fn prepare_lmdb(path: &Path, n: u64) -> Result<()> {
    let env = lmdb_env(path)?;
    let mut txn = env.write_txn()?;
    let records: heed3::Database<heed3::types::Bytes, heed3::types::Bytes> =
        env.create_database(&mut txn, Some("memories"))?;
    for id in 0..n {
        let (key, value) = encode(&Memory::new(id, 0, SEED))?;
        records.put(&mut txn, key.as_bytes(), &value)?;
    }
    txn.commit()?;
    env.force_sync()?;
    Ok(())
}

impl Engine for LmdbEngine {
    fn insert(&self, memory: &Memory) -> Result<()> {
        let (key, value) = encode(memory)?;
        let mut txn = self.env.as_ref().unwrap().write_txn()?;
        self.records.put(&mut txn, key.as_bytes(), &value)?;
        txn.commit()?;
        Ok(())
    }
    fn get(&self, id: u64) -> Result<Option<Vec<u8>>> {
        let txn = self.env.as_ref().unwrap().read_txn()?;
        Ok(self
            .records
            .get(&txn, model::key(id).as_bytes())?
            .map(<[u8]>::to_vec))
    }
}

// ----------------------------------------------------------------- sqlite

struct SqliteEngine {
    connection: Mutex<rusqlite::Connection>,
}

fn sqlite_connection(path: &Path) -> Result<rusqlite::Connection> {
    std::fs::create_dir_all(path)?;
    let connection = rusqlite::Connection::open(path.join("memories.sqlite3"))?;
    connection.pragma_update(None, "page_size", 4096)?;
    let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    if mode != "wal" {
        return Err("SQLite refused WAL".into());
    }
    connection.pragma_update(None, "synchronous", 2)?; // FULL
    connection.pragma_update(None, "wal_autocheckpoint", 1000)?;
    Ok(connection)
}

fn prepare_sqlite(path: &Path, n: u64) -> Result<()> {
    let mut connection = sqlite_connection(path)?;
    connection.execute_batch(
        "CREATE TABLE memories(key TEXT PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;",
    )?;
    let txn = connection.transaction()?;
    {
        let mut statement = txn.prepare("INSERT INTO memories(key, value) VALUES(?1, ?2)")?;
        for id in 0..n {
            let (key, value) = encode(&Memory::new(id, 0, SEED))?;
            statement.execute(rusqlite::params![key, value])?;
        }
    }
    txn.commit()?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}

impl Engine for SqliteEngine {
    fn insert(&self, memory: &Memory) -> Result<()> {
        let (key, value) = encode(memory)?;
        let connection = self.connection.lock().unwrap();
        connection
            .prepare_cached("INSERT INTO memories(key, value) VALUES(?1, ?2)")?
            .execute(rusqlite::params![key, value])?;
        Ok(())
    }
    fn get(&self, id: u64) -> Result<Option<Vec<u8>>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare_cached("SELECT value FROM memories WHERE key=?1")?;
        let mut rows = statement.query([model::key(id)])?;
        Ok(rows.next()?.map(|r| r.get::<_, Vec<u8>>(0)).transpose()?)
    }
}

// ---------------------------------------------------------------- rocksdb

struct RocksEngine {
    db: rocksdb::DB,
    write: rocksdb::WriteOptions,
}

fn rocks_options() -> rocksdb::Options {
    let mut block = rocksdb::BlockBasedOptions::default();
    block.set_bloom_filter(10.0, false);
    block.set_block_size(4096);
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.set_block_based_table_factory(&block);
    opts.set_compression_type(rocksdb::DBCompressionType::None);
    opts.set_max_background_jobs(2);
    opts.set_use_fsync(false);
    opts
}

fn prepare_rocks(path: &Path, n: u64) -> Result<()> {
    let db = rocksdb::DB::open(&rocks_options(), path)?;
    let mut batch = rocksdb::WriteBatch::default();
    for id in 0..n {
        let (key, value) = encode(&Memory::new(id, 0, SEED))?;
        batch.put(key, value);
    }
    db.write(batch)?;
    db.flush()?;
    Ok(())
}

fn open_rocks(path: &Path) -> Result<RocksEngine> {
    let mut write = rocksdb::WriteOptions::default();
    write.set_sync(true);
    Ok(RocksEngine {
        db: rocksdb::DB::open(&rocks_options(), path)?,
        write,
    })
}

impl Engine for RocksEngine {
    fn insert(&self, memory: &Memory) -> Result<()> {
        let (key, value) = encode(memory)?;
        self.db.put_opt(key, value, &self.write)?;
        Ok(())
    }
    fn get(&self, id: u64) -> Result<Option<Vec<u8>>> {
        Ok(self.db.get(model::key(id))?)
    }
}

// ------------------------------------------------------------------ driver

fn open(engine: &str, mode: &str, path: &Path) -> Result<Arc<dyn Engine>> {
    Ok(match engine {
        "kiban" => Arc::new(open_kiban(path, mode == "group")?),
        "lmdb" => Arc::new(open_lmdb(path)?),
        "sqlite" => Arc::new(SqliteEngine {
            connection: Mutex::new(sqlite_connection(path)?),
        }),
        "rocksdb" => Arc::new(open_rocks(path)?),
        _ => return Err("engine must be kiban, lmdb, sqlite or rocksdb".into()),
    })
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

fn run(args: &[String]) -> Result<()> {
    let [engine, mode, threads, per_thread, dir, out, rest @ ..] = args else {
        return Err("run ENGINE MODE THREADS OPS_PER_THREAD DIR OUT.json [N]".into());
    };
    let threads: usize = threads.parse()?;
    let per_thread: u64 = per_thread.parse()?;
    let n: u64 = rest.first().map_or(Ok(10_000), |v| v.parse())?;
    let dir = Path::new(dir);
    let probe = Probe::find();
    let db = open(engine, mode, dir)?;
    // Warm the connection/threads outside the timed region with one read.
    db.get(0)?.ok_or("base record 0 missing")?;
    let start = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads as u64)
        .map(|t| {
            let (db, start) = (db.clone(), start.clone());
            thread::spawn(move || -> std::result::Result<Vec<u64>, String> {
                let memories: Vec<_> = (0..per_thread)
                    .map(|i| Memory::new(n + t * per_thread + i, 0, SEED))
                    .collect();
                let mut latencies = Vec::with_capacity(memories.len());
                start.wait();
                for memory in &memories {
                    let tick = Instant::now();
                    db.insert(memory).map_err(|e| e.to_string())?;
                    latencies.push(tick.elapsed().as_nanos() as u64);
                }
                Ok(latencies)
            })
        })
        .collect();
    probe.reset();
    start.wait();
    let begin = Instant::now();
    let mut latencies = Vec::new();
    let mut failure = None;
    for handle in handles {
        match handle.join().map_err(|_| "writer panicked")? {
            Ok(l) => latencies.extend(l),
            Err(e) => failure = Some(e.to_string()),
        }
    }
    let wall = begin.elapsed();
    let counts = probe.read();
    if let Some(e) = failure {
        return Err(format!("writer failed: {e}").into());
    }
    drop(db);
    // Close and reopen; every acknowledged write must be there byte for byte.
    let db = open(engine, mode, dir)?;
    for id in 0..n + threads as u64 * per_thread {
        let expected = serde_json::to_vec(&Memory::new(id, 0, SEED))?;
        if db.get(id)?.as_deref() != Some(expected.as_slice()) {
            return Err(format!("record {id} missing or wrong after reopen").into());
        }
    }
    latencies.sort_unstable();
    let writes = latencies.len() as f64;
    let micros = |ns: u64| ns as f64 / 1000.0;
    let row = serde_json::json!({
        "engine": engine, "mode": mode, "threads": threads, "ops_per_thread": per_thread,
        "writes": latencies.len(), "wall_s": wall.as_secs_f64(),
        "throughput": writes / wall.as_secs_f64(),
        "p50_us": micros(percentile(&latencies, 0.50)),
        "p99_us": micros(percentile(&latencies, 0.99)),
        "max_us": micros(*latencies.last().unwrap()),
        "mean_us": micros(latencies.iter().sum::<u64>() / latencies.len() as u64),
        "fdatasync": counts[0], "fsync": counts[1], "msync": counts[2],
        "sync_file_range": counts[3], "write_calls": counts[4], "write_bytes": counts[5],
        "probe": probe.read.is_some(), "verified": true,
    });
    std::fs::write(out, format!("{row}\n"))?;
    println!("{row}");
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("prepare") if args.len() >= 3 => {
            let n = args.get(3).map_or(10_000, |v| v.parse().unwrap());
            let path = Path::new(&args[2]);
            match args[1].as_str() {
                "kiban" => prepare_kiban(path, n),
                "lmdb" => prepare_lmdb(path, n),
                "sqlite" => prepare_sqlite(path, n),
                "rocksdb" => prepare_rocks(path, n),
                _ => Err("unknown engine".into()),
            }
        }
        Some("run") => run(&args[1..]),
        _ => Err("usage: group-commit-bench prepare|run ...".into()),
    };
    if let Err(e) = result {
        eprintln!("INVALID: {e}");
        std::process::exit(1);
    }
}
