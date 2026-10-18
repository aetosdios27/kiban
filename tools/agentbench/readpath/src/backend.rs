pub use crate::api::{Backend, Result};
pub use crate::embedded::Embedded;
use crate::{
    Mode,
    model::{Memory, key},
};
use heed3::{Database, Env, EnvFlags, EnvOpenOptions, FlagSetMode, types::Bytes};
use std::path::Path;

pub fn configure(env: &Env, mode: Mode) -> Result<serde_json::Value> {
    let relaxed = EnvFlags::NO_SYNC | EnvFlags::NO_META_SYNC;
    if mode == Mode::Fast {
        // This process owns the environment; no transaction or worker is active.
        unsafe {
            env.set_flags(relaxed, FlagSetMode::Enable)?;
        }
    }
    let actual = env.flags()?.ok_or("unknown LMDB environment flag")?;
    let expected = if mode == Mode::Fast {
        relaxed
    } else {
        EnvFlags::empty()
    };
    if actual.intersection(relaxed) != expected {
        return Err("LMDB durability flags do not match requested mode".into());
    }
    Ok(
        serde_json::json!({"lmdb_flags": format!("{actual:?}"), "lmdb_flags_bits": actual.bits(),
        "map_size_bytes": env.info().map_size, "heed3": "0.22.0"}),
    )
}

pub struct Lmdb {
    env: Env,
    records: Database<Bytes, Bytes>,
}

impl Lmdb {
    pub fn open(path: &Path, mode: Mode) -> Result<(Self, serde_json::Value)> {
        std::fs::create_dir_all(path)?;
        // The path belongs exclusively to this trial/process; no external LMDB user.
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024 * 1024)
                .max_dbs(200)
                .max_readers(200)
                .open(path)?
        };
        let mut txn = env.write_txn()?;
        let records = env.create_database(&mut txn, Some("memories"))?;
        txn.commit()?;
        let details = configure(&env, mode)?;
        Ok((Self { env, records }, details))
    }
    pub(crate) fn point(&self, id: u64) -> Result<Memory> {
        let txn = self.env.read_txn()?;
        let bytes = self
            .records
            .get(&txn, key(id).as_bytes())?
            .ok_or("missing LMDB memory")?;
        let result = serde_json::from_slice(bytes)?;
        txn.commit()?;
        Ok(result)
    }
}

impl Backend for Lmdb {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        let mut txn = self.env.write_txn()?;
        for id in 0..n {
            let memory = Memory::new(id, 0, seed);
            self.records.put(
                &mut txn,
                memory.key().as_bytes(),
                &serde_json::to_vec(&memory)?,
            )?;
            if (id + 1) % 10000 == 0 {
                eprintln!("lmdb: bulk preload {}/{}", id + 1, n);
            }
        }
        txn.commit()?;
        Ok(())
    }
    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        let key = memory.key();
        let bytes = serde_json::to_vec(memory)?;
        let mut txn = self.env.write_txn()?;
        self.records.put(&mut txn, key.as_bytes(), &bytes)?;
        txn.commit()?;
        Ok(())
    }

    fn contains_key(&mut self, key: &[u8]) -> Result<bool> {
        let txn = self.env.read_txn()?;
        let found = self.records.get(&txn, key)?.is_some();
        txn.commit()?;
        Ok(found)
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        self.point(id)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let txn = self.env.read_txn()?;
        let prefix = format!("agent/{agent:04}/memory/");
        let result = self
            .records
            .prefix_iter(&txn, prefix.as_bytes())?
            .take(k)
            .map(|entry| {
                let (_, bytes) = entry?;
                Ok(serde_json::from_slice(bytes)?)
            })
            .collect::<Result<Vec<_>>>()?;
        txn.commit()?;
        Ok(result)
    }

    fn finish(&mut self) -> Result<()> {
        self.env.force_sync()?;
        Ok(())
    }
}
