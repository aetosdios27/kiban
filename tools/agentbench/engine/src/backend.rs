pub use crate::api::{Backend, Result};
pub use crate::embedded::Embedded;
use crate::{
    Mode,
    model::{Memory, key},
};
use heed3::{Database, Env, EnvFlags, EnvOpenOptions, FlagSetMode, types::Bytes};
use helix_db::{
    helix_engine::{
        storage_core::{HelixGraphStorage, version_info::VersionInfo},
        traversal_core::traversal_value::TraversalValue,
        types::GraphError,
    },
    protocol::value::Value,
};
use std::path::Path;

#[allow(unused_variables, clippy::all)]
#[rustfmt::skip]
#[path = "generated.rs"]
mod generated;

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

fn project(value: &TraversalValue<'_>) -> std::result::Result<Memory, GraphError> {
    let text = |name| match value.get_property(name) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(GraphError::New(format!("missing/string property {name}"))),
    };
    let number = |name| match value.get_property(name) {
        Some(Value::U32(v)) => Ok(u64::from(*v)),
        Some(Value::U64(v)) => Ok(*v),
        _ => Err(GraphError::New(format!("missing/unsigned property {name}"))),
    };
    Ok(Memory {
        memory_id: text("memory_id")?,
        agent: u32::try_from(number("agent")?).map_err(|e| GraphError::New(e.to_string()))?,
        created_at: number("created_at")?,
        revision: number("revision")?,
        text: text("text")?,
    })
}

pub struct Helix {
    storage: HelixGraphStorage,
}

impl Helix {
    pub fn open(path: &Path, mode: Mode) -> Result<(Self, serde_json::Value)> {
        let mut config = generated::config().ok_or("missing generated Helix config")?;
        // None of W1-W5 queries full-text search; avoid an unused automatic index.
        config.bm25 = Some(false);
        let storage = HelixGraphStorage::new(
            path.to_str().ok_or("non-UTF8 database path")?,
            config,
            VersionInfo::default(),
        )?;
        let mut details = configure(&storage.graph_env, mode)?;
        details["bm25"] = serde_json::json!(false);
        Ok((Self { storage }, details))
    }
}

impl Backend for Helix {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        generated::bulk_remember(&self.storage, n, seed)?;
        Ok(())
    }
    fn prepare(&mut self) -> Result<()> {
        for agent in 0..crate::model::AGENTS as u32 {
            generated::init_agent(&self.storage, &generated::InitAgentInput { agent })?;
        }
        Ok(())
    }

    fn put(&mut self, memory: &Memory, insert: bool) -> Result<()> {
        if insert {
            generated::remember(&self.storage, memory)?;
        } else {
            generated::update(&self.storage, memory)?;
        }
        Ok(())
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        Ok(generated::recall(
            &self.storage,
            &generated::RecallInput {
                memory_id: format!("memory-{id:020}"),
            },
        )?)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        Ok(generated::recent(
            &self.storage,
            &generated::RecentInput {
                agent,
                k: u32::try_from(k)?,
            },
        )?)
    }

    fn finish(&mut self) -> Result<()> {
        self.storage.graph_env.force_sync()?;
        Ok(())
    }
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

    fn get(&mut self, id: u64) -> Result<Memory> {
        let txn = self.env.read_txn()?;
        let bytes = self
            .records
            .get(&txn, key(id).as_bytes())?
            .ok_or("missing LMDB memory")?;
        let result = serde_json::from_slice(bytes)?;
        txn.commit()?;
        Ok(result)
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
