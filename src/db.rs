//! The engine handle: open, recover, read, write.
//!
//! Assembles WAL + memtable + sstables + MANIFEST into a crash-
//! recoverable database, per `docs/design/db-layout.md`. Single-threaded
//! by decision D7; only `sync()` earns a durability claim.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::manifest::{MANIFEST_NAME, Manifest, ManifestError};
use crate::memtable::{Entry as MemEntry, Memtable};
use crate::sstable::{Kind, SstError, SstTable, TableBuilder};
use crate::wal::{Wal, WalError};

pub const SST_EXTENSION: &str = "sst";
pub const WAL_EXTENSION: &str = "wal";

#[derive(Debug)]
pub enum DbError {
    Io(io::Error),
    Corrupt(String),
    CommitFailed(io::Error),
    CommitAmbiguous(io::Error),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Io(e) => write!(f, "database i/o error: {e}"),
            DbError::Corrupt(m) => write!(f, "database corrupt: {m}"),
            DbError::CommitFailed(e) => {
                write!(f, "manifest install failed; previous state intact: {e}")
            }
            DbError::CommitAmbiguous(e) => write!(
                f,
                "manifest rename not known durable; recovery must resolve: {e}"
            ),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Io(e) | DbError::CommitFailed(e) | DbError::CommitAmbiguous(e) => Some(e),
            DbError::Corrupt(_) => None,
        }
    }
}

impl From<io::Error> for DbError {
    fn from(e: io::Error) -> Self {
        DbError::Io(e)
    }
}

impl From<WalError> for DbError {
    fn from(e: WalError) -> Self {
        match e {
            WalError::Io(e) => DbError::Io(e),
            WalError::Corrupt { offset, reason } => {
                DbError::Corrupt(format!("wal offset {offset}: {reason}"))
            }
        }
    }
}

impl From<SstError> for DbError {
    fn from(e: SstError) -> Self {
        match e {
            SstError::Corrupt(m) => DbError::Corrupt(m),
            SstError::InvalidArgument(m) => DbError::Corrupt(format!("builder misuse: {m}")),
        }
    }
}

impl From<ManifestError> for DbError {
    fn from(e: ManifestError) -> Self {
        DbError::Corrupt(e.0)
    }
}

impl From<atomic::CommitError> for DbError {
    fn from(e: atomic::CommitError) -> Self {
        match e {
            atomic::CommitError::Failed(e) => DbError::CommitFailed(e),
            atomic::CommitError::RenamedNotDurable(e) => DbError::CommitAmbiguous(e),
        }
    }
}

fn file_name(number: u64, extension: &str) -> String {
    format!("{number}.{extension}")
}

struct TableEntry {
    number: u64,
    table: SstTable,
}

pub struct Kiban {
    dir: PathBuf,
    memtable: Memtable,
    wal: Wal,
    next_file_number: u64,
    tables: Vec<TableEntry>,
}

impl Kiban {
    /// Opens (or creates) a database in `dir`, running full recovery:
    /// MANIFEST validation, WAL replay, orphan sweep. See db-layout D3.
    pub fn open(dir: impl AsRef<Path>) -> Result<Kiban, DbError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        let manifest = match Manifest::load(&dir)? {
            Some(m) => m,
            None => Self::initialize_fresh(&dir)?,
        };

        // Sweep before touching anything: unreferenced artifacts are
        // garbage by definition (D3 step 4).
        Self::sweep_orphans(&dir, &manifest)?;

        let wal_path = dir.join(file_name(manifest.wal_number, WAL_EXTENSION));
        if !wal_path.exists() {
            return Err(DbError::Corrupt(format!(
                "manifest names wal {} which does not exist",
                manifest.wal_number
            )));
        }

        let mut memtable = Memtable::new();
        let (wal, _report) = Wal::open(&wal_path, &mut memtable)?;

        let mut tables = Vec::with_capacity(manifest.table_numbers.len());
        for number in &manifest.table_numbers {
            let path = dir.join(file_name(*number, SST_EXTENSION));
            let bytes = fs::read(&path).map_err(|e| {
                DbError::Corrupt(format!(
                    "manifest lists table {} which cannot be read: {e}",
                    number
                ))
            })?;
            tables.push(TableEntry {
                number: *number,
                table: SstTable::parse(bytes)?,
            });
        }

