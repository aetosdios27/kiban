//! The MANIFEST: the authoritative record of database state.
//!
//! Per `docs/design/db-layout.md` D1 and `docs/design/compaction.md`
//! D1/D2: a single small file, rewritten atomically through the commit
//! policy on every change. Table entries carry their level; the level
//! axis carries recency between levels, file numbers within L0.

use std::fmt;
use std::io;
use std::path::Path;

use crate::atomic;
use crate::sys;

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
    /// Every WAL generation recovery must replay, strictly ascending —
    /// replay order equals numeric order (phase 11.8). Normally one
    /// entry; briefly two while an immutable memtable's flush is
    /// pending (the frozen memtable's old WAL stays live alongside the
    /// new active one until that flush commits). Represented as a list
    /// because that is what is actually true on disk, not because more
    /// than two is ever expected under this phase's one-immutable rule.
    pub wal_numbers: Vec<u64>,
    /// Highest sequence number durably captured by this state.
    pub last_sequence: u64,
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
            wal_numbers: vec![1],
            last_sequence: 0,
            tables: Vec::new(),
        }
    }

    /// ```text
    /// [next_file_number : u64 LE]
    /// [num_wals         : u32 LE]
    /// [wal_number       : u64 LE] * num_wals      strictly ascending
    /// [last_sequence    : u64 LE]
    /// [num_tables       : u32 LE]
    /// ([level : u32 LE][number : u64 LE]) * num_tables
    /// ```
    pub fn encode(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(8 + 4 + self.wal_numbers.len() * 8 + 8 + 4 + self.tables.len() * 12);
        out.extend_from_slice(&self.next_file_number.to_le_bytes());
        out.extend_from_slice(&(self.wal_numbers.len() as u32).to_le_bytes());
        for w in &self.wal_numbers {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out.extend_from_slice(&self.last_sequence.to_le_bytes());
        out.extend_from_slice(&(self.tables.len() as u32).to_le_bytes());
        for t in &self.tables {
            out.extend_from_slice(&t.level.to_le_bytes());
            out.extend_from_slice(&t.number.to_le_bytes());
        }
        out
    }

    /// Strict decode: exact byte consumption, WAL numbers non-empty,
    /// unique, and strictly ascending, tables sorted by (level, number)
    /// with unique numbers, and numbering invariants. Any violation is
    /// corruption — never repaired, never guessed.
    pub fn decode(bytes: &[u8]) -> Result<Manifest, ManifestError> {
        let bad = |m: &str| ManifestError(m.to_string());
        if bytes.len() < 12 {
            return Err(bad("shorter than the fixed header"));
        }
        let next_file_number = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let num_wals = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if num_wals == 0 {
            return Err(bad("no live wal generations"));
        }
        let wal_end = 12 + num_wals * 8;
        if bytes.len() < wal_end + 12 {
            return Err(bad("shorter than the wal list plus fixed trailer header"));
        }
        let mut wal_numbers = Vec::with_capacity(num_wals);
        let mut prev_wal: Option<u64> = None;
        for i in 0..num_wals {
            let start = 12 + i * 8;
            let n = u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap());
            if n == 0 {
                return Err(bad("wal number zero is not allocatable"));
            }
            if n >= next_file_number {
                return Err(bad("wal number not below next_file_number"));
            }
            if let Some(p) = prev_wal
                && p >= n
            {
                return Err(bad("wal numbers not strictly ascending"));
            }
            prev_wal = Some(n);
            wal_numbers.push(n);
        }
        let last_sequence = u64::from_le_bytes(bytes[wal_end..wal_end + 8].try_into().unwrap());
        let num_tables =
            u32::from_le_bytes(bytes[wal_end + 8..wal_end + 12].try_into().unwrap()) as usize;
        if bytes.len() != wal_end + 12 + num_tables * 12 {
            return Err(bad("length does not match declared table count"));
        }
        let mut tables = Vec::with_capacity(num_tables);
        let mut prev: Option<TableRef> = None;
        for i in 0..num_tables {
            let start = wal_end + 12 + i * 12;
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
            wal_numbers,
            last_sequence,
            tables,
        })
    }

    pub fn install(&self, dir: &Path) -> Result<(), atomic::CommitError> {
        atomic::commit_file(&dir.join(MANIFEST_NAME), &self.encode())
    }

    /// `Ok(None)` when no MANIFEST exists (fresh or crashed-first-open).
    pub fn load(dir: &Path) -> Result<Option<Manifest>, ManifestError> {
        match sys::read(&dir.join(MANIFEST_NAME)) {
            Ok(bytes) => Manifest::decode(&bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ManifestError(format!("reading manifest failed: {e}"))),
        }
    }

    pub fn max_level(&self) -> Option<u32> {
        self.tables.last().map(|t| t.level)
    }
}
