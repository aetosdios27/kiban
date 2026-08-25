//! The MANIFEST: the authoritative record of database state.
//!
//! Per `docs/design/db-layout.md` D1: a single small file, rewritten
//! atomically through the commit policy on every change. The latest valid
//! MANIFEST *is* the database state.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::atomic;

pub const MANIFEST_NAME: &str = "MANIFEST";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub next_file_number: u64,
    pub wal_number: u64,
    pub table_numbers: Vec<u64>,
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
            table_numbers: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.table_numbers.len() * 8);
        out.extend_from_slice(&self.next_file_number.to_le_bytes());
        out.extend_from_slice(&self.wal_number.to_le_bytes());
        out.extend_from_slice(&(self.table_numbers.len() as u32).to_le_bytes());
        for t in &self.table_numbers {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out
    }

    /// Strict decode: exact byte consumption, ascending unique table
    /// numbers, and numbering invariants. Any violation is corruption.
    pub fn decode(bytes: &[u8]) -> Result<Manifest, ManifestError> {
        let bad = |m: &str| ManifestError(m.to_string());
        if bytes.len() < 20 {
            return Err(bad("shorter than the fixed header"));
        }
        let next_file_number = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let wal_number = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let num_tables = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        if bytes.len() != 20 + num_tables * 8 {
            return Err(bad("length does not match declared table count"));
        }
        if wal_number == 0 || next_file_number <= wal_number {
            return Err(bad("numbering invariant violated (wal/next)"));
        }
        let mut table_numbers = Vec::with_capacity(num_tables);
        let mut prev = None;
        for i in 0..num_tables {
            let start = 20 + i * 8;
            let t = u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap());
            if t == 0 {
                return Err(bad("table number zero is not allocatable"));
            }
            if t >= next_file_number {
                return Err(bad("table number not below next_file_number"));
            }
            if prev.is_some_and(|p: u64| p >= t) {
                return Err(bad("table numbers not strictly ascending"));
            }
            prev = Some(t);
            table_numbers.push(t);
        }
        Ok(Manifest {
            next_file_number,
            wal_number,
            table_numbers,
        })
    }

    /// Errors keep the atomic-commit distinction intact: `Failed` means
    /// the previous manifest is untouched and installable again;
    /// `RenamedNotDurable` means the new manifest may or may not have
    /// survived and recovery must resolve it.
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

    pub fn max_table_number(&self) -> Option<u64> {
        self.table_numbers.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn roundtrip() {
        let m = Manifest {
            next_file_number: 42,
            wal_number: 7,
            table_numbers: vec![2, 3, 5, 8],
        };
        assert_eq!(Manifest::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn fresh_manifest_roundtrips() {
        let m = Manifest::fresh();
        assert_eq!(Manifest::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn trailing_garbage_rejected() {
        let mut bytes = Manifest::fresh().encode();
        bytes.extend_from_slice(&[0u8; 8]);
        assert!(Manifest::decode(&bytes).is_err());
    }

    #[test]
    fn truncated_rejected() {
        let bytes = Manifest {
            next_file_number: 10,
            wal_number: 2,
            table_numbers: vec![3, 4],
        }
        .encode();
        assert!(Manifest::decode(&bytes[..bytes.len() - 4]).is_err());
        assert!(Manifest::decode(&bytes[..19]).is_err());
    }

    #[test]
    fn numbering_invariants_enforced() {
        let mk = |next, wal, tables| Manifest {
            next_file_number: next,
            wal_number: wal,
            table_numbers: tables,
        };
        assert!(Manifest::decode(&mk(1, 1, vec![]).encode()).is_err());
        assert!(Manifest::decode(&mk(5, 9, vec![]).encode()).is_err());
        assert!(Manifest::decode(&mk(5, 4, vec![4]).encode()).is_err());
        assert!(Manifest::decode(&mk(6, 1, vec![1, 1]).encode()).is_err());
        assert!(Manifest::decode(&mk(6, 1, vec![3, 2]).encode()).is_err());
        assert!(Manifest::decode(&mk(6, 1, vec![0]).encode()).is_err());
        // valid ones pass
        assert!(Manifest::decode(&mk(6, 5, vec![1, 2, 4]).encode()).is_ok());
    }

    #[test]
    fn install_then_load_roundtrips_and_overwrites() {
        let td = TempDir::new("manifest-install");
        let m1 = Manifest {
            next_file_number: 10,
            wal_number: 9,
            table_numbers: vec![1, 2],
        };
        m1.install(td.path()).unwrap();
        assert_eq!(Manifest::load(td.path()).unwrap(), Some(m1));

        let m2 = Manifest {
            next_file_number: 12,
            wal_number: 11,
            table_numbers: vec![1, 2, 10],
        };
        m2.install(td.path()).unwrap();
        assert_eq!(Manifest::load(td.path()).unwrap(), Some(m2));
    }

    #[test]
    fn load_absent_manifest_is_none() {
        let td = TempDir::new("manifest-absent");
        assert_eq!(Manifest::load(td.path()).unwrap(), None);
    }
}
