use crate::settings::Settings;
use kiban::db::SharedKiban;
use rocksdb::statistics::{StatsLevel, Ticker};
use rocksdb::{
    BlockBasedOptions, Cache, DB, DBCompressionType, FlushOptions, Options, ReadOptions,
    WriteOptions,
};
use serde_json::{Value, json};
use std::io;

pub enum Engine {
    Kiban(SharedKiban, u64, u64),
    Rocks {
        db: Box<DB>,
        options: Box<Options>,
        statistics: bool,
        read_options: ReadOptions,
        buffered_write_options: WriteOptions,
        durable_write_options: WriteOptions,
    },
}
impl Engine {
    pub fn open(s: &Settings) -> io::Result<Self> {
        let c = &s.workload;
        if s.engine == "kiban" {
            return SharedKiban::open_with_options(&c.db, c.options.clone())
                .map(|db| Self::Kiban(db, c.options.base_level_bytes, c.options.level_multiplier))
                .map_err(io::Error::other);
        }
        let mut options = Options::default();
        options.create_if_missing(true);
        if s.rocks_statistics {
            options.enable_statistics();
            options.set_statistics_level(StatsLevel::ExceptHistogramOrTimers);
        }
        if s.profile == "matched" {
            options.set_compression_type(DBCompressionType::None);
            options.set_write_buffer_size(c.options.write_buffer_bytes);
            options.set_max_write_buffer_number(2);
            options.set_level_compaction_dynamic_level_bytes(false);
            options.set_target_file_size_base(c.options.target_file_size);
            options.set_max_bytes_for_level_base(c.options.base_level_bytes);
            options.set_max_bytes_for_level_multiplier(c.options.level_multiplier as f64);
            options.set_level_zero_file_num_compaction_trigger(
                i32::try_from(c.options.l0_compaction_trigger).map_err(io::Error::other)?,
            );
            options.set_level_zero_slowdown_writes_trigger(
                i32::try_from(c.options.l0_write_stall_trigger - 1).map_err(io::Error::other)?,
            );
            options.set_level_zero_stop_writes_trigger(
                i32::try_from(c.options.l0_write_stall_trigger).map_err(io::Error::other)?,
            );
            options.set_max_open_files(
                i32::try_from(c.options.max_open_table_files).map_err(io::Error::other)?,
            );
            options.set_manual_wal_flush(true);
            let cache = Cache::new_lru_cache(c.options.block_cache_bytes);
            let mut table = BlockBasedOptions::default();
            table.set_block_cache(&cache);
            table.set_bloom_filter(10.0, false);
            options.set_block_based_table_factory(&table);
        }
        let db = DB::open(&options, &c.db).map_err(io::Error::other)?;
        let mut durable_write_options = WriteOptions::default();
        durable_write_options.set_sync(true);
        Ok(Self::Rocks {
            db: Box::new(db),
            options: Box::new(options),
            statistics: s.rocks_statistics,
            read_options: ReadOptions::default(),
            buffered_write_options: WriteOptions::default(),
            durable_write_options,
        })
    }
    pub fn get(&self, key: &[u8]) -> io::Result<Option<Vec<u8>>> {
        match self {
            Self::Kiban(db, ..) => db.get(key).map_err(io::Error::other),
            Self::Rocks {
                db, read_options, ..
            } => db.get_opt(key, read_options).map_err(io::Error::other),
        }
    }
    pub fn put(&self, key: &[u8], value: &[u8], durable: bool) -> io::Result<()> {
        match self {
            Self::Kiban(db, ..) => {
                db.put(key, value)?;
                if durable {
                    db.sync()?;
                }
                Ok(())
            }
            Self::Rocks {
                db,
                buffered_write_options,
                durable_write_options,
                ..
            } => db
                .put_opt(
                    key,
                    value,
                    if durable {
                        durable_write_options
                    } else {
                        buffered_write_options
                    },
                )
                .map_err(io::Error::other),
        }
    }
    pub fn delete(&self, key: &[u8]) -> io::Result<()> {
        match self {
            Self::Kiban(db, ..) => db.delete(key).map_err(Into::into),
            Self::Rocks { db, .. } => db.delete(key).map_err(io::Error::other),
        }
    }
    pub fn sync(&self) -> io::Result<()> {
        match self {
            Self::Kiban(db, ..) => db.sync().map_err(Into::into),
            Self::Rocks { db, .. } => db.flush_wal(true).map_err(io::Error::other),
        }
    }
    pub fn flush(&self) -> io::Result<()> {
        match self {
            Self::Kiban(db, ..) => db.flush().map_err(io::Error::other),
            Self::Rocks { db, .. } => {
                let mut o = FlushOptions::default();
                o.set_wait(true);
                db.flush_opt(&o).map_err(io::Error::other)
            }
        }
    }
    pub fn stats(&self) -> io::Result<Value> {
        match self {
            Self::Kiban(db, ..) => {
                if let Some(e) = db.maintenance_error() {
                    return Err(io::Error::other(e));
                }
                let s = db.stats().map_err(io::Error::other)?;
                let m = s.maintenance;
                let (base, mult) = match self {
                    Self::Kiban(_, base, mult) => (*base, *mult),
                    _ => unreachable!(),
                };
                let debt =
                    s.levels
                        .iter()
                        .filter(|l| l.level > 0)
                        .fold(0u64, |debt, l| {
                            debt.saturating_add(l.bytes.saturating_sub(
                                base.saturating_mul(mult.saturating_pow(l.level - 1)),
                            ))
                        });
                Ok(
                    json!({"sst_bytes_written":m.flush_output_bytes + m.compaction_output_bytes,"compaction_read_bytes":m.compaction_input_bytes,
                    "compaction_write_bytes":m.compaction_output_bytes,"compaction_count":m.compactions_completed,"compaction_ns":m.compaction_elapsed_ns,
                    "flush_count":m.flushes_completed,"stall_count":m.write_stalls,"stall_ns":m.write_stall_ns,
                    "native_debt_bytes":debt,"pending_flush":s.immutable_present,"maintenance_running":m.compaction_running,"memtable_bytes":s.memtable_logical_bytes+s.immutable_logical_bytes,
                    "levels":s.levels.iter().map(|l|json!({"level":l.level,"files":l.tables,"bytes":l.bytes})).collect::<Vec<_>>(),
                    "read_amp":format!("{:?}",s.read_amp),"governor":format!("{:?}",s.governor)}),
                )
            }
            Self::Rocks {
                db,
                options,
                statistics,
                ..
            } => {
                let prop = |p: &str| db.property_int_value(p).map_err(io::Error::other);
                let ticker = |t| statistics.then(|| options.get_ticker_count(t));
                let files = db.live_files().map_err(io::Error::other)?;
                let levels: Vec<_>=(0..7).map(|level|json!({"level":level,"files":files.iter().filter(|f|f.level==level).count(),"bytes":files.iter().filter(|f|f.level==level).map(|f|f.size as u64).sum::<u64>()})).collect();
                Ok(
                    json!({"sst_bytes_written":if *statistics {Some(options.get_ticker_count(Ticker::FlushWriteBytes)+options.get_ticker_count(Ticker::CompactWriteBytes))} else {None},
                    "compaction_read_bytes":ticker(Ticker::CompactReadBytes),"compaction_write_bytes":ticker(Ticker::CompactWriteBytes),
                    "compaction_count":null,"compaction_ns":null,"flush_count":null,"stall_count":null,"stall_ns":null,
                    "pending_flush":prop("rocksdb.mem-table-flush-pending")?,"maintenance_running":prop("rocksdb.num-running-compactions")?,
                    "pending_compaction":prop("rocksdb.compaction-pending")?,"native_debt_bytes":prop("rocksdb.estimate-pending-compaction-bytes")?,
                    "memtable_bytes":prop("rocksdb.cur-size-all-mem-tables")?,"levels":levels,"background_errors":prop("rocksdb.background-errors")?}),
                )
            }
        }
    }
    pub fn settled(&self) -> io::Result<bool> {
        let s = self.stats()?;
        let no = |k| {
            s[k].as_bool()
                .map(|v| !v)
                .unwrap_or_else(|| s[k].as_u64() == Some(0))
        };
        Ok(no("pending_flush")
            && no("maintenance_running")
            && s["native_debt_bytes"].as_u64() == Some(0)
            && (s.get("pending_compaction").is_none() || no("pending_compaction")))
    }
}
