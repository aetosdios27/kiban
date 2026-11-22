use crate::{
    Options,
    api::{Backend, Result},
    model::{AGENTS, Memory, key},
};
use fjall::{
    CompressionType, Database, Keyspace, KeyspaceCreateOptions, PersistMode,
    config::{
        BlockSizePolicy, BloomConstructionPolicy, CompressionPolicy, FilterPolicy,
        FilterPolicyEntry, HashRatioPolicy, PinningPolicy, RestartIntervalPolicy,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Tuning {
    block_size: u32,
    bloom_bits: u8,
    memtable_mib: u32,
    lz4: bool,
    hash_ratio: f32,
    pin_blocks: bool,
    expect_point_read_hits: bool,
    restart_interval: Option<u8>,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            block_size: 4096,
            bloom_bits: 10,
            memtable_mib: 64,
            lz4: false,
            hash_ratio: 0.0,
            pin_blocks: false,
            expect_point_read_hits: false,
            restart_interval: None,
        }
    }
}

pub(crate) struct Fjall {
    records: Keyspace,
    db: Database,
    durable: bool,
    bulk: bool,
    memtable_bytes: u64,
}

impl Fjall {
    pub(crate) fn open(o: &Options) -> Result<Self> {
        let tuning = o.fjall;
        if !(1024..=1048576).contains(&tuning.block_size)
            || !(1..=30).contains(&tuning.bloom_bits)
            || !(1..=64).contains(&tuning.memtable_mib)
            || !(0.0..=4.0).contains(&tuning.hash_ratio)
            || tuning.restart_interval == Some(0)
        {
            return Err("invalid Fjall table options".into());
        }
        let compression = if tuning.lz4 {
            CompressionType::Lz4
        } else {
            CompressionType::None
        };
        let db = Database::builder(&o.suite.db)
            .cache_size(o.cache_bytes() as u64)
            .worker_threads(1)
            .journal_compression(CompressionType::None)
            .open()?;
        let records = db.keyspace("memories", || {
            let mut options = KeyspaceCreateOptions::default()
                .manual_journal_persist(true)
                .max_memtable_size(u64::from(tuning.memtable_mib) << 20)
                .data_block_size_policy(BlockSizePolicy::all(tuning.block_size))
                .filter_policy(FilterPolicy::all(FilterPolicyEntry::Bloom(
                    BloomConstructionPolicy::BitsPerKey(f32::from(tuning.bloom_bits)),
                )))
                .data_block_compression_policy(CompressionPolicy::all(compression))
                .index_block_compression_policy(CompressionPolicy::all(CompressionType::None))
                .data_block_hash_ratio_policy(HashRatioPolicy::all(tuning.hash_ratio))
                .expect_point_read_hits(tuning.expect_point_read_hits);
            if tuning.pin_blocks {
                options = options
                    .filter_block_pinning_policy(PinningPolicy::all(true))
                    .index_block_pinning_policy(PinningPolicy::all(true));
            }
            if let Some(interval) = tuning.restart_interval {
                options = options
                    .data_block_restart_interval_policy(RestartIntervalPolicy::all(interval));
            }
            options
        })?;
        let engine = Self {
            records,
            db,
            durable: o.durable,
            bulk: o.layout == "bulk",
            memtable_bytes: u64::from(tuning.memtable_mib) << 20,
        };
        eprintln!(
            "{}",
            json!({"type":"backend_config","backend":"fjall",
            "cache_bytes":engine.db.cache_capacity(),"requested_tuning":tuning,
            "manual_journal_persist":true,"workers":1,"durable":o.durable,"state":engine.state()})
        );
        Ok(engine)
    }

    fn state(&self) -> Value {
        json!({"tables":self.records.table_count(),"l0_tables":self.records.l0_table_count(),
            "active_compactions":self.db.active_compactions(),"outstanding_flushes":self.db.outstanding_flushes(),
            "sealed_memtables":self.records.sealed_memtable_count(),"journals":self.db.journal_count(),
            "write_buffer_bytes":self.db.write_buffer_size()})
    }
}

impl Backend for Fjall {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        if self.bulk {
            let mut ingest = self.records.start_ingestion()?;
            for agent in 0..AGENTS {
                for offset in (0..(n - agent).div_ceil(AGENTS)).rev() {
                    let id = agent + offset * AGENTS;
                    let m = Memory::new(id, 0, seed);
                    ingest.write(m.key().into_bytes(), serde_json::to_vec(&m)?)?;
                }
            }
            ingest.finish()?;
        } else {
            for id in 0..n {
                let m = Memory::new(id, 0, seed);
                self.records
                    .insert(m.key().into_bytes(), serde_json::to_vec(&m)?)?;
                // Leave space for one complete record. A single worker can stall if
                // automatic rotation fills the bounded queue with duplicate requests.
                if self.db.write_buffer_size() >= self.memtable_bytes - 1024 {
                    self.finish()?;
                }
            }
        }
        Ok(())
    }

    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        self.records
            .insert(memory.key().into_bytes(), serde_json::to_vec(memory)?)?;
        if self.durable {
            self.db.persist(PersistMode::SyncData)?;
        }
        Ok(())
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        let value = self
            .records
            .get(key(id).as_bytes())?
            .ok_or("missing Fjall memory")?;
        Ok(serde_json::from_slice(&value)?)
    }

    fn contains_key(&mut self, key: &[u8]) -> Result<bool> {
        Ok(self.records.contains_key(key)?)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        // The shared key encodes MAX - id, so forward key order is reverse creation order.
        let prefix = format!("agent/{agent:04}/memory/");
        self.records
            .prefix(prefix.as_bytes())
            .take(k)
            .map(|entry| Ok(serde_json::from_slice(&entry.value()?)?))
            .collect()
    }

    fn finish(&mut self) -> Result<()> {
        self.db.persist(PersistMode::SyncData)?;
        self.records.rotate_memtable()?;
        let started = Instant::now();
        loop {
            let state = self.state();
            if state["sealed_memtables"] == 0
                && state["outstanding_flushes"] == 0
                && state["active_compactions"] == 0
            {
                eprintln!("{}", json!({"type":"fjall_settled","state":state}));
                return Ok(());
            }
            if started.elapsed() >= Duration::from_secs(120) {
                return Err(format!("Fjall did not settle: {state}").into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

pub(crate) fn crash_write(o: &Options) -> Result<()> {
    use std::io::{Read, Write};

    if o.suite.backend != "fjall" || !o.durable {
        return Err("crash-write requires durable Fjall".into());
    }
    let c = &o.suite;
    let mut db = Fjall::open(o)?;
    crate::runner::verify_initial(&mut db, c)?;
    let mut stdout = std::io::stdout().lock();
    for id in c.n..c.n + c.operations {
        let memory = Memory::new(id, 0, c.seed);
        db.put(&memory, true)?;
        crate::runner::check(&db.get(id)?, id, 0, c.seed)?;
        writeln!(stdout, "ack {id}")?;
        stdout.flush()?;
    }
    // The parent keeps the pipe open and kills the process before finish or drop.
    std::io::stdin().read_exact(&mut [0])?;
    Err("expected SIGKILL after the final acknowledgment".into())
}
