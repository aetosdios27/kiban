//! The MANIFEST: the authoritative record of database state.
//!
//! Per `docs/design/db-layout.md` D1 and `docs/design/compaction.md`
//! D1/D2: a single small file, rewritten atomically through the commit
//! policy on every change. Table entries carry their level; the level
//! axis carries recency between levels, file numbers within L0.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::atomic;

pub const MANIFEST_NAME: &str = "MANIFEST";
pub const MAX_LEVEL: u32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TableRef {
    pub level: u32,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub next_file_number: u64,
    pub wal_number: u64,
    pub tables: Vec<TableRef>,
}

#[derive(Debug)]
pub struct ManifestError(pub String);

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "manifest corrupt: {}", self.0)
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    pub fn fresh() -> Manifest {
        Manifest {
            next_file_number: 2,
            wal_number: 1,
            tables: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.tables.len() * 12);
        out.extend_from_slice(&self.next_file_number.to_le_bytes());
        out.extend_from_slice(&self.wal_number.to_le_bytes());
        out.extend_from_slice(&(self.tables.len() as u32).to_le_bytes());
        for t in &self.tables {
            out.extend_from_slice(&t.level.to_le_bytes());
            out.extend_from_slice(&t.number.to_le_bytes());
        }
        out
    }

    /// Strict decode: exact byte consumption, tables sorted by (level,
    /// number) with unique numbers, and numbering invariants. Any
    /// violation is corruption.
    pub fn decode(bytes: &[u8]) -> Result<Manifest, ManifestError> {
        let bad = |m: &str| ManifestError(m.to_string());
        if bytes.len() < 20 {
            return Err(bad("shorter than the fixed header"));
        }
        let next_file_number = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let wal_number = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let num_tables = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        if bytes.len() != 20 + num_tables * 12 {
            return Err(bad("length does not match declared table count"));
        }
        if wal_number == 0 || next_file_number <= wal_number {
            return Err(bad("numbering invariant violated (wal/next)"));
        }
        let mut tables = Vec::with_capacity(num_tables);
        let mut prev: Option<TableRef> = None;
        for i in 0..num_tables {
            let start = 20 + i * 12;
            let level = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
            let number = u64::from_le_bytes(bytes[start + 4..start + 12].try_into().unwrap());
            if level > MAX_LEVEL {
                return Err(bad("level beyond MAX_LEVEL"));
            }
            if number == 0 {
                return Err(bad("file number zero is not allocatable"));
            }
            if number >= next_file_number {
                return Err(bad("table number not below next_file_number"));
            }
            if let Some(p) = prev {
                let ordered = p < TableRef { level, number } && p.number != number;
                if !ordered {
                    return Err(bad(
                        "tables not in strictly ascending (level, number) order",
                    ));
                }
            }
            prev = Some(TableRef { level, number });
            tables.push(TableRef { level, number });
        }
        Ok(Manifest {
            next_file_number,
            wal_number,
            tables,
        })
    }

    pub fn install(&self, dir: &Path) -> Result<(), atomic::CommitError> {
        atomic::commit_file(&dir.join(MANIFEST_NAME), &self.encode())
    }

    /// `Ok(None)` when no MANIFEST exists (fresh or crashed-first-open).
    pub fn load(dir: &Path) -> Result<Option<Manifest>, ManifestError> {
        match fs::read(dir.join(MANIFEST_NAME)) {
            Ok(bytes) => Manifest::decode(&bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ManifestError(format!("reading manifest failed: {e}"))),
        }
    }

    pub fn max_level(&self) -> Option<u32> {
        self.tables.last().map(|t| t.level)
    }
}
