use crate::model::Memory;
use std::error::Error;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub trait Backend {
    fn prepare(&mut self) -> Result<()> {
        Ok(())
    }
    fn put(&mut self, memory: &Memory, insert: bool) -> Result<()>;
    #[allow(dead_code)] // Used by the optional in-process engine harness.
    fn bulk_load(&mut self, _n: u64, _seed: u64) -> Result<()> {
        Err("bulk loading is unsupported by this backend".into())
    }
    fn get(&mut self, id: u64) -> Result<Memory>;
    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>>;
    fn finish(&mut self) -> Result<()>;
}
