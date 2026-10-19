//! SSTable format: immutable sorted tables.
//!
//! Specified in `ARCHITECTURE.md`.

mod block;
pub mod builder;
mod filter;
pub(crate) mod point;
pub mod reader;
mod separator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Put,
    Tombstone,
}

impl Kind {
    pub fn to_u8(self) -> u8 {
        match self {
            Kind::Put => 0x01,
            Kind::Tombstone => 0x02,
        }
    }

    pub fn from_u8(b: u8) -> Option<Kind> {
        match b {
            0x01 => Some(Kind::Put),
            0x02 => Some(Kind::Tombstone),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum SstError {
    Corrupt(String),
    InvalidArgument(String),
}

impl std::fmt::Display for SstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SstError::Corrupt(m) => write!(f, "sstable corrupt: {m}"),
            SstError::InvalidArgument(m) => write!(f, "sstable invalid argument: {m}"),
        }
    }
}

impl std::error::Error for SstError {}

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

// Both a sorted output and its point arena can store every reconstructed
// full key. An input restart covers at most RESTART_INTERVAL keys; values
// stay in sorted blocks. Extra slack covers per-key headers and slots.
pub(crate) const POINT_OUTPUT_EXPANSION_BOUND: u64 = (2 * block::RESTART_INTERVAL + 8) as u64;

pub use block::{BlockIter, VerifiedBlock};
pub use builder::TableBuilder;
pub use reader::{Found, Iter, SstTable};
