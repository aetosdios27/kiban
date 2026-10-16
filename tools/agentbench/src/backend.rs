use crate::model::{Memory, key};
use kiban::db::Kiban;
use std::{error::Error, path::Path};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub trait Backend {
    fn prepare(&mut self) -> Result<()> {
        Ok(())
    }
    fn put(&mut self, memory: &Memory, insert: bool) -> Result<()>;
    fn get(&mut self, id: u64) -> Result<Memory>;
    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>>;
    fn finish(&mut self) -> Result<()>;
}

pub struct Embedded {
    db: Kiban,
}

impl Embedded {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: Kiban::open(path)?,
        })
    }
}

impl Backend for Embedded {
    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        self.db.put(memory.key(), serde_json::to_vec(memory)?)?;
        self.db.sync()?;
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
