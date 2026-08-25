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
use crate::manifest::{MANIFEST_NAME, Manifest, ManifestError, TableRef};
use crate::memtable::{Entry as MemEntry, Memtable};
use crate::sstable::{Kind, SstError, SstTable, TableBuilder};
use crate::sys;
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
    level: u32,
    number: u64,
    size: u64,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    table: SstTable,
}

/// Tunables for flush/compaction behavior (compaction.md configuration).
#[derive(Debug, Clone)]
pub struct KibanOptions {
    pub l0_compaction_trigger: usize,
    pub base_level_bytes: u64,
    pub level_multiplier: u64,
    pub target_file_size: u64,
}

impl Default for KibanOptions {
    fn default() -> Self {
        const MIB: u64 = 1 << 20;
        KibanOptions {
            l0_compaction_trigger: 4,
            base_level_bytes: 4 * MIB,
            level_multiplier: 10,
            target_file_size: 4 * MIB,
        }
    }
}

pub struct Kiban {
    dir: PathBuf,
    options: KibanOptions,
    memtable: Memtable,
    wal: Wal,
    next_file_number: u64,
    wal_number: u64,
    /// sorted by (level, number)
    tables: Vec<TableEntry>,
}

impl Kiban {
    /// Opens (or creates) a database in `dir`, running full recovery:
    /// MANIFEST validation, WAL replay, orphan sweep. See db-layout D3.
    pub fn open(dir: impl AsRef<Path>) -> Result<Kiban, DbError> {
        Self::open_with_options(dir, KibanOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: KibanOptions,
    ) -> Result<Kiban, DbError> {
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

        let mut tables = Vec::with_capacity(manifest.tables.len());
        for tref in &manifest.tables {
            let path = dir.join(file_name(tref.number, SST_EXTENSION));
            let bytes = sys::read(&path).map_err(|e| {
                DbError::Corrupt(format!(
                    "manifest lists table {} which cannot be read: {e}",
                    tref.number
                ))
            })?;
            let size = bytes.len() as u64;
            let table = SstTable::parse(bytes)?;
            let first_key = table.smallest_key()?;
            let last_key = table.largest_key()?;
            tables.push(TableEntry {
                level: tref.level,
                number: tref.number,
                size,
                first_key,
                last_key,
                table,
            });
        }

        // compaction.md D2: L>=1 levels must be range-disjoint. Within a
        // level, files are checked in KEY order — file numbers record
        // creation time, which need not match keyspace position.
        let mut level_view: Vec<&TableEntry> = tables.iter().filter(|t| t.level >= 1).collect();
        level_view.sort_by(|a, b| a.first_key.cmp(&b.first_key));
        for window in level_view.windows(2) {
            if window[0].level == window[1].level && window[0].last_key >= window[1].first_key {
                return Err(DbError::Corrupt(format!(
                    "level {} tables {} and {} have overlapping ranges: [{:?}..{:?}] vs [{:?}..{:?}]",
                    window[0].level,
                    window[0].number,
                    window[1].number,
                    String::from_utf8_lossy(&window[0].first_key),
                    String::from_utf8_lossy(&window[0].last_key),
                    String::from_utf8_lossy(&window[1].first_key),
                    String::from_utf8_lossy(&window[1].last_key),
                )));
            }
        }

        Ok(Kiban {
            dir,
            options,
            memtable,
            wal,
            next_file_number: manifest.next_file_number,
            wal_number: manifest.wal_number,
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
                SST_EXTENSION => !manifest.tables.iter().any(|t| t.number == number),
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
        // L0 first, newest file number wins
        for entry in self.tables.iter().rev().filter(|t| t.level == 0) {
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
        // then L>=1 by ascending level; disjoint ranges -> one candidate
        for entry in self.tables.iter().filter(|t| t.level >= 1) {
            if key < entry.first_key.as_slice() || key > entry.last_key.as_slice() {
                continue;
            }
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

    /// The engine's active configuration.
    pub fn options(&self) -> &KibanOptions {
        &self.options
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
        let mut table_refs: Vec<TableRef> = self
            .tables
            .iter()
            .map(|t| TableRef {
                level: t.level,
                number: t.number,
            })
            .collect();
        table_refs.push(TableRef {
            level: 0,
            number: sst_number,
        });
        table_refs.sort();
        Manifest {
            next_file_number: new_next_file_number,
            wal_number: new_wal_number,
            tables: table_refs,
        }
        .install(&self.dir)
        .map_err(DbError::from)?;

        // D2 step 5: everything below only runs once the commit point has
        // returned success.
        self.next_file_number = new_next_file_number;
        self.wal_number = new_wal_number;
        let size = bytes.len() as u64;
        let table = SstTable::parse(bytes)?;
        let entry = TableEntry {
            level: 0,
            number: sst_number,
            size,
            first_key: table.smallest_key()?,
            last_key: table.largest_key()?,
            table,
        };
        let pos = self
            .tables
            .partition_point(|t| (t.level, t.number) < (entry.level, entry.number));
        self.tables.insert(pos, entry);

        let old_wal_path = self.wal.path().to_path_buf();
        let mut fresh_memtable = Memtable::new();
        let (wal, _report) = Wal::open(&new_wal_path, &mut fresh_memtable)?;
        self.wal = wal;
        self.memtable = fresh_memtable;

        // Best-effort deletion; recovery's sweep owns stragglers (D2).
        let _ = fs::remove_file(old_wal_path);

        self.maybe_compact()
    }

    #[cfg(test)]
    pub(crate) fn wal_for_test(&mut self) -> &mut Wal {
        &mut self.wal
    }

    #[cfg(test)]
    pub(crate) fn live_table_numbers(&self) -> Vec<u64> {
        self.tables.iter().map(|t| t.number).collect()
    }

    /// Iterates all live entries in ascending byte-wise key order.
    pub fn iter(&self) -> DbIter<'_> {
        DbIter {
            core: self.merge_core(true),
        }
    }

    /// Iterates the half-open key range `[start, end)` of live entries.
    pub fn range<'a>(
        &'a self,
        start: &'a [u8],
        end: &'a [u8],
    ) -> impl Iterator<Item = Result<(Vec<u8>, &'a [u8]), DbError>> + 'a {
        let core = MergeCore {
            sources: self.sources_from(start),
            user_mode: true,
            failed: false,
        };
        let end = end.to_vec();
        DbIter { core }.take_while(move |item| match item {
            Ok((k, _)) => k.as_slice() < end.as_slice(),
            Err(_) => true,
        })
    }

    /// Internal iteration (raw mode) — newest entry per key, tombstones
    /// included. Compaction's input stream; exercised via scan tests
    /// until compaction consumes it.
    #[allow(dead_code)]
    pub(crate) fn iter_internal(&self) -> DbRawIter<'_> {
        DbRawIter {
            core: self.merge_core(false),
        }
    }

    fn merge_core<'a>(&'a self, user_mode: bool) -> MergeCore<'a> {
        MergeCore {
            sources: self.sources_from(b""),
            user_mode,
            failed: false,
        }
    }

    fn sources_from<'a>(&'a self, start: &[u8]) -> Vec<SourceHead<'a>> {
        let mut sources = Vec::with_capacity(self.tables.len() + 1);
        // newest first: memtable, then L0 by descending number, then
        // deeper levels ascending (within a level, higher number = newer
        // for L0; deeper levels are disjoint so order is irrelevant but
        // kept deterministic)
        sources.push(SourceHead {
            feed: SourceFeed::Mem(self.memtable.iter_from(start)),
            head: None,
            exhausted: false,
        });
        for table in self.tables.iter().rev().filter(|t| t.level == 0) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(table.table.iter_from(start)),
                head: None,
                exhausted: false,
            });
        }
        for table in self.tables.iter().filter(|t| t.level >= 1) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(table.table.iter_from(start)),
                head: None,
                exhausted: false,
            });
        }
        sources
    }

    #[allow(dead_code)]
    pub(crate) fn iter_from<'a>(&'a self, start: &[u8]) -> DbIter<'a> {
        DbIter {
            core: MergeCore {
                sources: self.sources_from(start),
                user_mode: true,
                failed: false,
            },
        }
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
            tables: vec![
                TableRef {
                    level: 0,
                    number: 1,
                },
                TableRef {
                    level: 0,
                    number: 2,
                },
            ],
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
            tables: vec![TableRef {
                level: 0,
                number: 3,
            }],
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
        assert!(m.tables.is_empty());
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
        assert_eq!(
            m.tables,
            vec![TableRef {
                level: 0,
                number: 2
            }]
        );
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