        Ok(Kiban {
            dir,
            memtable,
            wal,
            next_file_number: manifest.next_file_number,
            tables,
        })
    }

    fn initialize_fresh(dir: &Path) -> Result<Manifest, DbError> {
        let _ = fs::read_dir(dir).map(|entries| {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if is_recognized_artifact(name.to_str().unwrap_or("")) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        });
        let manifest = Manifest::fresh();
        atomic::create_durably(&dir.join(file_name(manifest.wal_number, WAL_EXTENSION)))?;
        manifest.install(dir).map_err(DbError::from)?;
        Ok(manifest)
    }

    fn sweep_orphans(dir: &Path, manifest: &Manifest) -> io::Result<()> {
        for entry in fs::read_dir(dir)?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some((number, extension)) = parse_artifact_name(name) else {
                continue;
            };
            let orphan = match extension {
                SST_EXTENSION => !manifest.table_numbers.contains(&number),
                WAL_EXTENSION => number != manifest.wal_number,
                _ => false,
            };
            if orphan {
                fs::remove_file(entry.path())?;
            }
        }
        for entry in fs::read_dir(dir)?.flatten() {
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|n| n.contains(atomic::TEMP_MARKER))
            {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> io::Result<()> {
        self.wal.put(key.as_ref(), value.as_ref())?;
        self.memtable.put(key, value);
        Ok(())
    }

    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> io::Result<()> {
        self.wal.delete(key.as_ref())?;
        self.memtable.delete(key);
        Ok(())
    }

    /// Makes all prior writes crash-durable. Only after this returns
    /// success may the caller treat them as acknowledged (db-layout D7).
    pub fn sync(&mut self) -> io::Result<()> {
        self.wal.sync()
    }

    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>, DbError> {
        let key = key.as_ref();
        match self.memtable.entry(key) {
            Some(MemEntry::Value(v)) => return Ok(Some(v.clone())),
            Some(MemEntry::Tombstone) => return Ok(None),
            None => {}
        }
        for entry in self.tables.iter().rev() {
            match entry.table.get(key)? {
                Some(found) => {
                    return Ok(match found.kind {
                        Kind::Put => Some(found.value.to_vec()),
                        Kind::Tombstone => None,
                    });
                }
                None => continue,
            }
        }
        Ok(None)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Flushes the memtable to a new sstable and retires the current WAL,
    /// following db-layout D2's single-commit-point pipeline.
    pub fn flush(&mut self) -> Result<(), DbError> {
        if self.memtable.is_empty() {
            return Ok(());
        }

        let sst_number = self.next_file_number;
        let new_wal_number = self.next_file_number + 1;
        let new_next_file_number = self.next_file_number + 2;

        let mut builder = TableBuilder::new();
        for (key, entry) in self.memtable.iter() {
            match entry {
                MemEntry::Value(v) => builder.add(Kind::Put, key, v)?,
                MemEntry::Tombstone => builder.add(Kind::Tombstone, key, b"")?,
            }
        }
        let bytes = builder.finish()?;

        // D2 step 2: publish the table. Crash here leaves an orphan the
        // sweep removes; the old MANIFEST still rules.
        atomic::commit_file(&self.dir.join(file_name(sst_number, SST_EXTENSION)), &bytes)?;

        // D2 step 3: the WAL named by the upcoming MANIFEST must exist
        // durably before that MANIFEST does.
        let new_wal_path = self.dir.join(file_name(new_wal_number, WAL_EXTENSION));
        atomic::create_durably(&new_wal_path)?;

        // D2 step 4: the commit point.
        let mut table_numbers: Vec<u64> = self.tables.iter().map(|t| t.number).collect();
        table_numbers.push(sst_number);
        table_numbers.sort_unstable();
        Manifest {
            next_file_number: new_next_file_number,
            wal_number: new_wal_number,
            table_numbers,
        }
        .install(&self.dir)
        .map_err(DbError::from)?;

        // D2 step 5: everything below only runs once the commit point has
        // returned success.
        self.next_file_number = new_next_file_number;
        self.tables.push(TableEntry {
            number: sst_number,
            table: SstTable::parse(bytes)?,
        });

        let old_wal_path = self.wal.path().to_path_buf();
        let mut fresh_memtable = Memtable::new();
        let (wal, _report) = Wal::open(&new_wal_path, &mut fresh_memtable)?;
        self.wal = wal;
        self.memtable = fresh_memtable;

        // Best-effort deletion; recovery's sweep owns stragglers (D2).
        let _ = fs::remove_file(old_wal_path);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn wal_for_test(&mut self) -> &mut Wal {
        &mut self.wal
    }

    #[cfg(test)]
    pub(crate) fn live_table_numbers(&self) -> Vec<u64> {
        self.tables.iter().map(|t| t.number).collect()
    }
}

fn parse_artifact_name(name: &str) -> Option<(u64, &str)> {
    let (number, extension) = name.split_once('.')?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !(extension == SST_EXTENSION || extension == WAL_EXTENSION) {
        return None;
    }
    Some((number.parse().ok()?, extension))
}

fn is_recognized_artifact(name: &str) -> bool {
    name == MANIFEST_NAME
        || parse_artifact_name(name).is_some()
        || name.contains(atomic::TEMP_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sstable::TableBuilder;
    use crate::testutil::TempDir;

    fn fresh_db(label: &str) -> (TempDir, Kiban) {
        let td = TempDir::new(label);
        let db = Kiban::open(td.path()).unwrap();
        (td, db)
    }

    #[test]
    fn fresh_open_creates_layout_and_survives_reopen() {
        let (td, mut db) = fresh_db("fresh");
        assert_eq!(Manifest::load(td.path()).unwrap().unwrap().wal_number, 1);
        db.put(b"a", b"1").unwrap();
        db.sync().unwrap();
        drop(db);

        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(db.get("missing").unwrap(), None);
    }

    #[test]
    fn unsynced_but_flushed_writes_replay_on_reopen() {
        let (td, mut db) = fresh_db("replay");
        db.put(b"k", b"v").unwrap();
        drop(db); // no sync; bytes are in the OS page cache and survive
        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn delete_tombstones_survive_reopen_and_shadow_nothing_yet() {
        let (td, mut db) = fresh_db("delete");
        db.put(b"k", b"v1").unwrap();
        db.sync().unwrap();
        db.delete(b"k").unwrap();
        db.sync().unwrap();
        assert_eq!(db.get("k").unwrap(), None);
        drop(db);
        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("k").unwrap(), None);
    }

    #[test]
    fn torn_wal_tail_is_truncated_on_engine_open() {
        let (td, mut db) = fresh_db("engine-torn");
        db.put(b"good", b"v").unwrap();
        db.sync().unwrap();
        {
            let wal = db.wal_for_test();
            wal.writer_flush_for_test();
            use std::io::Write;
            wal.writer_get_mut_for_test()
                .write_all(&[0xde, 0xad])
                .unwrap();
        }
        drop(db);
        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("good").unwrap(), Some(b"v".to_vec()));
        // the truncated tail did not wedge the log
        let mut db = db;
        db.put(b"after", b"w").unwrap();
        db.sync().unwrap();
        drop(db);
        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("after").unwrap(), Some(b"w".to_vec()));
    }

    #[test]
    fn orphans_are_swept_referenced_files_are_not() {
        let (td, mut db) = fresh_db("orphans");
        db.put(b"k", b"v").unwrap();
        db.sync().unwrap();
        let manifest = Manifest::load(td.path()).unwrap().unwrap();

        // plant debris: an unlisted table, an old wal, a temp file
        fs::write(td.path().join(file_name(99, SST_EXTENSION)), b"junk").unwrap();
        fs::write(td.path().join(file_name(98, WAL_EXTENSION)), b"junk").unwrap();
        fs::write(td.path().join(".MANIFEST.kiban-tmp.1.2"), b"junk").unwrap();
        // an unrecognized file must be left alone
        fs::write(td.path().join("README.txt"), b"mine").unwrap();
        drop(db);

        Kiban::open(td.path()).unwrap();
        assert!(!td.path().join(file_name(99, SST_EXTENSION)).exists());
        assert!(!td.path().join(file_name(98, WAL_EXTENSION)).exists());
        assert!(!td.path().join(".MANIFEST.kiban-tmp.1.2").exists());
        assert!(td.path().join("README.txt").exists());
        assert!(
            td.path()
                .join(file_name(manifest.wal_number, WAL_EXTENSION))
                .exists()
        );
    }

    #[test]
    fn corrupted_manifest_refuses_to_open() {
        let (td, mut db) = fresh_db("bad-manifest");
        db.put(b"k", b"v").unwrap();
        drop(db);
        let path = td.path().join(MANIFEST_NAME);
        let mut raw = fs::read(&path).unwrap();
        raw.push(0x00); // trailing garbage -> strict decode fails
        fs::write(path, raw).unwrap();
        match Kiban::open(td.path()) {
            Err(DbError::Corrupt(_)) => {}
            Err(e) => panic!("expected corruption, got {e}"),
            Ok(_) => panic!("expected corruption, opened cleanly"),
        }
    }

    #[test]
    fn reads_shadow_correctly_across_memtable_and_preexisting_tables() {
        let td = TempDir::new("shadowing");
        let dir = td.path();

        // hand-build state: table 2 holds k=tombstone and other=old;
        // table 1 holds k=v1 and solo=only-in-1
        let mut t1 = TableBuilder::new();
        t1.add(Kind::Put, b"k", b"v1").unwrap();
        t1.add(Kind::Put, b"solo", b"only-in-1").unwrap();
        atomic::commit_file(
            &dir.join(file_name(1, SST_EXTENSION)),
            &t1.finish().unwrap(),
        )
        .unwrap();
        let mut t2 = TableBuilder::new();
        t2.add(Kind::Tombstone, b"k", b"").unwrap();
        t2.add(Kind::Put, b"other", b"old").unwrap();
        atomic::commit_file(
            &dir.join(file_name(2, SST_EXTENSION)),
            &t2.finish().unwrap(),
        )
        .unwrap();

        let manifest = Manifest {
            next_file_number: 3,
            wal_number: 1,
            table_numbers: vec![1, 2],
        };
        atomic::create_durably(&dir.join(file_name(1, WAL_EXTENSION))).unwrap();
        manifest.install(dir).unwrap();

        let mut db = Kiban::open(dir).unwrap();
        // tombstone in newest table shadows value in older one
        assert_eq!(db.get("k").unwrap(), None);
        // miss in newest falls through to older one
        assert_eq!(db.get("solo").unwrap(), Some(b"only-in-1".to_vec()));
        // newest-table hit
        assert_eq!(db.get("other").unwrap(), Some(b"old".to_vec()));
        // memtable outranks everything
        db.put(b"k", b"resurrected").unwrap();
        assert_eq!(db.get("k").unwrap(), Some(b"resurrected".to_vec()));
        // memtable tombstone outranks all tables
        db.delete(b"other").unwrap();
        assert_eq!(db.get("other").unwrap(), None);
    }

    #[test]
    fn missing_referenced_files_are_corruption_not_panic() {
        let td = TempDir::new("missing-sst");
        let manifest = Manifest {
            next_file_number: 5,
            wal_number: 1,
            table_numbers: vec![3],
        };
        atomic::create_durably(&td.path().join(file_name(1, WAL_EXTENSION))).unwrap();
        manifest.install(td.path()).unwrap();
        match Kiban::open(td.path()) {
            Err(DbError::Corrupt(_)) => {}
            Err(e) => panic!("expected corruption, got {e}"),
            Ok(_) => panic!("expected corruption, opened cleanly"),
        }
    }

    #[test]
    fn artifact_name_parsing_is_strict() {
        assert_eq!(parse_artifact_name("12.sst"), Some((12, SST_EXTENSION)));
        assert_eq!(parse_artifact_name("12.wal"), Some((12, WAL_EXTENSION)));
        assert!(parse_artifact_name("MANIFEST").is_none());
        assert!(parse_artifact_name(".sst").is_none());
        assert!(parse_artifact_name("12x.sst").is_none());
        assert!(parse_artifact_name("12.txt").is_none());
        assert!(parse_artifact_name("-1.sst").is_none());
    }
}

#[cfg(test)]
mod flush_tests {
    use super::*;
    use crate::testutil::TempDir;

    fn fresh_db(label: &str) -> (TempDir, Kiban) {
        let td = TempDir::new(label);
        let db = Kiban::open(td.path()).unwrap();
        (td, db)
    }

    #[test]
    fn empty_flush_is_a_noop() {
        let (td, mut db) = fresh_db("flush-empty");
        db.flush().unwrap();
        let m = Manifest::load(td.path()).unwrap().unwrap();
        assert_eq!(m.wal_number, 1);
        assert_eq!(m.next_file_number, 2);
        assert!(m.table_numbers.is_empty());
    }

    #[test]
    fn full_pipeline_across_two_flushes_and_reopen() {
        let (td, mut db) = fresh_db("flush-pipeline");
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();

        assert_eq!(db.get("a").unwrap(), Some(b"1".to_vec()));
        db.delete(b"a").unwrap();
        db.put(b"c", b"3").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();

        // a is now tombstoned in table 4; b survives in table 2
        assert_eq!(db.get("a").unwrap(), None);
        assert_eq!(db.get("b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(db.live_table_numbers(), vec![2, 4]);

        drop(db);
        let mut db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("a").unwrap(), None);
        assert_eq!(db.get("b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(db.get("c").unwrap(), Some(b"3".to_vec()));

        // memtable writes after recovery still outrank tables
        db.put(b"a", b"back").unwrap();
        assert_eq!(db.get("a").unwrap(), Some(b"back".to_vec()));
    }

    #[test]
    fn memtable_is_empty_after_flush_and_wal_rotated() {
        let (td, mut db) = fresh_db("flush-rotation");
        db.put(b"k", b"v").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        let m = Manifest::load(td.path()).unwrap().unwrap();
        assert_eq!(m.wal_number, 3);
        assert_eq!(m.table_numbers, vec![2]);
        assert!(!td.path().join(file_name(1, WAL_EXTENSION)).exists());
        // new wal is live: write, reopen without sync, still there
        db.put(b"k2", b"v2").unwrap();
        drop(db);
        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("k2").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn crash_before_commit_point_loses_nothing_and_sweeps_orphan() {
        // simulate a crash between D2 steps 2 and 4: an sst file exists,
        // but the MANIFEST still names the old wal holding all writes.
        let (td, mut db) = fresh_db("crash-before-commit");
        db.put(b"k", b"v").unwrap();
        db.sync().unwrap();
        drop(db);

        let manifest = Manifest::load(td.path()).unwrap().unwrap();
        assert_eq!(manifest.wal_number, 1);
        fs::write(td.path().join(file_name(5, SST_EXTENSION)), b"orphan").unwrap();

        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("k").unwrap(), Some(b"v".to_vec()));
        assert!(!td.path().join(file_name(5, SST_EXTENSION)).exists());
    }

    #[test]
    fn crash_after_commit_point_but_before_wal_deletion() {
        // real flush completes through the commit point, then we pretend
        // the process died before removing the retired wal
        let (td, mut db) = fresh_db("crash-after-commit");
        db.put(b"k", b"v").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        fs::write(
            td.path().join(file_name(1, WAL_EXTENSION)),
            b"retired-but-present",
        )
        .unwrap();
        drop(db);

        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("k").unwrap(), Some(b"v".to_vec()));
        assert!(!td.path().join(file_name(1, WAL_EXTENSION)).exists());
    }

    #[test]
    fn flushed_tombstones_shadow_older_table_values_after_reopen() {
        let (td, mut db) = fresh_db("flush-tombstone-shadow");
        db.put(b"k", b"old").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // table 2 holds k=old
        db.delete(b"k").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // table 4 holds k=tombstone
        drop(db);

        let db = Kiban::open(td.path()).unwrap();
        assert_eq!(db.get("k").unwrap(), None);
    }
}
