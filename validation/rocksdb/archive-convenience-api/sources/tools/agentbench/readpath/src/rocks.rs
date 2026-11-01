//! RocksDB adapter: the identical composite keys and JSON bytes, no secondary index.
use crate::{
    Options as BenchOptions,
    api::{Backend, Result},
    model::{AGENTS, Memory, key},
};
use rocksdb::{
    BlockBasedOptions, Cache, DB, DBCompressionType, DataBlockIndexType, Options, ReadOptions,
    SliceTransform, SstFileWriter, WaitForCompactOptions, WriteOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tuning {
    pub block_size: usize,
    pub ribbon: bool,
    pub total_order: bool,
    pub lz4: bool,
    pub mmap: bool,
    pub row_cache: bool,
    pub disable_wal: bool,
}
impl Default for Tuning {
    fn default() -> Self {
        Self {
            block_size: 4096,
            ribbon: false,
            total_order: false,
            lz4: false,
            mmap: false,
            row_cache: false,
            disable_wal: false,
        }
    }
}
fn options(o: &BenchOptions) -> Options {
    let t = &o.rocks;
    let mut block = BlockBasedOptions::default();
    block.set_block_cache(&Cache::new_lru_cache(o.cache_bytes()));
    block.set_cache_index_and_filter_blocks(true);
    block.set_pin_l0_filter_and_index_blocks_in_cache(true);
    block.set_data_block_index_type(DataBlockIndexType::BinaryAndHash);
    block.set_block_size(t.block_size);
    if t.ribbon {
        block.set_ribbon_filter(10.0);
    } else {
        block.set_bloom_filter(10.0, false);
    }
    block.set_whole_key_filtering(true);
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_block_based_table_factory(&block);
    opts.set_prefix_extractor(SliceTransform::create_fixed_prefix(11));
    opts.set_memtable_prefix_bloom_ratio(0.1);
    opts.set_compression_type(if t.lz4 {
        DBCompressionType::Lz4
    } else {
        DBCompressionType::None
    });
    opts.set_allow_mmap_reads(t.mmap);
    if t.row_cache {
        opts.set_row_cache(&Cache::new_lru_cache(o.cache_bytes()));
    }
    opts.set_max_background_jobs(2);
    opts.set_use_fsync(false);
    opts
}
pub struct Rocks {
    db: DB,
    write: WriteOptions,
    tuning: Tuning,
    bulk: bool,
    path: std::path::PathBuf,
    opts: Options,
}
impl Rocks {
    pub fn open(o: &BenchOptions) -> Result<Self> {
        if o.durable && o.rocks.disable_wal {
            return Err("sync=true with disableWAL is invalid".into());
        }
        let opts = options(o);
        let db = DB::open(&opts, &o.suite.db)?;
        let mut write = WriteOptions::default();
        write.set_sync(o.durable);
        write.disable_wal(o.rocks.disable_wal);
        let this = Self {
            db,
            write,
            tuning: o.rocks.clone(),
            bulk: o.layout == "bulk",
            path: o.suite.db.clone(),
            opts,
        };
        if matches!(o.action.as_str(), "read" | "profile") {
            this.settle()?;
        }
        eprintln!(
            "{}",
            json!({"type":"backend_config","backend":"rocksdb","version":"11.8.1","crate":"0.25.0","tuning":this.tuning,"cache_bytes":o.cache_bytes(),"sync":o.durable,"prefix_bytes":11,"state":this.state()?})
        );
        Ok(this)
    }
    pub fn state(&self) -> Result<serde_json::Value> {
        let names = [
            "rocksdb.num-running-compactions",
            "rocksdb.num-running-flushes",
            "rocksdb.compaction-pending",
            "rocksdb.mem-table-flush-pending",
            "rocksdb.estimate-pending-compaction-bytes",
            "rocksdb.cur-size-all-mem-tables",
            "rocksdb.block-cache-capacity",
            "rocksdb.block-cache-usage",
        ];
        let mut result = serde_json::Map::new();
        for name in names {
            result.insert(name.into(), json!(self.db.property_int_value(name)?));
        }
        result.insert("live_files".into(), json!(self.db.live_files()?.into_iter().map(|f| json!({"name":f.name,"level":f.level,"size":f.size,"entries":f.num_entries,"deletions":f.num_deletions})).collect::<Vec<_>>()));
        Ok(json!(result))
    }
    fn settle(&self) -> Result<()> {
        let mut wait = WaitForCompactOptions::default();
        wait.set_flush(true);
        wait.set_timeout(120_000_000);
        self.db.wait_for_compact(&wait)?;
        Ok(())
    }
}
impl Backend for Rocks {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        if self.bulk {
            let file = self.path.join("preload.sst");
            let mut sst = SstFileWriter::create(&self.opts);
            sst.open(&file)?;
            for agent in 0..AGENTS {
                for id in (0..n).rev().filter(|id| id % AGENTS == agent) {
                    let m = Memory::new(id, 0, seed);
                    sst.put(m.key(), serde_json::to_vec(&m)?)?;
                }
            }
            sst.finish()?;
            self.db.ingest_external_file(vec![file.clone()])?;
            std::fs::remove_file(file)?;
        } else {
            let mut load = WriteOptions::default();
            load.set_sync(false);
            for id in 0..n {
                let m = Memory::new(id, 0, seed);
                self.db.put_opt(m.key(), serde_json::to_vec(&m)?, &load)?;
            }
        }
        self.finish()
    }
    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        self.db
            .put_opt(memory.key(), serde_json::to_vec(memory)?, &self.write)?;
        Ok(())
    }
    fn get(&mut self, id: u64) -> Result<Memory> {
        let value = self
            .db
            .get_pinned(key(id))?
            .ok_or("missing RocksDB memory")?;
        Ok(serde_json::from_slice(&value)?)
    }
    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let start = format!("agent/{agent:04}/memory/");
        let end = format!("agent/{agent:04}/memory0");
        let mut read = ReadOptions::default();
        read.set_iterate_upper_bound(end.into_bytes());
        read.set_total_order_seek(self.tuning.total_order);
        read.set_prefix_same_as_start(!self.tuning.total_order);
        let mut iter = self.db.raw_iterator_opt(read);
        iter.seek(start.as_bytes());
        let mut out = Vec::with_capacity(k);
        while out.len() < k && iter.valid() {
            out.push(serde_json::from_slice(
                iter.value().ok_or("iterator missing value")?,
            )?);
            if out.len() < k {
                iter.next();
            }
        }
        iter.status()?;
        Ok(out)
    }
    fn finish(&mut self) -> Result<()> {
        self.db.flush_wal(true)?;
        self.settle()?;
        eprintln!(
            "{}",
            json!({"type":"rocksdb_settled","state":self.state()?})
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_table_seek_and_write_modes_roundtrip_and_reopen() -> Result<()> {
        let root = std::env::temp_dir().join(format!("rocksbench-matrix-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let mut case = 0;
        for ribbon in [false, true] {
            for block_size in [4096, 16384] {
                for total_order in [false, true] {
                    for mode in 0..3 {
                        case += 1;
                        let tuning = Tuning {
                            ribbon,
                            block_size,
                            total_order,
                            lz4: mode == 2,
                            disable_wal: mode == 1,
                            ..Tuning::default()
                        };
                        let mut o: BenchOptions = serde_json::from_value(json!({
                            "backend":"rocksdb", "db":root.join(format!("db-{case}")),
                            "n":640,"operations":100,"k":10,"seed":20261016,
                            "output":root.join(format!("read-{case}.jsonl")),"action":"read",
                            "layout":if mode == 2 {"normal"} else {"bulk"},
                            "cache_mib":32,"rocks":tuning,"durable":mode == 2
                        }))?;
                        crate::prepare(&o)?;
                        crate::reads(&o)?;
                        o.suite.output = root.join(format!("write-{case}.jsonl"));
                        o.action = "write".into();
                        crate::writes(&o)?;
                        let mut db = Rocks::open(&o)?;
                        db.put(&Memory::new(5, 999, o.suite.seed), false)?;
                        assert!(crate::runner::verify_initial(&mut db, &o.suite).is_err());
                    }
                }
            }
        }
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