/// One entry as the merge sees it: the newest version of a key.
pub struct RawEntry<'a> {
    pub kind: Kind,
    pub value: &'a [u8],
}

struct HeadEntry<'a> {
    key: Vec<u8>,
    kind: Kind,
    value: &'a [u8],
}

enum SourceFeed<'a> {
    Mem(std::collections::btree_map::Range<'a, Vec<u8>, MemEntry>),
    Table(crate::sstable::Iter<'a>),
}

struct SourceHead<'a> {
    feed: SourceFeed<'a>,
    head: Option<HeadEntry<'a>>,
    exhausted: bool,
}

impl<'a> SourceHead<'a> {
    fn fill(&mut self) -> Result<(), SstError> {
        if self.head.is_some() || self.exhausted {
            return Ok(());
        }
        let next = match &mut self.feed {
            SourceFeed::Mem(it) => it.next().map(|(k, e)| match e {
                MemEntry::Value(v) => HeadEntry {
                    key: k.clone(),
                    kind: Kind::Put,
                    value: v,
                },
                MemEntry::Tombstone => HeadEntry {
                    key: k.clone(),
                    kind: Kind::Tombstone,
                    value: &[],
                },
            }),
            SourceFeed::Table(it) => match it.next() {
                Some(Ok((kind, key, value))) => Some(HeadEntry { key, kind, value }),
                Some(Err(e)) => return Err(e),
                None => None,
            },
        };
        self.head = next;
        self.exhausted = self.head.is_none();
        Ok(())
    }

