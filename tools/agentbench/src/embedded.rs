use crate::{
    api::{Backend, Result},
    model::{Memory, key},
};
use kiban::db::Kiban;
use std::path::Path;

pub struct Embedded {
    db: Kiban,
    durable: bool,
}

impl Embedded {
    pub fn open(path: &Path) -> Result<Self> {
        Self::with_durability(path, true)
    }

    pub fn with_durability(path: &Path, durable: bool) -> Result<Self> {
        Ok(Self {
            db: Kiban::open(path)?,
            durable,
        })
    }

    #[allow(dead_code)] // Optional cache diagnosis binary; original harness keeps defaults.
    pub fn with_options(path: &Path, options: kiban::db::KibanOptions) -> Result<Self> {
        Ok(Self {
            db: Kiban::open_with_options(path, options)?,
            durable: false,
        })
    }
}

impl Backend for Embedded {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        for start in (0..n).step_by(10000) {
            let mut batch = kiban::db::WriteBatch::new();
            for id in start..(start + 10000).min(n) {
                let memory = Memory::new(id, 0, seed);
                batch.put(memory.key().into_bytes(), serde_json::to_vec(&memory)?);
            }
            self.db.write(batch)?;
            eprintln!("kiban: bulk preload {}/{}", (start + 10000).min(n), n);
        }
        Ok(())
    }
    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        self.db.put(memory.key(), serde_json::to_vec(memory)?)?;
        if self.durable {
            self.db.sync()?;
        }
        Ok(())
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        let value = self.db.get(key(id))?.ok_or("missing memory")?;
        Ok(serde_json::from_slice(&value)?)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let start = format!("agent/{agent:04}/memory/");
        let end = format!("agent/{agent:04}/memory0");
        self.db
            .range(start.as_bytes(), end.as_bytes())
            .take(k)
            .map(|entry| {
                let (_, value) = entry?;
                Ok(serde_json::from_slice(&value)?)
            })
            .collect()
    }

    fn finish(&mut self) -> Result<()> {
        self.db.sync()?;
        self.db.flush()?;
        Ok(())
    }
}
