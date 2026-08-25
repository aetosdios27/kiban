//! SSTable format: immutable sorted tables.
//!
//! Specified in `docs/design/sstable.md`.

mod block;
pub mod builder;
pub mod reader;

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

pub use block::{BlockIter, VerifiedBlock};
pub use builder::TableBuilder;
pub use reader::{Found, Iter, SstTable};