    fn advance(&mut self) {
        self.head = None;
    }
}

struct MergeCore<'a> {
    sources: Vec<SourceHead<'a>>,
    user_mode: bool,
    failed: bool,
}

impl<'a> MergeCore<'a> {
    /// Newest-wins merge over all sources (db-iterator.md D2). In user
    /// mode tombstones are skipped; in raw mode they are emitted.
    fn next_raw(&mut self) -> Option<Result<(Vec<u8>, RawEntry<'a>), DbError>> {
        if self.failed {
            return None;
        }
        loop {
            for source in &mut self.sources {
                if let Err(e) = source.fill() {
                    self.failed = true;
                    return Some(Err(DbError::from(e)));
                }
            }

            let mut min_key: Option<Vec<u8>> = None;
            for source in &self.sources {
                if let Some(head) = &source.head {
                    min_key = Some(match min_key {
                        Some(current) if current.as_slice() <= head.key.as_slice() => current,
                        _ => head.key.clone(),
                    });
                }
            }
            let min_key = min_key?;

            let mut newest: Option<(Kind, &'a [u8])> = None;
            for source in &mut self.sources {
                let matches = source
                    .head
                    .as_ref()
                    .is_some_and(|h| h.key.as_slice() == min_key);
                if matches {
                    if newest.is_none() {
                        let head = source.head.as_ref().unwrap();
                        newest = Some((head.kind, head.value));
                    }
                    source.advance();
                }
            }
            let (kind, value) = newest.expect("at least one source matched the minimum key");

            if self.user_mode && kind == Kind::Tombstone {
                continue;
            }
            return Some(Ok((min_key, RawEntry { kind, value })));
        }
    }
}

/// User-visible iteration: live data only (tombstones and shadowed
/// versions invisible). Agrees exactly with `get` by construction.
pub struct DbIter<'a> {
    core: MergeCore<'a>,
}

impl<'a> Iterator for DbIter<'a> {
    type Item = Result<(Vec<u8>, &'a [u8]), DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.core
            .next_raw()
            .map(|r| r.map(|(k, e)| (k.to_vec(), e.value)))
    }
}

/// Internal iteration: newest entry per key including tombstones.
/// This is the stream compaction will consume.
pub struct DbRawIter<'a> {
    core: MergeCore<'a>,
}

impl<'a> Iterator for DbRawIter<'a> {
    type Item = Result<(Vec<u8>, Kind, &'a [u8]), DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.core
            .next_raw()
            .map(|r| r.map(|(k, e)| (k.to_vec(), e.kind, e.value)))
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    /// Builds a database across three generations with an operation log,
    /// and returns it plus the reference model of live data.
    fn build_three_generations(label: &str) -> (TempDir, Kiban, BTreeMap<Vec<u8>, Vec<u8>>) {
        let td = TempDir::new(label);
        let mut db = Kiban::open(td.path()).unwrap();
        let mut reference: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let apply =
            |db: &mut Kiban, r: &mut BTreeMap<Vec<u8>, Vec<u8>>, k: &[u8], v: Option<&[u8]>| {
                if let Some(v) = v {
                    db.put(k, v).unwrap();
                    r.insert(k.to_vec(), v.to_vec());
                } else {
                    db.delete(k).unwrap();
                    r.remove(k);
                }
            };

        for i in 0..300u32 {
            apply(
                &mut db,
                &mut reference,
                format!("key-{i:06}").as_bytes(),
                Some(format!("gen1-{i}").as_bytes()),
            );
        }
        db.sync().unwrap();
        db.flush().unwrap();

        // generation 2: overwrite evens, delete multiples of 10, add new keys
        for i in 0..300u32 {
            if i % 2 == 0 {
                apply(
                    &mut db,
                    &mut reference,
                    format!("key-{i:06}").as_bytes(),
                    Some(format!("gen2-{i}").as_bytes()),
                );
            }
            if i % 10 == 0 {
                apply(
                    &mut db,
                    &mut reference,
                    format!("key-{i:06}").as_bytes(),
                    None,
                );
            }
            apply(
                &mut db,
                &mut reference,
                format!("extra-{i:06}").as_bytes(),
                Some(format!("e{i}").as_bytes()),
            );
        }
        db.sync().unwrap();
        db.flush().unwrap();

        // generation 3 lives in the memtable
        for i in 0..50u32 {
            apply(
                &mut db,
                &mut reference,
                format!("key-{i:06}").as_bytes(),
                Some(format!("mem-{i}").as_bytes()),
            );
            apply(
                &mut db,
                &mut reference,
                format!("extra-{i:06}").as_bytes(),
                None,
            );
        }

        (td, db, reference)
    }

    #[test]
    fn full_scan_matches_reference_model_across_generations() {
        let (_td, db, reference) = build_three_generations("scan-full");
        let scanned: Vec<(Vec<u8>, Vec<u8>)> = db
            .iter()
            .map(|r| r.unwrap())
            .map(|(k, v)| (k, v.to_vec()))
            .collect();
        let expected: Vec<(Vec<u8>, Vec<u8>)> = reference.into_iter().collect();
        assert_eq!(scanned.len(), expected.len());
        assert_eq!(scanned, expected);
    }

    #[test]
    fn point_gets_agree_with_scan_contents() {
        let (_td, db, _reference) = build_three_generations("scan-agrees");
        let from_scan: BTreeMap<Vec<u8>, Vec<u8>> = db
            .iter()
            .map(|r| r.unwrap())
            .map(|(k, v)| (k.clone(), v.to_vec()))
            .collect();
        // every scan entry must be retrievable by get with identical value
        for (k, v) in &from_scan {
            assert_eq!(
                db.get(k.as_slice()).unwrap().as_deref(),
                Some(v.as_slice()),
                "get disagrees with scan on {k:?}"
            );
        }
        // spot-check keys absent from the scan are truly gone
        assert_eq!(db.get(b"extra-000000").unwrap(), None); // deleted in memtable
        // and one re-added in the memtable beats its table tombstone
        assert_eq!(db.get(b"key-000000").unwrap(), Some(b"mem-0".to_vec()));
    }

    #[test]
    fn raw_iteration_exposes_tombstones_and_newest_only() {
        let (_td, db, _) = build_three_generations("scan-raw");
        let entries: Vec<(Vec<u8>, Kind)> = db
            .iter_internal()
            .map(|r| {
                let (k, kind, _) = r.unwrap();
                (k, kind)
            })
            .collect();

        // strictly ascending, unique keys
        for w in entries.windows(2) {
            assert!(w[0].0 < w[1].0);
        }
        // extra-000000 exists as a tombstone (written to a table in
        // gen2, deleted by the memtable in gen3)
        let zero = entries.iter().find(|(k, _)| k == b"extra-000000").unwrap();
        assert_eq!(zero.1, Kind::Tombstone);
        // key-000000's newest version is the memtable re-add
        let zero = entries.iter().find(|(k, _)| k == b"key-000000").unwrap();
        assert_eq!(zero.1, Kind::Put);
        // no duplicate keys anywhere
        assert_eq!(
            entries
                .iter()
                .map(|(k, _)| k)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            entries.len()
        );
    }

    #[test]
    fn seek_positions_at_first_key_ge_target() {
        let (_td, db, reference) = build_three_generations("scan-seek");
        for probe in ["", "a", "key-000049", "key-000050", "key-000123x", "zzzz"] {
            let got: Vec<Vec<u8>> = db
                .iter_from(probe.as_bytes())
                .map(|r| r.unwrap().0)
                .take(5)
                .collect();
            let want: Vec<Vec<u8>> = reference
                .range(probe.as_bytes().to_vec()..)
                .take(5)
                .map(|(k, _)| k.clone())
                .collect();
            assert_eq!(got, want, "seek from {probe:?}");
        }
    }

    #[test]
    fn range_is_half_open_subspace_of_full_scan() {
        let (_td, db, reference) = build_three_generations("scan-range");
        let ranged: Vec<Vec<u8>> = db
            .range(b"key-000100".as_slice(), b"key-000200".as_slice())
            .map(|r| r.unwrap().0)
            .collect();
        let expected: Vec<Vec<u8>> = reference
            .range(b"key-000100".to_vec()..b"key-000200".to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        assert_eq!(ranged, expected);

        // inverted/empty ranges yield nothing
        assert_eq!(db.range(b"z", b"a").count(), 0);
        assert_eq!(db.range(b"x", b"x").count(), 0);
    }
}

impl Kiban {
    fn l0_count(&self) -> usize {
        self.tables.iter().filter(|t| t.level == 0).count()
    }

    fn level_bytes(&self, level: u32) -> u64 {
        self.tables
            .iter()
            .filter(|t| t.level == level)
            .map(|t| t.size)
            .sum()
    }

    fn level_budget(&self, level: u32) -> Option<u64> {
        if level < 1 {
            return None;
        }
        self.options
            .base_level_bytes
            .checked_mul(self.options.level_multiplier.pow(level - 1))
    }

    /// Runs compactions the current state demands, synchronously and in
    /// a deterministic order (compaction.md D3).
    fn maybe_compact(&mut self) -> Result<(), DbError> {
        while self.l0_count() >= self.options.l0_compaction_trigger {
            self.compact_level(0)?;
        }
        let mut level = 1;
        loop {
            match self.level_budget(level) {
                Some(budget) if self.level_bytes(level) > budget => {
                    // nothing to compact from an empty/missing level
                    if self.tables.iter().any(|t| t.level == level) {
                        self.compact_level(level)?;
                    } else {
                        break;
                    }
                    level += 1;
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Compacts one level into the next, per compaction.md D3-D6.
    fn compact_level(&mut self, level: u32) -> Result<(), DbError> {
        debug_assert!(self.tables.iter().any(|t| t.level == level));

        // choose inputs (compaction.md D3)
        let mut input_indices: Vec<usize> = Vec::new();
        let range_lo: Vec<u8>;
        let range_hi: Vec<u8>;
        if level == 0 {
            for (i, t) in self.tables.iter().enumerate() {
                if t.level == 0 {
                    input_indices.push(i);
                }
            }
            range_lo = input_indices
                .iter()
                .map(|i| self.tables[*i].first_key.clone())
                .min()
                .expect("level 0 nonempty");
            range_hi = input_indices
                .iter()
                .map(|i| self.tables[*i].last_key.clone())
                .max()
                .expect("level 0 nonempty");
        } else {
            let seed = self
                .tables
                .iter()
                .enumerate()
                .filter(|(_, t)| t.level == level)
                .min_by_key(|(_, t)| t.number)
                .expect("seed exists");
            input_indices.push(seed.0);
            range_lo = seed.1.first_key.clone();
            range_hi = seed.1.last_key.clone();
        }
        let target = level + 1;
        for (i, t) in self.tables.iter().enumerate() {
            if t.level == target && t.first_key <= range_hi && t.last_key >= range_lo {
                input_indices.push(i);
            }
        }
        input_indices.sort();

        let deepest = self.tables.iter().map(|t| t.level).max().unwrap_or(0);
        // tombstone GC is legal only when no level deeper than the target
        // exists and all target overlaps are inputs (compaction.md D5)
        let gc_allowed = target > deepest;

        // merge inputs newest-first; collapse via raw mode.
        // Newest = shallower level first; within a level, higher file
        // number first. (Getting this backwards lets stale deep-level
        // values resurrect over fresh ones.)
        let mut ordered: Vec<&TableEntry> =
            input_indices.iter().map(|i| &self.tables[*i]).collect();
        ordered.sort_by(|a, b| a.level.cmp(&b.level).then(b.number.cmp(&a.number)));

        let mut outputs: Vec<TableEntry> = Vec::new();
        let mut builder = TableBuilder::new();
        let mut output_entries = 0usize;
        let mut core = MergeCore {
            sources: ordered
                .iter()
                .map(|t| SourceHead {
                    feed: SourceFeed::Table(t.table.iter_from(b"")),
                    head: None,
                    exhausted: false,
                })
                .collect(),
            user_mode: false,
            failed: false,
        };
        let emit_output = |dir: &Path,
                           builder: TableBuilder,
                           number: u64,
                           outputs: &mut Vec<TableEntry>|
         -> Result<(), DbError> {
            let bytes = builder.finish()?;
            atomic::commit_file(&dir.join(file_name(number, SST_EXTENSION)), &bytes)?;
            let size = bytes.len() as u64;
            let table = SstTable::parse(bytes)?;
            let entry = TableEntry {
                level: target,
                number,
                size,
                first_key: table.smallest_key()?,
                last_key: table.largest_key()?,
                table,
            };
            outputs.push(entry);
            Ok(())
        };

        let mut next_number = self.next_file_number;
        while let Some(item) = core.next_raw() {
            let (key, raw) = item?;
            if gc_allowed && raw.kind == Kind::Tombstone {
                continue;
            }
            if output_entries > 0
                && builder.approximate_size() >= self.options.target_file_size as usize
            {
                let number = next_number;
                next_number += 1;
                emit_output(&self.dir, builder, number, &mut outputs)?;
                builder = TableBuilder::new();
                output_entries = 0;
            }
            builder.add(raw.kind, &key, raw.value)?;
            output_entries += 1;
        }

        if output_entries > 0 {
            let number = next_number;
            next_number += 1;
            emit_output(&self.dir, builder, number, &mut outputs)?;
        }

        // D6 step 4: the commit point — outputs in, inputs out.
        let input_refs: Vec<TableRef> = input_indices
            .iter()
            .map(|i| TableRef {
                level: self.tables[*i].level,
                number: self.tables[*i].number,
            })
            .collect();
        let mut new_tables: Vec<TableRef> = self
            .tables
            .iter()
            .enumerate()
            .filter(|(i, _)| !input_indices.contains(i))
            .map(|(_, t)| TableRef {
                level: t.level,
                number: t.number,
            })
            .collect();
        new_tables.extend(outputs.iter().map(|o| TableRef {
            level: o.level,
            number: o.number,
        }));
        new_tables.sort();
        Manifest {
            next_file_number: next_number,
            wal_number: self.wal_number,
            tables: new_tables,
        }
        .install(&self.dir)
        .map_err(DbError::from)?;

        // post-commit: swap in-memory state, retire inputs
        self.next_file_number = next_number;
        let removed: std::collections::HashSet<u64> = input_refs.iter().map(|r| r.number).collect();
        self.tables.retain(|t| !removed.contains(&t.number));
        for out in outputs {
            let pos = self
                .tables
                .partition_point(|t| (t.level, t.number) < (out.level, out.number));
            self.tables.insert(pos, out);
        }
        for r in &input_refs {
            let _ = sys::remove_file(&self.dir.join(file_name(r.number, SST_EXTENSION)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod compaction_tests {
    #[test]
    fn reopening_every_round_stays_valid_and_correct() {
        let td = crate::testutil::TempDir::new("bisect");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        let mut reference: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..60u64 {
            for _ in 0..12 {
                let i = next() % 80;
                let key = format!("k{i:03}");
                if next() % 5 == 0 {
                    db.delete(key.as_bytes()).unwrap();
                    reference.remove(key.as_bytes());
                } else {
                    let val = format!("r{round}-i{i}");
                    db.put(key.as_bytes(), val.as_bytes()).unwrap();
                    reference.insert(key.into_bytes(), val.into_bytes());
                }
            }
            db.sync().unwrap();
            db.flush().unwrap();
            drop(db);
            db = match Kiban::open_with_options(td.path(), tiny_options()) {
                Ok(d) => d,
                Err(e) => {
                    println!("REOPEN FAILED at round {round}: {e}");
                    panic!("reopen failed");
                }
            };
            let scanned: Vec<(Vec<u8>, Vec<u8>)> = db
                .iter()
                .map(|r| r.unwrap())
                .map(|(k, v)| (k.clone(), v.to_vec()))
                .collect();
            let expected: Vec<(Vec<u8>, Vec<u8>)> = reference.clone().into_iter().collect();
            if scanned != expected {
                println!(
                    "DIVERGED at round {round}: tables={:?} scan={} ref={}",
                    db.debug_tables(),
                    scanned.len(),
                    expected.len()
                );
                panic!("diverged");
            }
        }
    }

    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    fn tiny_options() -> KibanOptions {
        KibanOptions {
            l0_compaction_trigger: 2,
            base_level_bytes: 300,
            level_multiplier: 4,
            target_file_size: 250,
        }
    }

    #[test]
    fn compactions_keep_reads_and_scans_correct_over_many_generations() {
        let td = TempDir::new("compact-longrun");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        let mut reference: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        // deterministic pseudo-random workload
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for round in 0..60u64 {
            for _ in 0..12 {
                let i = next() % 80;
                let key = format!("k{i:03}");
                if next() % 5 == 0 {
                    db.delete(key.as_bytes()).unwrap();
                    reference.remove(key.as_bytes());
                } else {
                    let val = format!("r{round}-i{i}");
                    db.put(key.as_bytes(), val.as_bytes()).unwrap();
                    reference.insert(key.into_bytes(), val.into_bytes());
                }
            }
            db.sync().unwrap();
            db.flush().unwrap();
        }

        // reference equality through everything compaction did
        let scanned: Vec<(Vec<u8>, Vec<u8>)> = db
            .iter()
            .map(|r| r.unwrap())
            .map(|(k, v)| (k, v.to_vec()))
            .collect();
        assert_eq!(scanned, reference.clone().into_iter().collect::<Vec<_>>());

        // per-key get agreement
        for (k, v) in &reference {
            assert_eq!(db.get(k.as_slice()).unwrap().as_deref(), Some(v.as_slice()));
        }

        // bounded structure: L0 stays short, deeper levels exist
        assert!(db.l0_count() < 2);
        assert!(db.tables.iter().any(|t| t.level >= 2));

        // invariant survives reopen (open re-validates disjointness)
        drop(db);
        let db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        let rescanned: Vec<(Vec<u8>, Vec<u8>)> = db
            .iter()
            .map(|r| r.unwrap())
            .map(|(k, v)| (k, v.to_vec()))
            .collect();
        assert_eq!(rescanned, scanned);
    }

    #[test]
    fn tombstones_are_fully_reclaimed_at_the_deepest_level() {
        let td = TempDir::new("compact-gc");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        db.put(b"doomed", b"value").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        db.delete(b"doomed").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // triggers L0->L1; L1 may be deepest -> GC legal

        // keep pushing data until the tombstone either reaches the deepest
        // level and vanishes or provably remains shadowed correctly
        for round in 0..10 {
            db.put(format!("filler{round}"), b"x").unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }

        // get agrees regardless
        assert_eq!(db.get(b"doomed").unwrap(), None);

        // eventually no trace: neither value nor tombstone in any table
        let raw_keys: Vec<Vec<u8>> = db.iter_internal().map(|r| r.unwrap().0).collect();
        assert!(
            !raw_keys.contains(&b"doomed".to_vec()),
            "doomed key still physically present after deep compaction"
        );

        // and the deleted VALUE bytes are gone from every file on disk
        for entry in fs::read_dir(td.path()).unwrap().flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.ends_with(SST_EXTENSION) {
                let bytes = fs::read(entry.path()).unwrap();
                assert!(
                    !bytes.windows(5).any(|w| w == b"value"),
                    "deleted value still on disk in {name}"
                );
            }
        }
    }

    #[test]
    fn compaction_retires_inputs_and_never_leaves_garbage_in_manifest() {
        let td = TempDir::new("compact-retire");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        for round in 0..6u32 {
            db.put(format!("key{round}"), b"v").unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }
        let manifest = Manifest::load(td.path()).unwrap().unwrap();

        // every referenced file exists, every existing sst is referenced
        for tref in &manifest.tables {
            assert!(
                td.path()
                    .join(file_name(tref.number, SST_EXTENSION))
                    .exists()
            );
        }
        let on_disk: Vec<u64> = fs::read_dir(td.path())
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_str()?.to_string();
                n.strip_suffix(".sst").and_then(|s| s.parse().ok())
            })
            .collect();
        assert_eq!(on_disk.len(), manifest.tables.len());

        // compaction actually happened: more flushes than surviving tables
        assert!(manifest.tables.len() < 6);
        assert!(db.get(b"key5").unwrap().is_some());
    }
}

#[cfg(test)]
impl Kiban {
    pub(crate) fn debug_tables(&self) -> Vec<(u32, u64)> {
        self.tables.iter().map(|t| (t.level, t.number)).collect()
    }
}
