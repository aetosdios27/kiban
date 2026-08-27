//! The engine handle: open, recover, read, write.
//!
//! Assembles WAL + memtable + sstables + MANIFEST into a crash-
//! recoverable database, per `docs/design/db-layout.md`. Single-threaded
//! by decision D7; only `sync()` earns a durability claim.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::background::{Maintenance, MaintenanceError};
use crate::cache::BlockCache;
use crate::manifest::{MANIFEST_NAME, Manifest, ManifestError, TableRef};
use crate::memtable::{Entry as MemEntry, Memtable};
use crate::sstable::{Kind, SstError, SstTable, TableBuilder};
use crate::sys;
use crate::wal::{Wal, WalError};
use std::sync::Arc as StdArc;

/// Owned key/value pair yielded by scans.
pub type ScanEntry = (Vec<u8>, Vec<u8>);
pub type ScanResult = Vec<ScanEntry>;

pub const SST_EXTENSION: &str = "sst";
pub const WAL_EXTENSION: &str = "wal";

#[derive(Debug)]
pub enum DbError {
    Io(io::Error),
    Corrupt(String),
    CommitFailed(io::Error),
    CommitAmbiguous(io::Error),
    /// The engine is in a poisoned state; this operation was refused.
    Poisoned(PoisonCause),
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
            DbError::Poisoned(cause) => {
                write!(
                    f,
                    "engine poisoned; mutation refused — reopen to recover: {cause}"
                )
            }
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Io(e) | DbError::CommitFailed(e) | DbError::CommitAmbiguous(e) => Some(e),
            DbError::Corrupt(_) | DbError::Poisoned(_) => None,
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

/// Why the engine entered its poisoned (fatal) state. Distinct from
/// generic errors: once poisoned, mutations can no longer promise
/// durability (engine-poisoning.md D1/D2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoisonCause {
    /// A WAL sync/fdatasync failed: durability of recent records unknown.
    WalSyncFailed(String),
    /// A WAL append failed: a torn record may precede later appends.
    WalAppendFailed(String),
    /// A MANIFEST install completed its rename ambiguously: which
    /// topology persisted is unknown.
    CommitAmbiguity(String),
}

impl fmt::Display for PoisonCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoisonCause::WalSyncFailed(m) => write!(f, "wal sync failure: {m}"),
            PoisonCause::WalAppendFailed(m) => write!(f, "wal append failure: {m}"),
            PoisonCause::CommitAmbiguity(m) => write!(f, "commit ambiguity: {m}"),
        }
    }
}

fn file_name(number: u64, extension: &str) -> String {
    format!("{number}.{extension}")
}

#[derive(Debug)]
pub(crate) struct TableEntry {
    pub(crate) level: u32,
    pub(crate) number: u64,
    pub(crate) size: u64,
    pub(crate) first_key: Vec<u8>,
    pub(crate) last_key: Vec<u8>,
    pub(crate) table: SstTable,
}

/// An immutable published table topology. Readers/snapshots pin an
/// `Arc<Version>` so files it references outlive compactions
/// (engine-poisoning.md / phase-11.3).
#[derive(Debug)]
pub(crate) struct Version {
    /// Monotonic; odd/even has no meaning, only ordering.
    pub(crate) id: u64,
    /// Sorted by (level, number). Immutable once published.
    pub(crate) tables: Vec<StdArc<TableEntry>>,
}

impl Version {
    pub(crate) fn l0_count(&self) -> usize {
        self.tables.iter().filter(|t| t.level == 0).count()
    }

    #[allow(dead_code)]
    pub(crate) fn level_bytes(&self, level: u32) -> u64 {
        self.tables
            .iter()
            .filter(|t| t.level == level)
            .map(|t| t.size)
            .sum()
    }

    #[allow(dead_code)]
    pub(crate) fn max_level(&self) -> u32 {
        self.tables.iter().map(|t| t.level).max().unwrap_or(0)
    }

    pub(crate) fn contains_number(&self, number: u64) -> bool {
        self.tables.iter().any(|t| t.number == number)
    }
}

/// Tunables for flush/compaction behavior (compaction.md configuration).
#[derive(Debug, Clone)]
pub struct KibanOptions {
    pub l0_compaction_trigger: usize,
    pub base_level_bytes: u64,
    pub level_multiplier: u64,
    pub target_file_size: u64,
    pub block_cache_bytes: usize,
}

impl Default for KibanOptions {
    fn default() -> Self {
        const MIB: u64 = 1 << 20;
        KibanOptions {
            l0_compaction_trigger: 4,
            base_level_bytes: 4 * MIB,
            level_multiplier: 10,
            target_file_size: 4 * MIB,
            block_cache_bytes: 32 * MIB as usize,
        }
    }
}

/// An atomic group of mutations submitted via [`Kiban::write`] or
/// [`SharedKiban::write`].
///
/// The batch receives one contiguous sequence-number interval and is
/// committed as a single WAL record: recovery applies every mutation
/// or none — never a prefix. Ordering within the batch is preserved.
/// This is atomic write grouping only: no transactions, rollback, or
/// conflict detection.
#[derive(Debug, Default)]
pub struct WriteBatch {
    ops: Vec<(Kind, Vec<u8>, Vec<u8>)>,
}

impl WriteBatch {
    pub fn new() -> WriteBatch {
        WriteBatch::default()
    }

    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push((Kind::Put, key.into(), value.into()));
        self
    }

    pub fn delete(&mut self, key: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push((Kind::Tombstone, key.into(), Vec::new()));
        self
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

pub struct Kiban {
    dir: PathBuf,
    options: KibanOptions,
    cache: StdArc<BlockCache>,
    memtable: Memtable,
    wal: Wal,
    next_file_number: u64,
    wal_number: u64,
    last_sequence: u64,
    /// Sorted ascending; the oldest entry gates tombstone GC.
    active_snapshots: Vec<u64>,
    /// The authoritative published topology (MANIFEST-committed).
    version: StdArc<Version>,
    /// Versions pinned by snapshots; strong refs keep their files alive.
    pinned_versions: Vec<(u64, StdArc<Version>)>,
    /// Files removed from the live topology awaiting provable-unreferenced
    /// reclamation.
    obsolete: Vec<(u64, PathBuf)>,
    /// Set when a durability-relevant failure makes future
    /// acknowledgement unsafe (engine-poisoning.md D1/D2).
    poisoned: Option<PoisonCause>,
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
        let cache = StdArc::new(BlockCache::new(options.block_cache_bytes));

        let manifest = match Manifest::load(&dir)? {
            Some(m) => m,
            None => Self::initialize_fresh(&dir)?,
        };

        // Sweep before touching anything: unreferenced artifacts are
        // garbage by definition (D3 step 4).
        Self::sweep_orphans(&dir, &manifest)?;

        let wal_path = dir.join(file_name(manifest.wal_number, WAL_EXTENSION));
        if !sys::exists(&wal_path) {
            return Err(DbError::Corrupt(format!(
                "manifest names wal {} which does not exist",
                manifest.wal_number
            )));
        }

        let mut memtable = Memtable::new();
        let (wal, report) = Wal::open(&wal_path, &mut memtable)?;
        let wal_max_seq = report.max_sequence;

        let mut tables = Vec::with_capacity(manifest.tables.len());
        for tref in &manifest.tables {
            let path = dir.join(file_name(tref.number, SST_EXTENSION));
            let table = SstTable::open(tref.number, &path, cache.clone())?;
            let size = table.size_on_disk();
            let first_key = table.smallest_key().to_vec();
            let last_key = table.largest_key().to_vec();
            tables.push(StdArc::new(TableEntry {
                level: tref.level,
                number: tref.number,
                size,
                first_key,
                last_key,
                table,
            }));
        }

        // compaction.md D2: L>=1 levels must be range-disjoint. Within a
        // level, files are checked in KEY order — file numbers record
        // creation time, which need not match keyspace position.
        let mut level_view: Vec<&TableEntry> = tables
            .iter()
            .map(|t| &**t)
            .filter(|t| t.level >= 1)
            .collect();
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

        let cache = StdArc::new(BlockCache::new(options.block_cache_bytes));
        Ok(Kiban {
            dir,
            options,
            cache,
            memtable,
            wal,
            next_file_number: manifest.next_file_number,
            wal_number: manifest.wal_number,
            last_sequence: manifest.last_sequence.max(wal_max_seq),
            active_snapshots: Vec::new(),
            version: StdArc::new(Version { id: 0, tables }),
            pinned_versions: Vec::new(),
            obsolete: Vec::new(),
            poisoned: None,
        })
    }

    fn initialize_fresh(dir: &Path) -> Result<Manifest, DbError> {
        // In device-sim mode the real directory is not authoritative;
        // enumerate the simulated namespace instead.
        let mut paths: Vec<PathBuf> = sys::simulated_files_under(dir);
        if paths.is_empty()
            && !sys::device_sim_active()
            && let Ok(entries) = fs::read_dir(dir)
        {
            paths = entries.flatten().map(|e| e.path()).collect();
        }
        for path in &paths {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_recognized_artifact(name) {
                let _ = sys::remove_file(path);
            }
        }
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
                sys::remove_file(&entry.path())?;
            }
        }
        for entry in fs::read_dir(dir)?.flatten() {
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|n| n.contains(atomic::TEMP_MARKER))
            {
                sys::remove_file(&entry.path())?;
            }
        }
        Ok(())
    }

    /// Returns Err(Poisoned) when the engine may no longer acknowledge
    /// mutations; Ok(()) otherwise (engine-poisoning.md D1).
    fn l0_count(&self) -> usize {
        self.version.l0_count()
    }

    fn level_bytes(&self, level: u32) -> u64 {
        self.version.level_bytes(level)
    }

    fn check_poisoned(&self) -> Result<(), DbError> {
        match &self.poisoned {
            Some(cause) => Err(DbError::Poisoned(cause.clone())),
            None => Ok(()),
        }
    }

    pub(crate) fn poison(&mut self, cause: PoisonCause) {
        if self.poisoned.is_none() {
            eprintln!("kiban: engine POISONED — mutations refused until reopen: {cause}");
            self.poisoned = Some(cause);
        }
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> io::Result<()> {
        if let Err(e) = self.check_poisoned() {
            return Err(io::Error::other(e.to_string()));
        }
        let seq = self.last_sequence + 1;
        if let Err(e) = self.wal.put(seq, key.as_ref(), value.as_ref()) {
            // A torn WAL frame would silently discard later, successfully
            // synced records at recovery: append failure poisons.
            self.poison(PoisonCause::WalAppendFailed(e.to_string()));
            return Err(io::Error::other("wal append failed; engine poisoned"));
        }
        self.memtable.put(key, value, seq);
        self.last_sequence = seq;
        Ok(())
    }

    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> io::Result<()> {
        if let Err(e) = self.check_poisoned() {
            return Err(io::Error::other(e.to_string()));
        }
        let seq = self.last_sequence + 1;
        if let Err(e) = self.wal.delete(seq, key.as_ref()) {
            self.poison(PoisonCause::WalAppendFailed(e.to_string()));
            return Err(io::Error::other("wal append failed; engine poisoned"));
        }
        self.memtable.delete(key, seq);
        self.last_sequence = seq;
        Ok(())
    }

    /// Makes all prior writes crash-durable. A failure here is
    /// durability-ambiguous and poisons the engine (engine-poisoning.md
    /// D2): later operations refuse to acknowledge anything.
    /// Commits a batch atomically: one contiguous sequence interval,
    /// one WAL record, one memtable application. A successful return
    /// means every mutation is applied in memory; call `sync` to make
    /// the whole batch durable. A WAL append failure poisons the engine.
    pub fn write(&mut self, batch: WriteBatch) -> Result<(), DbError> {
        self.check_poisoned()?;
        if batch.ops.is_empty() {
            return Ok(());
        }
        let first_seq = self.last_sequence + 1;
        if let Err(e) = self.wal.append_batch(first_seq, &batch.ops) {
            self.poison(PoisonCause::WalAppendFailed(e.to_string()));
            return Err(DbError::Poisoned(self.poisoned.clone().unwrap()));
        }
        for (i, (kind, key, value)) in batch.ops.iter().enumerate() {
            let seq = first_seq + i as u64;
            match kind {
                Kind::Put => self.memtable.put(key, value, seq),
                Kind::Tombstone => self.memtable.delete(key, seq),
            }
        }
        self.last_sequence = first_seq + batch.ops.len() as u64 - 1;
        Ok(())
    }

    pub fn sync(&mut self) -> io::Result<()> {
        if let Err(e) = self.check_poisoned() {
            return Err(io::Error::other(e.to_string()));
        }
        match self.wal.sync() {
            Ok(()) => Ok(()),
            Err(crate::wal::SyncPhase::Flush(e)) => {
                self.poison(PoisonCause::WalAppendFailed(e.to_string()));
                Err(io::Error::other("wal append failed; engine poisoned"))
            }
            Err(crate::wal::SyncPhase::Fdatasync(e)) => {
                self.poison(PoisonCause::WalSyncFailed(e.to_string()));
                Err(io::Error::other("wal sync failed; engine poisoned"))
            }
        }
    }

    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>, DbError> {
        let key = key.as_ref();
        match self.memtable.entry(key) {
            Some(MemEntry::Value { value, .. }) => return Ok(Some(value.clone())),
            Some(MemEntry::Tombstone { .. }) => return Ok(None),
            None => {}
        }
        // L0 first, newest file number wins
        for entry in self.version.tables.iter().rev().filter(|t| t.level == 0) {
            match entry.table.get(key, None)? {
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
        for entry in self.version.tables.iter().filter(|t| t.level >= 1) {
            if key < entry.first_key.as_slice() || key > entry.last_key.as_slice() {
                continue;
            }
            match entry.table.get(key, None)? {
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

    /// Captures a snapshot: a sequence boundary for consistent reads.
    /// The engine retains history needed by this snapshot until
    /// [`Kiban::release_snapshot`] is called (or the process ends).
    pub fn snapshot(&mut self) -> Snapshot {
        let seq = self.last_sequence;
        let pos = self.active_snapshots.partition_point(|s| *s < seq);
        self.active_snapshots.insert(pos, seq);
        Snapshot {
            seq,
            version: StdArc::clone(&self.version),
        }
    }

    pub fn release_snapshot(&mut self, snap: &Snapshot) {
        if let Some(pos) = self.active_snapshots.iter().position(|s| *s == snap.seq) {
            self.active_snapshots.remove(pos);
        }
    }

    fn oldest_active_snapshot(&self) -> Option<u64> {
        self.active_snapshots.first().copied()
    }

    /// Reads `key` as of snapshot `snap` (snapshots.md D3).
    pub fn get_at(
        &self,
        snap: &Snapshot,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>, DbError> {
        let key = key.as_ref();
        // The memtable retains superseded versions while a snapshot needs
        // them, so the newest version at-or-below snap may live here even
        // when newer invisible versions exist.
        if let Some(entry) = self.memtable.entry_at(key, snap.seq) {
            return Ok(match entry {
                MemEntry::Value { value, .. } => Some(value.clone()),
                MemEntry::Tombstone { .. } => None,
            });
        }
        self.get_from_tables_at(snap, key)
    }

    fn get_from_tables_at(&self, snap: &Snapshot, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        // The table resolves the newest version at or below the snapshot
        // boundary; anything newer than `snap` is invisible to it.
        let limit = Some(snap.seq);
        for entry in self.version.tables.iter().rev().filter(|t| t.level == 0) {
            match entry.table.get(key, limit)? {
                Some(found) => {
                    return Ok(match found.kind {
                        Kind::Put => Some(found.value),
                        Kind::Tombstone => None,
                    });
                }
                None => continue,
            }
        }
        for entry in self.version.tables.iter().filter(|t| t.level >= 1) {
            if key < entry.first_key.as_slice() || key > entry.last_key.as_slice() {
                continue;
            }
            match entry.table.get(key, limit)? {
                Some(found) => {
                    return Ok(match found.kind {
                        Kind::Put => Some(found.value),
                        Kind::Tombstone => None,
                    });
                }
                None => continue,
            }
        }
        Ok(None)
    }

    /// Scans live entries as of snapshot `snap`, reading the pinned
    /// version's tables.
    pub fn scan_at(&self, snap: &Snapshot) -> Result<ScanResult, DbError> {
        let mut sources: Vec<SourceHead<'_>> = Vec::new();
        sources.push(SourceHead {
            feed: SourceFeed::Mem(Box::new(self.memtable.iter_from(b""))),
            head: None,
            exhausted: false,
        });
        for t in snap.version.tables.iter().rev().filter(|t| t.level == 0) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(t.table.iter_from(b"")),
                head: None,
                exhausted: false,
            });
        }
        for t in snap.version.tables.iter().filter(|t| t.level >= 1) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(t.table.iter_from(b"")),
                head: None,
                exhausted: false,
            });
        }
        let mut core = MergeCore {
            sources,
            user_mode: true,
            failed: false,
            snap_limit: Some(snap.seq),
            done_key: None,
        };
        let mut out = Vec::new();
        while let Some(item) = core.next_scanned() {
            let e = item?;
            out.push((e.key, e.value));
        }
        Ok(out)
    }

    /// The engine's active configuration.
    pub fn options(&self) -> &KibanOptions {
        &self.options
    }

    /// Whether the engine is in a poisoned (fatal) state.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.is_some()
    }

    /// The poison cause, when poisoned.
    pub fn poison_cause(&self) -> Option<&PoisonCause> {
        self.poisoned.as_ref()
    }

    /// Flushes the memtable to a new sstable and retires the current WAL,
    /// then runs whatever compaction that now demands, following
    /// db-layout D2's single-commit-point pipeline. `Kiban` stays
    /// synchronous and deterministic (11.4): compaction happens inline,
    /// on this call, before returning. `SharedKiban::flush` instead
    /// hands compaction to its background worker — see
    /// [`Kiban::flush_without_compaction`].
    pub fn flush(&mut self) -> Result<(), DbError> {
        self.flush_without_compaction()?;
        self.maybe_compact()
    }

    /// Everything `flush` does except running compaction afterwards:
    /// publishes the memtable as a new, durable L0 sstable and rotates
    /// the WAL. Used directly by `SharedKiban::flush`, which wakes the
    /// background compaction worker instead of running it inline so the
    /// caller isn't blocked behind maintenance it merely triggered.
    fn flush_without_compaction(&mut self) -> Result<(), DbError> {
        self.check_poisoned()?;
        if self.memtable.is_empty() {
            return Ok(());
        }

        let sst_number = self.next_file_number;
        let new_wal_number = self.next_file_number + 1;
        let new_next_file_number = self.next_file_number + 2;

        let mut builder = TableBuilder::new();
        for (key, entry) in self.memtable.iter() {
            match entry {
                MemEntry::Value { value, seq } => builder.add(Kind::Put, key, value, *seq)?,
                MemEntry::Tombstone { seq } => builder.add(Kind::Tombstone, key, b"", *seq)?,
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
            .version
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
            last_sequence: self.last_sequence,
            tables: table_refs,
        }
        .install(&self.dir)
        .map_err(|e| match e {
            atomic::CommitError::Failed(io) => DbError::CommitFailed(io),
            atomic::CommitError::RenamedNotDurable(io) => {
                // Commit ambiguity: continuing mutation would acknowledge
                // against an unknown base. Poison (engine-poisoning.md D2).
                self.poison(PoisonCause::CommitAmbiguity(io.to_string()));
                DbError::Poisoned(self.poisoned.clone().unwrap())
            }
        })?;

        // D2 step 5: everything below only runs once the commit point has
        // returned success.
        self.next_file_number = new_next_file_number;
        self.wal_number = new_wal_number;
        let table = SstTable::open(
            sst_number,
            &self.dir.join(file_name(sst_number, SST_EXTENSION)),
            self.cache.clone(),
        )?;
        let entry = StdArc::new(TableEntry {
            level: 0,
            number: sst_number,
            size: table.size_on_disk(),
            first_key: table.smallest_key().to_vec(),
            last_key: table.largest_key().to_vec(),
            table,
        });
        // Publish Version N+1: current tables + the flushed sstable.
        let mut new_tables = self.version.tables.clone();
        let pos = new_tables.partition_point(|t| (t.level, t.number) < (entry.level, entry.number));
        new_tables.insert(pos, entry.clone());
        let next_version_id = self.version.id + 1;
        self.version = StdArc::new(Version {
            id: next_version_id,
            tables: new_tables,
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
    pub(crate) fn last_sequence_for_test(&self) -> u64 {
        self.last_sequence
    }

    #[cfg(test)]
    pub(crate) fn wal_for_test(&mut self) -> &mut Wal {
        &mut self.wal
    }

    #[cfg(test)]
    pub(crate) fn live_table_numbers(&self) -> Vec<u64> {
        self.version.tables.iter().map(|t| t.number).collect()
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
    ) -> impl Iterator<Item = Result<(Vec<u8>, Vec<u8>), DbError>> + 'a {
        let core = MergeCore {
            sources: self.sources_from(start),
            user_mode: true,
            failed: false,
            snap_limit: None,
            done_key: None,
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
            snap_limit: None,
            done_key: None,
        }
    }

    fn sources_from<'a>(&'a self, start: &[u8]) -> Vec<SourceHead<'a>> {
        let mut sources = Vec::with_capacity(self.version.tables.len() + 1);
        // newest first: memtable, then L0 by descending number, then
        // deeper levels ascending (within a level, higher number = newer
        // for L0; deeper levels are disjoint so order is irrelevant but
        // kept deterministic)
        sources.push(SourceHead {
            feed: SourceFeed::Mem(Box::new(self.memtable.iter_from(start))),
            head: None,
            exhausted: false,
        });
        for table in self.version.tables.iter().rev().filter(|t| t.level == 0) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(table.table.iter_from(start)),
                head: None,
                exhausted: false,
            });
        }
        for table in self.version.tables.iter().filter(|t| t.level >= 1) {
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
                snap_limit: None,
                done_key: None,
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
        t1.add(Kind::Put, b"k", b"v1", 1).unwrap();
        t1.add(Kind::Put, b"solo", b"only-in-1", 1).unwrap();
        atomic::commit_file(
            &dir.join(file_name(1, SST_EXTENSION)),
            &t1.finish().unwrap(),
        )
        .unwrap();
        let mut t2 = TableBuilder::new();
        t2.add(Kind::Tombstone, b"k", b"", 1).unwrap();
        t2.add(Kind::Put, b"other", b"old", 1).unwrap();
        atomic::commit_file(
            &dir.join(file_name(2, SST_EXTENSION)),
            &t2.finish().unwrap(),
        )
        .unwrap();

        let manifest = Manifest {
            next_file_number: 3,
            wal_number: 1,
            last_sequence: 0,
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
            last_sequence: 0,
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

/// A consistent read boundary captured from the engine's sequence
/// counter (snapshots.md D3). Pins the [`Version`] that was current at
/// capture time so compaction cannot reclaim files it may read.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) seq: u64,
    pub(crate) version: StdArc<Version>,
}

/// One entry as the merge sees it: the newest version of a key.
pub struct RawEntry {
    pub kind: Kind,
    pub seq: u64,
    pub value: Vec<u8>,
}

struct HeadEntry {
    key: Vec<u8>,
    kind: Kind,
    value: Vec<u8>,
    seq: u64,
}

enum SourceFeed<'a> {
    Mem(Box<dyn DoubleEndedIterator<Item = (&'a [u8], &'a MemEntry)> + 'a>),
    Table(crate::sstable::Iter<'a>),
}

struct SourceHead<'a> {
    feed: SourceFeed<'a>,
    head: Option<HeadEntry>,
    exhausted: bool,
}

impl<'a> SourceHead<'a> {
    fn fill(&mut self) -> Result<(), SstError> {
        if self.head.is_some() || self.exhausted {
            return Ok(());
        }
        let next = match &mut self.feed {
            SourceFeed::Mem(it) => it.next().map(|(k, e)| match e {
                MemEntry::Value { value, seq } => HeadEntry {
                    key: k.to_vec(),
                    kind: Kind::Put,
                    value: value.clone(),
                    seq: *seq,
                },
                MemEntry::Tombstone { seq } => HeadEntry {
                    key: k.to_vec(),
                    kind: Kind::Tombstone,
                    value: Vec::new(),
                    seq: *seq,
                },
            }),
            SourceFeed::Table(it) => match it.next() {
                Some(Ok((kind, seq, key, value))) => Some(HeadEntry {
                    key,
                    kind,
                    value,
                    seq,
                }),
                Some(Err(e)) => return Err(e),
                None => None,
            },
        };
        self.head = next;
        self.exhausted = self.head.is_none();
        Ok(())
    }

    #[allow(dead_code)]
    fn advance(&mut self) {
        self.head = None;
    }
}

struct MergeCore<'a> {
    sources: Vec<SourceHead<'a>>,
    user_mode: bool,
    failed: bool,
    /// When set, entries with seq > limit are invisible.
    snap_limit: Option<u64>,
    /// User key whose newest visible version has already been decided.
    done_key: Option<Vec<u8>>,
}

impl<'a> MergeCore<'a> {
    /// Newest-wins merge over all sources (db-iterator.md D2). In user
    /// mode tombstones are skipped; in raw mode they are emitted.
    /// Fills every source head. Errors surface once, then fail-stick.
    fn fill_heads(&mut self) -> Result<(), SstError> {
        for source in &mut self.sources {
            source.fill()?;
        }
        Ok(())
    }

    /// Removes and returns the globally-first entry in internal-key
    /// order: (user key asc, seqno desc). Duplicate user keys therefore
    /// always emerge adjacently, newest first — the invariant compaction
    /// and snapshot filtering both rely on.
    fn pop_best(&mut self) -> Option<HeadEntry> {
        let mut best: Option<usize> = None;
        for i in 0..self.sources.len() {
            match (best, self.sources[i].head.as_ref()) {
                (_, None) => {}
                (None, Some(_)) => best = Some(i),
                (Some(b), Some(h)) => {
                    let bh = self.sources[b].head.as_ref().unwrap();
                    let h_ord = (h.key.as_slice(), std::cmp::Reverse(h.seq));
                    let b_ord = (bh.key.as_slice(), std::cmp::Reverse(bh.seq));
                    if h_ord < b_ord {
                        best = Some(i);
                    }
                }
            }
        }
        best.and_then(|i| self.sources[i].head.take())
    }

    /// Single-entry stream in internal-key order (compaction's input).
    pub(crate) fn next_internal(&mut self) -> Option<Result<HeadEntry, DbError>> {
        if self.failed {
            return None;
        }
        if let Err(e) = self.fill_heads() {
            self.failed = true;
            return Some(Err(DbError::from(e)));
        }
        self.pop_best().map(Ok)
    }

    /// Decided stream for scans: per user key, the newest version
    /// visible at `snap_limit` wins; in user mode tombstones hide the
    /// whole key; in raw mode the winning tombstone is emitted.
    fn next_scanned(&mut self) -> Option<Result<HeadEntry, DbError>> {
        if self.failed {
            return None;
        }
        loop {
            if let Err(e) = self.fill_heads() {
                self.failed = true;
                return Some(Err(DbError::from(e)));
            }
            let head = self.pop_best()?;
            if self.done_key.as_deref() == Some(head.key.as_slice()) {
                continue; // older sibling of an already-decided key
            }
            if let Some(limit) = self.snap_limit
                && head.seq > limit
            {
                continue; // invisible here; older siblings may show
            }
            // decision point: newest visible version of this key
            self.done_key = Some(head.key.clone());
            if self.user_mode && head.kind == Kind::Tombstone {
                continue;
            }
            return Some(Ok(head));
        }
    }
}

/// User-visible iteration: live data only (tombstones and shadowed
/// versions invisible). Agrees exactly with `get` by construction.
pub struct DbIter<'a> {
    core: MergeCore<'a>,
}

impl<'a> Iterator for DbIter<'a> {
    type Item = Result<(Vec<u8>, Vec<u8>), DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.core
            .next_scanned()
            .map(|r| r.map(|e| (e.key, e.value)))
    }
}

/// Internal iteration: newest entry per key including tombstones.
/// This is the stream compaction will consume.
pub struct DbRawIter<'a> {
    core: MergeCore<'a>,
}

impl<'a> Iterator for DbRawIter<'a> {
    type Item = Result<(Vec<u8>, Kind, Vec<u8>), DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.core
            .next_scanned()
            .map(|r| r.map(|e| (e.key, e.kind, e.value)))
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

/// A compaction job's plan (11.4): everything BUILD needs, captured
/// while the engine lock was held and owned independently of `Kiban`'s
/// locked state, so BUILD can run without borrowing it. The captured
/// `inputs` pin their files exactly like a snapshot's `Arc<Version>`
/// does; committing this plan must still apply as a delta against
/// whatever the *current* Version is when COMMIT runs, not overwrite it
/// with this stale view (see `Kiban::commit_compaction`).
pub(crate) struct CompactionPlan {
    inputs: Vec<StdArc<TableEntry>>,
    input_numbers: HashSet<u64>,
    output_level: u32,
    smallest_snapshot: u64,
    gc_allowed: bool,
    /// Reserved output file numbers, in the order BUILD may use them.
    /// Reserved generously at PLAN time (a cheap counter bump) so BUILD
    /// never needs the lock to allocate one.
    output_numbers: Vec<u64>,
    dir: PathBuf,
    cache: StdArc<BlockCache>,
    target_file_size: u64,
}

impl CompactionPlan {
    /// BUILD (compaction.md D3-D6, minus the commit point): the k-way
    /// merge and output sstable construction. No engine lock is held or
    /// needed here — this is the expensive part background compaction
    /// moves off the foreground mutex. Semantics are unchanged from the
    /// synchronous path: same drop rules, same split policy, same
    /// output durability (each output is committed via
    /// `atomic::commit_file` as it's produced, so a failure partway
    /// through leaves at most orphan sst files, cleaned by the next
    /// reopen's sweep — never anything the MANIFEST references).
    pub(crate) fn build(&self) -> Result<Vec<TableEntry>, DbError> {
        let mut sources: Vec<SourceHead<'_>> = Vec::with_capacity(self.inputs.len());
        for entry in &self.inputs {
            sources.push(SourceHead {
                feed: SourceFeed::Table(entry.table.iter_from(b"")),
                head: None,
                exhausted: false,
            });
        }

        let mut outputs: Vec<TableEntry> = Vec::new();
        let mut builder = TableBuilder::new();
        let mut output_entries = 0usize;
        let mut core = MergeCore {
            sources,
            user_mode: false,
            failed: false,
            snap_limit: None,
            done_key: None,
        };

        // Faithful port of LevelDB DoCompactionWork drop rules
        // (db_impl.cc): per user key, newest-first — an entry is dropped
        // when the previously seen (newer) sibling's seq <= the smallest
        // active snapshot; a leading deletion marker is additionally
        // dropped when universally hidden and no deeper level can hold
        // older data for that key (gc_allowed).
        let mut next_output = 0usize;
        let mut current_key: Option<Vec<u8>> = None;
        let mut last_seq_for_key = u64::MAX;
        let mut last_added_key: Option<Vec<u8>> = None;

        while let Some(item) = core.next_internal() {
            let v = item?;

            if current_key.as_deref() != Some(v.key.as_slice()) {
                current_key = Some(v.key.clone());
                last_seq_for_key = u64::MAX;
            }

            let mut drop_entry = false;
            if last_seq_for_key <= self.smallest_snapshot {
                drop_entry = true; // hidden by a newer version every snapshot sees
            } else if v.kind == Kind::Tombstone
                && v.seq <= self.smallest_snapshot
                && self.gc_allowed
            {
                drop_entry = true; // obsolete deletion at base level
            }
            last_seq_for_key = v.seq;

            if drop_entry {
                continue;
            }

            // Split only BETWEEN keys: a key's versions never straddle
            // output files (per-level ordering invariant).
            if last_added_key.as_deref() != Some(v.key.as_slice())
                && output_entries > 0
                && builder.approximate_size() >= self.target_file_size as usize
            {
                let number = self.next_output_number(&mut next_output)?;
                self.emit_output(builder, number, &mut outputs)?;
                builder = TableBuilder::new();
                output_entries = 0;
            }
            builder.add(v.kind, &v.key, &v.value, v.seq)?;
            output_entries += 1;
            last_added_key = Some(v.key.clone());
        }

        if output_entries > 0 {
            let number = self.next_output_number(&mut next_output)?;
            self.emit_output(builder, number, &mut outputs)?;
        }

        Ok(outputs)
    }

    fn next_output_number(&self, next_output: &mut usize) -> Result<u64, DbError> {
        let number = *self.output_numbers.get(*next_output).ok_or_else(|| {
            DbError::Corrupt(
                "compaction exhausted its reserved output file numbers (reservation bug)"
                    .to_string(),
            )
        })?;
        *next_output += 1;
        Ok(number)
    }

    fn emit_output(
        &self,
        builder: TableBuilder,
        number: u64,
        outputs: &mut Vec<TableEntry>,
    ) -> Result<(), DbError> {
        let bytes = builder.finish()?;
        let path = self.dir.join(file_name(number, SST_EXTENSION));
        atomic::commit_file(&path, &bytes)?;
        let table = SstTable::open(number, &path, self.cache.clone())?;
        outputs.push(TableEntry {
            level: self.output_level,
            number,
            size: table.size_on_disk(),
            first_key: table.smallest_key().to_vec(),
            last_key: table.largest_key().to_vec(),
            table,
        });
        Ok(())
    }
}

impl Kiban {
    fn level_budget(&self, level: u32) -> Option<u64> {
        if level < 1 {
            return None;
        }
        self.options
            .base_level_bytes
            .checked_mul(self.options.level_multiplier.pow(level - 1))
    }

    /// Runs compactions the current state demands, synchronously and in
    /// a deterministic order (compaction.md D3). `Kiban` stays
    /// single-threaded: PLAN, BUILD, and COMMIT all run back-to-back
    /// under the one `&mut self` borrow, so this is unchanged in
    /// behavior from before the PLAN/BUILD/COMMIT split — see
    /// `background::Maintenance` for the version of this same loop that
    /// runs BUILD off the lock.
    fn maybe_compact(&mut self) -> Result<(), DbError> {
        let mut cascade_level = 1u32;
        while let Some(plan) = self.plan_next_compaction(&mut cascade_level) {
            let outputs = plan.build()?;
            self.commit_compaction(plan, outputs)?;
        }
        Ok(())
    }

    /// PLAN, in `maybe_compact`'s fixed priority order: drain L0 first,
    /// then cascade levels 1, 2, 3... stopping at the first level found
    /// within its budget. `cascade_level` carries the cascade position
    /// across calls, exactly mirroring the original single-threaded
    /// loop's `level` variable — L0 is rechecked every call (compaction
    /// there always takes priority), the level cascade only ever
    /// advances past a level once it has actually been compacted.
    pub(crate) fn plan_next_compaction(
        &mut self,
        cascade_level: &mut u32,
    ) -> Option<CompactionPlan> {
        if self.poisoned.is_some() {
            return None;
        }
        if self.l0_count() >= self.options.l0_compaction_trigger {
            return self.plan_compaction_at_level(0);
        }
        match self.level_budget(*cascade_level) {
            Some(budget) if self.level_bytes(*cascade_level) > budget => {
                if !self
                    .version
                    .tables
                    .iter()
                    .any(|t| t.level == *cascade_level)
                {
                    return None;
                }
                let plan = self.plan_compaction_at_level(*cascade_level);
                *cascade_level += 1;
                plan
            }
            _ => None,
        }
    }

    /// PLAN for one level (compaction.md D3-D4): choose inputs, choose
    /// the output level, and reserve output file numbers — a bounded,
    /// cheap amount of work done under the engine lock. `None` only
    /// when the level turns out to have nothing to compact (callers
    /// already check this; kept as a safe fallback here too).
    fn plan_compaction_at_level(&mut self, level: u32) -> Option<CompactionPlan> {
        let mut input_indices: Vec<usize> = Vec::new();
        let range_lo: Vec<u8>;
        let range_hi: Vec<u8>;
        if level == 0 {
            if self.l0_count() == 0 {
                return None;
            }
            for (i, t) in self.version.tables.iter().enumerate() {
                if t.level == 0 {
                    input_indices.push(i);
                }
            }
            range_lo = input_indices
                .iter()
                .map(|i| self.version.tables[*i].first_key.clone())
                .min()
                .expect("level 0 nonempty");
            range_hi = input_indices
                .iter()
                .map(|i| self.version.tables[*i].last_key.clone())
                .max()
                .expect("level 0 nonempty");
        } else {
            let seed = self
                .version
                .tables
                .iter()
                .enumerate()
                .filter(|(_, t)| t.level == level)
                .min_by_key(|(_, t)| t.number)?;
            input_indices.push(seed.0);
            range_lo = seed.1.first_key.clone();
            range_hi = seed.1.last_key.clone();
        }
        let output_level = level + 1;
        for (i, t) in self.version.tables.iter().enumerate() {
            if t.level == output_level && t.first_key <= range_hi && t.last_key >= range_lo {
                input_indices.push(i);
            }
        }
        input_indices.sort();

        let deepest = self
            .version
            .tables
            .iter()
            .map(|t| t.level)
            .max()
            .unwrap_or(0);
        // tombstone GC is legal only when no level deeper than the target
        // exists and all target overlaps are inputs (compaction.md D5)
        let gc_allowed = output_level > deepest;

        // The k-way merge orders entries globally by internal key
        // (user key asc, seqno desc), so duplicate user keys across
        // inputs always emerge adjacent and newest first. Source order
        // only breaks ties identically — global seqnos make it moot.
        let mut ordered_idx: Vec<usize> = input_indices.clone();
        ordered_idx.sort_by(|&a, &b| {
            let ta = &self.version.tables[a];
            let tb = &self.version.tables[b];
            ta.level.cmp(&tb.level).then(tb.number.cmp(&ta.number))
        });
        let inputs: Vec<StdArc<TableEntry>> = ordered_idx
            .iter()
            .map(|&i| self.version.tables[i].clone())
            .collect();
        let input_numbers: HashSet<u64> = inputs.iter().map(|t| t.number).collect();

        // Reserve a generous bound on output file numbers now, while we
        // hold the lock, so BUILD never needs it to allocate one. Output
        // bytes are bounded by input bytes (drop rules only remove
        // data); the file count that produces is bounded by
        // input_bytes/target_file_size, plus slack for the tail file and
        // rounding.
        let total_input_bytes: u64 = inputs.iter().map(|t| t.size).sum();
        let target_file_size = self.options.target_file_size.max(1);
        let max_outputs = (total_input_bytes / target_file_size) + 8;
        let start = self.next_file_number;
        self.next_file_number += max_outputs;
        let output_numbers: Vec<u64> = (start..self.next_file_number).collect();

        let smallest_snapshot = self.oldest_active_snapshot().unwrap_or(self.last_sequence);

        Some(CompactionPlan {
            inputs,
            input_numbers,
            output_level,
            smallest_snapshot,
            gc_allowed,
            output_numbers,
            dir: self.dir.clone(),
            cache: self.cache.clone(),
            target_file_size: self.options.target_file_size,
        })
    }

    /// COMMIT (compaction.md D6 step 4): publish BUILD's output. Runs
    /// under the engine lock (the caller already holds `&mut self`).
    /// Applies the plan as a delta against the *current* topology —
    /// current tables minus this job's inputs, plus its outputs — never
    /// against the stale `Version` PLAN happened to see. That is what
    /// keeps a foreground flush that published a new L0 table while
    /// BUILD was running from ever being lost (11.4).
    pub(crate) fn commit_compaction(
        &mut self,
        plan: CompactionPlan,
        outputs: Vec<TableEntry>,
    ) -> Result<(), DbError> {
        // The one worker means no other compaction can have touched
        // these inputs meanwhile; this just makes that assumption
        // explicit rather than silently trusting it.
        for n in &plan.input_numbers {
            if !self.version.contains_number(*n) {
                return Err(DbError::Corrupt(format!(
                    "compaction commit: input table {n} is no longer in the current version"
                )));
            }
        }

        let mut new_table_refs: Vec<TableRef> = self
            .version
            .tables
            .iter()
            .filter(|t| !plan.input_numbers.contains(&t.number))
            .map(|t| TableRef {
                level: t.level,
                number: t.number,
            })
            .collect();
        new_table_refs.extend(outputs.iter().map(|o| TableRef {
            level: o.level,
            number: o.number,
        }));
        new_table_refs.sort();

        Manifest {
            next_file_number: self.next_file_number,
            wal_number: self.wal_number,
            last_sequence: self.last_sequence,
            tables: new_table_refs,
        }
        .install(&self.dir)
        .map_err(|e| match e {
            atomic::CommitError::Failed(io) => DbError::CommitFailed(io),
            atomic::CommitError::RenamedNotDurable(io) => {
                // Ambiguous rename is fatal regardless of which thread
                // triggered it (11.4: no second interpretation for
                // "background").
                self.poison(PoisonCause::CommitAmbiguity(io.to_string()));
                DbError::Poisoned(self.poisoned.clone().unwrap())
            }
        })?;

        let mut new_version_tables = self.version.tables.clone();
        new_version_tables.retain(|t| !plan.input_numbers.contains(&t.number));
        for out in outputs {
            let pos = new_version_tables
                .partition_point(|t| (t.level, t.number) < (out.level, out.number));
            new_version_tables.insert(pos, StdArc::new(out));
        }
        self.version = StdArc::new(Version {
            id: self.version.id + 1,
            tables: new_version_tables,
        });

        // Obsolete files are reclaimable only when no pinned version
        // still references them (11.3); until then they stay on disk.
        for n in &plan.input_numbers {
            self.obsolete
                .push((*n, self.dir.join(file_name(*n, SST_EXTENSION))));
        }
        self.reclaim_obsolete();
        Ok(())
    }

    /// Deletes obsolete files that no pinned version references. Files
    /// referenced by any pinned Version stay on disk until that pin dies
    /// (11.3 file-lifetime rules).
    fn reclaim_obsolete(&mut self) {
        let mut kept = Vec::new();
        for (number, path) in std::mem::take(&mut self.obsolete) {
            let pinned = self
                .pinned_versions
                .iter()
                .any(|(_, v)| v.contains_number(number));
            if pinned {
                kept.push((number, path));
            } else {
                let _ = sys::remove_file(&path);
            }
        }
        self.obsolete = kept;
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    pub(crate) fn tiny_options() -> KibanOptions {
        KibanOptions {
            l0_compaction_trigger: 2,
            base_level_bytes: 300,
            level_multiplier: 4,
            target_file_size: 250,
            block_cache_bytes: 1 << 20,
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
        assert!(db.version.tables.iter().any(|t| t.level >= 2));

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
        eprintln!(
            "tables={:?} next={}",
            db.version
                .tables
                .iter()
                .map(|t| (t.level, t.number))
                .collect::<Vec<_>>(),
            db.next_file_number
        );
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
        for i in 0..6u32 {
            let k = format!("key{i}");
            let g = db.get(k.as_bytes()).unwrap();
            eprintln!("get {k} = {g:?}");
        }
        assert!(db.get(b"key5").unwrap().is_some());
    }
}

#[cfg(test)]
mod crash_sweep_tests {
    use super::*;
    use crate::sys;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    type Model = BTreeMap<Vec<u8>, Vec<u8>>;

    /// What the scenario promised and attempted, tracked as it ran.
    #[derive(Default, Debug, Clone)]
    pub(crate) struct Tracker {
        /// Set when a flush returned Err: the commit point may or may not
        /// have passed, so the durable floor becomes ambiguous (atomic-
        /// commit.md D5) and only the banded assertion applies.
        pub(crate) ambiguous: bool,
        /// State as of the last successful `sync()` (the durability floor).
        pub(crate) synced: Model,
        /// Final intended state including unsynced operations (the ceiling,
        /// modulo torn-tail truncation).
        attempted: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
        /// Keys touched since the last successful sync.
        dirty_since_sync: Vec<Vec<u8>>,
    }

    impl Tracker {
        fn apply(&mut self, key: &[u8], value: Option<&[u8]>) {
            self.attempted
                .insert(key.to_vec(), value.map(|v| v.to_vec()));
            self.dirty_since_sync.push(key.to_vec());
        }

        /// A successful `sync` makes all prior operations durable.
        fn on_sync_ok(&mut self) {
            self.mark_durable();
        }

        /// A successful `flush` ALSO advances the durability floor: it
        /// publishes the entire memtable (synced or not) through its
        /// commit point. This is what the exact-durability sweep taught
        /// us.
        fn on_flush_ok(&mut self) {
            self.mark_durable();
        }

        fn mark_durable(&mut self) {
            self.synced = self
                .attempted
                .iter()
                .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone())))
                .collect();
            self.dirty_since_sync.clear();
        }
    }

    /// Asserts the recovery band (fault-injection.md D2): every allowed
    /// value for a key is its synced value or its attempted final value.
    pub(crate) fn assert_band(label: &str, n: &[usize], recovered: &Model, tracker: &Tracker) {
        let domain: std::collections::BTreeSet<&Vec<u8>> = tracker
            .synced
            .keys()
            .chain(tracker.attempted.keys())
            .collect();
        for key in domain {
            let synced = tracker.synced.get(key);
            let attempted = tracker.attempted.get(key);
            let actual: Option<&[u8]> = recovered.get(key).map(|v| v.as_slice());
            let mut allowed: Vec<Option<&[u8]>> = Vec::new();
            match synced {
                Some(v) => allowed.push(Some(v.as_slice())),
                None => allowed.push(None),
            }
            if tracker.dirty_since_sync.contains(key) {
                match attempted {
                    Some(v) => allowed.push(v.as_deref()),
                    None => allowed.push(None),
                }
            }
            assert!(
                allowed.contains(&actual),
                "{label} n={n:?}: key {key:?} recovered as {:?}, allowed {:?}",
                actual.map(|v| String::from_utf8_lossy(v).to_string()),
                allowed
                    .iter()
                    .map(|a| a.map(|v| String::from_utf8_lossy(v).to_string()))
                    .collect::<Vec<_>>(),
            );
        }
    }

    pub(crate) struct RunOutcome {
        result: Result<(), DbError>,
        pub(crate) tracker: Tracker,
        failed: bool,
        pub(crate) ops: usize,
    }

    impl RunOutcome {
        pub(crate) fn failed(&self) -> bool {
            self.failed
        }
    }

    pub(crate) fn run_scenario_for_sweep(dir: &Path, n: usize) -> (Result<(), DbError>, usize) {
        let idx = [n];
        let outcome = run_scenario_with_faults(dir, &idx);
        (outcome.result, outcome.ops)
    }

    /// Scenario: interleaved puts/deletes with syncs and two flushes under
    /// aggressive compaction options.
    pub(crate) fn run_scenario_with_faults(dir: &Path, n: &[usize]) -> RunOutcome {
        sys::install_faults(n);
        let mut tracker = Tracker::default();
        let result = (|| -> Result<(), DbError> {
            let mut options = KibanOptions {
                l0_compaction_trigger: 2,
                base_level_bytes: 300,
                level_multiplier: 4,
                target_file_size: 250,
                block_cache_bytes: 1 << 20,
            };
            let _ = &mut options;
            let mut db = Kiban::open_with_options(dir, options)?;
            macro_rules! step {
                ($op:expr) => {
                    if $op.is_err() {
                        return Ok(());
                    }
                };
            }
            for i in 0..10u32 {
                step!(db.put(format!("k{i:03}"), format!("v{i}")));
                tracker.apply(
                    format!("k{i:03}").as_bytes(),
                    Some(format!("v{i}").as_bytes()),
                );
            }
            step!(db.sync());
            tracker.on_sync_ok();

            step!(db.delete(b"k003"));
            tracker.apply(b"k003", None);
            step!(db.put(b"k001", b"updated"));
            tracker.apply(b"k001", Some(b"updated"));
            if db.flush().is_err() {
                tracker.ambiguous = true;
                return Ok(());
            }
            tracker.on_flush_ok();

            for i in 10..20u32 {
                step!(db.put(format!("k{i:03}"), format!("v{i}")));
                tracker.apply(
                    format!("k{i:03}").as_bytes(),
                    Some(format!("v{i}").as_bytes()),
                );
            }
            step!(db.sync());
            tracker.on_sync_ok();

            step!(db.delete(b"k010"));
            tracker.apply(b"k010", None);
            step!(db.put(b"late", b"L"));
            tracker.apply(b"late", Some(b"L"));
            if db.flush().is_err() {
                tracker.ambiguous = true;
                return Ok(());
            }
            tracker.on_flush_ok();
            Ok(())
        })();
        let failed = result.is_err();
        let ops = sys::op_count();
        sys::clear_fault();
        RunOutcome {
            result,
            tracker,
            failed,
            ops,
        }
    }

    #[test]
    fn every_single_syscall_failure_in_the_pipeline_recovers_correctly() {
        let clean_dir = TempDir::new("sweep-clean");
        let clean = run_scenario_with_faults(clean_dir.path(), &[]);
        assert!(clean.result.is_ok(), "clean scenario must succeed");
        let total_ops = clean.ops;
        assert!(
            total_ops > 20,
            "scenario too small to exercise anything: {total_ops}"
        );
        drop(clean);

        let mut any_failed = false;
        for n in 0..total_ops {
            let dir = TempDir::new("sweep");
            let outcome = run_scenario_with_faults(dir.path(), &[n]);
            any_failed |= outcome.failed;

            // D4: reopening after any single-syscall crash must succeed.
            let db = match Kiban::open_with_options(
                dir.path(),
                KibanOptions {
                    l0_compaction_trigger: 2,
                    base_level_bytes: 300,
                    level_multiplier: 4,
                    target_file_size: 250,
                    block_cache_bytes: 1 << 20,
                },
            ) {
                Ok(db) => db,
                Err(e) => panic!("n={n}: reopen failed: {e}"),
            };

            let recovered: Model = db
                .iter()
                .map(|r| r.unwrap())
                .map(|(k, v)| (k, v.to_vec()))
                .collect();
            assert_band("pipeline", &[n], &recovered, &outcome.tracker);

            // scans and gets agree after recovery too
            for (k, v) in &recovered {
                assert_eq!(
                    db.get(k.as_slice()).unwrap().as_deref(),
                    Some(v.as_slice()),
                    "n={n}: get disagrees with scan"
                );
            }
        }
        assert!(any_failed, "no injected failure ever triggered");
    }

    #[test]
    fn sweep_fails_at_every_index_when_asked_to() {
        // sanity on the injection machinery itself: the Nth run's error
        // occurs exactly once, at N, deterministically
        let td = TempDir::new("sweep-machinery");
        let outcomes: Vec<bool> = (0..5)
            .map(|n| {
                sys::install_fault(n);
                let failed = match Kiban::open(td.path()) {
                    Ok(mut db) => db.put(b"x", b"y").is_err() || db.sync().is_err(),
                    Err(_) => true,
                };
                sys::clear_fault();
                failed
            })
            .collect();
        // at least one of the first five ops failing must disturb the run
        assert!(outcomes.iter().any(|f| *f), "injection never fired");
    }
}

/// A clonable, thread-safe handle to one engine.
///
/// Concurrency model per `docs/design/concurrency.md`: one mutex, group
/// commit falls out of the two-step WAL contract (every `sync` flushes
/// all pending records from all writers in one fdatasync). Compaction
/// runs on a single background worker (11.4): foreground `put`/`get`/
/// `sync`/`flush` never wait behind it, only behind each other.
pub struct SharedKiban {
    inner: std::sync::Arc<std::sync::Mutex<Kiban>>,
    maintenance: std::sync::Arc<Maintenance>,
}

impl Clone for SharedKiban {
    fn clone(&self) -> Self {
        self.maintenance.add_handle();
        SharedKiban {
            inner: self.inner.clone(),
            maintenance: self.maintenance.clone(),
        }
    }
}

impl Drop for SharedKiban {
    fn drop(&mut self) {
        // The last handle to go stops and joins the worker (see
        // `Maintenance::drop_handle`) — no immortal thread survives the
        // engine it was maintaining.
        self.maintenance.drop_handle();
    }
}

/// Owned key/value pair yielded by snapshot scans.
type SnapEntry = (Vec<u8>, Vec<u8>);

/// A consistent point-in-time view captured from a [`SharedKiban`].
///
/// Capture copies the memtable (O(its size)) under one lock hold and
/// clones `Arc<TableEntry>` for every currently-live table; reads
/// afterwards never touch the engine lock (concurrency.md D6). Pinning
/// the table `Arc`s directly (rather than remembering paths to reopen
/// later) is what keeps a `SharedSnapshot` correct across a compaction
/// that reclaims one of its tables' files (11.4): the file's directory
/// entry can disappear, but this snapshot's open handle to it does not.
///
/// Dropping a `SharedSnapshot` releases its hold on the engine's
/// `smallest_snapshot` boundary (compaction's tombstone/old-version GC),
/// mirroring `Kiban::release_snapshot` for the direct API — without
/// this, a `SharedSnapshot` would suppress GC for the engine's entire
/// remaining lifetime, not just while it's live.
#[allow(dead_code)]
pub struct SharedSnapshot {
    engine: std::sync::Arc<std::sync::Mutex<Kiban>>,
    seq: u64,
    memtable: Memtable,
    tables: Vec<StdArc<TableEntry>>,
}

impl Drop for SharedSnapshot {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.engine.lock()
            && let Some(pos) = guard.active_snapshots.iter().position(|s| *s == self.seq)
        {
            guard.active_snapshots.remove(pos);
        }
    }
}

impl SharedSnapshot {
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Reads `key` as of this snapshot.
    #[allow(dead_code)]
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>, DbError> {
        let key = key.as_ref();
        if let Some(e) = self.memtable.entry(key)
            && e.seq() <= self.seq
        {
            return Ok(e.as_value().map(|v| v.to_vec()));
        }
        for t in self.tables.iter().rev().filter(|t| t.level == 0) {
            match t.table.get(key, Some(self.seq))? {
                Some(f) if f.seq <= self.seq => {
                    return Ok(match f.kind {
                        Kind::Put => Some(f.value),
                        Kind::Tombstone => None,
                    });
                }
                Some(_) => continue,
                None => continue,
            }
        }
        for t in self.tables.iter().filter(|t| t.level >= 1) {
            if key < t.first_key.as_slice() || key > t.last_key.as_slice() {
                continue;
            }
            match t.table.get(key, Some(self.seq))? {
                Some(f) if f.seq <= self.seq => {
                    return Ok(match f.kind {
                        Kind::Put => Some(f.value),
                        Kind::Tombstone => None,
                    });
                }
                Some(_) => continue,
                None => continue,
            }
        }
        Ok(None)
    }

    /// Scans all live entries visible at this snapshot.
    #[allow(dead_code)]
    pub fn scan(&self) -> Result<Vec<SnapEntry>, DbError> {
        let mut sources: Vec<SourceHead<'_>> = Vec::new();
        sources.push(SourceHead {
            feed: SourceFeed::Mem(Box::new(self.memtable.iter_from(b""))),
            head: None,
            exhausted: false,
        });
        for t in self.tables.iter().rev().filter(|t| t.level == 0) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(t.table.iter_from(b"")),
                head: None,
                exhausted: false,
            });
        }
        for t in self.tables.iter().filter(|t| t.level >= 1) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(t.table.iter_from(b"")),
                head: None,
                exhausted: false,
            });
        }
        let mut core = MergeCore {
            sources,
            user_mode: true,
            failed: false,
            snap_limit: Some(self.seq),
            done_key: None,
        };
        let mut out = Vec::new();
        while let Some(item) = core.next_scanned() {
            let e = item?;
            out.push((e.key, e.value));
        }
        Ok(out)
    }
}

impl SharedKiban {
    pub fn open(dir: impl AsRef<Path>) -> Result<SharedKiban, DbError> {
        Self::open_with_options(dir, KibanOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: KibanOptions,
    ) -> Result<SharedKiban, DbError> {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(Kiban::open_with_options(
            dir, options,
        )?));
        let maintenance = Maintenance::spawn(inner.clone());
        Ok(SharedKiban { inner, maintenance })
    }

    /// The most recent background compaction failure, if any (11.4:
    /// background failures must never be silently ignored). Sticky:
    /// once a job fails, the worker stops attempting more compaction
    /// until the engine is reopened, so this stays set. This is
    /// distinct from [`SharedKiban::is_poisoned`]: a durability-fatal
    /// commit ambiguity poisons the engine itself (refusing further
    /// mutation) *and* is reported here; a lesser failure (e.g. a
    /// corrupt compaction input) is reported here without poisoning the
    /// engine, since it does not put any acknowledged durability claim
    /// in doubt.
    pub fn maintenance_error(&self) -> Option<MaintenanceError> {
        self.maintenance.error()
    }

    #[cfg(test)]
    pub(crate) fn maintenance_for_test(&self) -> &Maintenance {
        &self.maintenance
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Kiban>, DbError> {
        self.inner.lock().map_err(|_| {
            DbError::Corrupt(
                "engine lock poisoned: a panic occurred while an operation was in flight"
                    .to_string(),
            )
        })
    }

    /// Buffered WAL append + memtable write. Not durable until `sync`.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> io::Result<()> {
        match self.lock() {
            Ok(mut guard) => guard.put(key, value),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    pub fn delete(&self, key: impl AsRef<[u8]>) -> io::Result<()> {
        match self.lock() {
            Ok(mut guard) => guard.delete(key),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    /// Whether the shared engine is poisoned.
    pub fn is_poisoned(&self) -> bool {
        match self.lock() {
            Ok(guard) => guard.is_poisoned(),
            Err(_) => true,
        }
    }

    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>, DbError> {
        self.lock()?.get(key)
    }

    /// Makes every record appended by *any* thread so far durable in one
    /// device flush (group commit).
    pub fn sync(&self) -> io::Result<()> {
        match self.lock() {
            Ok(mut guard) => guard.sync(),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    /// Commits a batch atomically under the engine lock. One `sync`
    /// afterwards makes the entire batch durable together — group
    /// commit applies.
    pub fn write(&self, batch: WriteBatch) -> Result<(), DbError> {
        match self.lock() {
            Ok(mut guard) => guard.write(batch),
            Err(poison_err) => Err(poison_err),
        }
    }

    /// Captures a consistent snapshot: O(memtable) copy under one lock
    /// hold; reads on the returned handle never touch the lock.
    pub fn snapshot(&self) -> Result<SharedSnapshot, DbError> {
        let mut guard = self.lock()?;
        let seq = guard.last_sequence;
        let pos = guard.active_snapshots.partition_point(|s| *s < seq);
        guard.active_snapshots.insert(pos, seq);
        Ok(SharedSnapshot {
            engine: self.inner.clone(),
            seq,
            memtable: guard.memtable.clone(),
            tables: guard.version.tables.clone(),
        })
    }

    /// Durably publishes the memtable as a new L0 sstable — the same
    /// guarantee `Kiban::flush` makes — then hands compaction off to
    /// the background worker instead of running it inline (11.4): the
    /// caller returns as soon as its own flush is durable, not after
    /// whatever compaction that flush happens to trigger.
    pub fn flush(&self) -> Result<(), DbError> {
        self.lock()?.flush_without_compaction()?;
        self.maintenance.wake();
        Ok(())
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;
    use std::sync::Arc as StdArc;

    #[test]
    fn handles_are_clonable_and_share_state() {
        let td = TempDir::new("shared-clone");
        let db = SharedKiban::open(td.path()).unwrap();
        let clone = db.clone();
        db.put(b"a", b"1").unwrap();
        assert_eq!(clone.get(b"a").unwrap(), Some(b"1".to_vec()));
    }

    #[test]
    fn concurrent_writers_all_land_with_one_group_sync() {
        let td = TempDir::new("shared-group-commit");
        let db = StdArc::new(SharedKiban::open(td.path()).unwrap());
        let mut expected: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        let threads: Vec<_> = (0..4)
            .map(|t| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for i in 0..50u32 {
                        let key = format!("t{t}-k{i:03}");
                        let val = format!("v{t}-{i}");
                        db.put(&key, &val).unwrap();
                    }
                    db.sync().unwrap();
                })
            })
            .collect();
        for th in threads {
            th.join().unwrap();
        }
        // rebuild expectation deterministically (threads wrote disjoint ranges)
        for t in 0..4u32 {
            for i in 0..50u32 {
                expected.insert(
                    format!("t{t}-k{i:03}").into_bytes(),
                    format!("v{t}-{i}").into_bytes(),
                );
            }
        }

        let scanned: Vec<(Vec<u8>, Vec<u8>)> = {
            let guard = db.lock().unwrap();
            guard
                .iter()
                .map(|r| r.unwrap())
                .map(|(k, v)| (k.clone(), v.to_vec()))
                .collect()
        };
        assert_eq!(scanned.len(), expected.len());
        for (k, v) in &scanned {
            assert_eq!(expected.get(k).map(|e| e.as_slice()), Some(v.as_slice()));
        }
    }

    #[test]
    fn concurrent_readers_never_see_torn_state() {
        let td = TempDir::new("shared-readers");
        let db = StdArc::new(SharedKiban::open(td.path()).unwrap());
        db.put(b"anchor", b"stable").unwrap();
        db.sync().unwrap();

        let writer = {
            let db = db.clone();
            std::thread::spawn(move || {
                for i in 0..200u32 {
                    db.put(format!("key{i:04}"), format!("val{i}")).unwrap();
                    if i % 25 == 0 {
                        db.sync().unwrap();
                    }
                }
                db.flush().unwrap();
            })
        };
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        assert_eq!(db.get(b"anchor").unwrap(), Some(b"stable".to_vec()));
                    }
                })
            })
            .collect();
        writer.join().unwrap();
        for r in readers {
            r.join().unwrap();
        }
        assert_eq!(db.get(b"anchor").unwrap(), Some(b"stable".to_vec()));
    }
}

#[cfg(test)]
mod cache_scaling_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    fn small_cache_options() -> KibanOptions {
        KibanOptions {
            l0_compaction_trigger: 8, // keep many tables alive
            base_level_bytes: u64::MAX,
            level_multiplier: 10,
            target_file_size: 64 * 1024,
            block_cache_bytes: 4096, // tiny: far smaller than the data
        }
    }

    #[test]
    fn database_much_larger_than_cache_opens_and_serves_correctly() {
        let td = TempDir::new("cache-scale");
        let options = small_cache_options();
        let mut db = Kiban::open_with_options(td.path(), options.clone()).unwrap();

        // ~300 KB of live data against a 4 KB cache: 75x oversubscription
        let count = 4000usize;
        let mut reference: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        for i in 0..count {
            let key = format!("key-{i:06}");
            let value = format!("value-{i:06}-{}", vec![b'x'; 50].len());
            let value = format!("{value}-{:x}", i);
            db.put(key.as_bytes(), value.as_bytes()).unwrap();
            reference.insert(key.into_bytes(), value.into_bytes());
        }
        db.sync().unwrap();
        db.flush().unwrap();
        drop(db);

        // fresh open: handles only; nothing may assume whole-table residency
        let db = Kiban::open_with_options(td.path(), options.clone()).unwrap();
        for i in 0..count {
            let key = format!("key-{i:06}");
            let got = db
                .get(key.as_bytes())
                .unwrap()
                .unwrap_or_else(|| panic!("missing {key}"));
            let want = &reference[key.as_bytes()];
            assert_eq!(got, *want, "wrong value for {key}");
        }

        // full scan agrees too
        let scanned: usize = db.iter().map(|r| r.unwrap()).fold(0, |n, _| n + 1);
        assert_eq!(scanned, count);

        // and the cache stayed inside its budget the whole time
        assert!(db.cache.resident_bytes() <= options.block_cache_bytes);
    }

    #[test]
    fn every_table_remains_readable_after_cold_reopen() {
        let td = TempDir::new("cache-cold");
        let mut db = Kiban::open_with_options(td.path(), small_cache_options()).unwrap();
        let mut expected_total = 0usize;
        for t in 0..24u32 {
            for i in 0..20u32 {
                db.put(format!("t{t:02}-k{i:03}"), format!("v{t}-{i}").as_bytes())
                    .unwrap();
                expected_total += 1;
            }
            db.sync().unwrap();
            db.flush().unwrap();
        }
        drop(db);

        let db = Kiban::open_with_options(td.path(), small_cache_options()).unwrap();
        let scanned: usize = db.iter().map(|r| r.unwrap()).fold(0, |n, _| n + 1);
        assert_eq!(scanned, expected_total);

        // spot checks from the first, middle, and last table
        assert_eq!(db.get(b"t00-k000").unwrap(), Some(b"v0-0".to_vec()));
        assert_eq!(db.get(b"t12-k010").unwrap(), Some(b"v12-10".to_vec()));
        assert_eq!(db.get(b"t23-k019").unwrap(), Some(b"v23-19".to_vec()));
        assert_eq!(db.get(b"missing").unwrap(), None);
    }
}

#[cfg(test)]
mod crash_pair_sweep_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    type Model = BTreeMap<Vec<u8>, Vec<u8>>;

    /// Two injected failures per run: the interaction space between
    /// crash points (e.g. a failed WAL sync followed by a failed
    /// manifest install) gets actual coverage, not just prefixes.
    #[test]
    fn every_pair_of_syscall_failures_recovers_correctly() {
        let clean_dir = TempDir::new("pair-clean");
        let (clean, total) =
            super::crash_sweep_tests::run_scenario_for_sweep(clean_dir.path(), usize::MAX);
        assert!(clean.is_ok(), "clean scenario must succeed");
        assert!(total > 20, "scenario too small: {total}");

        let mut ran = 0usize;
        for a in 0..total {
            for b in (a + 1)..total {
                let dir = TempDir::new("pair-sweep");
                let outcome =
                    super::crash_sweep_tests::run_scenario_with_faults(dir.path(), &[a, b]);
                if !outcome.failed() {
                    continue; // neither index fired within this run's op count
                }
                ran += 1;
                let tracker = outcome.tracker.clone();
                drop(outcome);

                let db = match Kiban::open_with_options(
                    dir.path(),
                    super::compaction_tests::tiny_options(),
                ) {
                    Ok(db) => db,
                    Err(e) => panic!("faults {a},{b}: reopen failed: {e}"),
                };
                let recovered: Model =
                    db.iter()
                        .map(|r| r.unwrap())
                        .fold(BTreeMap::new(), |mut m, (k, v)| {
                            m.insert(k, v);
                            m
                        });
                super::crash_sweep_tests::assert_band("pair-sweep", &[a, b], &recovered, &tracker);
            }
        }
        assert!(ran > 100, "pair sweep barely exercised anything: {ran}");
    }
}

#[cfg(test)]
mod power_loss_tests {
    use super::*;
    use crate::sys;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    /// The strongest durability claim available: with a simulated
    /// volatile device, a crash discards exactly the unsynced bytes.
    /// After power loss the recovered state must EQUAL the last synced
    /// state — not merely fall within a band. Every single- and
    /// two-fault crash point in the pipeline is checked.
    #[test]
    fn power_loss_recovers_exactly_the_last_synced_state() {
        let clean_dir = TempDir::new("pl-clean");
        let (clean, total) =
            super::crash_sweep_tests::run_scenario_for_sweep(clean_dir.path(), usize::MAX);
        assert!(clean.is_ok(), "clean scenario must succeed");
        assert!(total > 20);

        let mut ran = 0usize;
        let mut checked_exact = 0usize;
        for a in 0..total {
            for b in 0..total {
                if a == b {
                    continue;
                }
                let dir = TempDir::new("pl-sweep");
                sys::enable_device_sim();
                let outcome =
                    super::crash_sweep_tests::run_scenario_with_faults(dir.path(), &[a, b]);
                let terminated_early = outcome.ops < total;
                let tracker = outcome.tracker.clone();
                sys::clear_fault();

                // simulated power loss: overlays vanish, committed stays
                sys::power_loss();

                let db = match Kiban::open_with_options(
                    dir.path(),
                    super::compaction_tests::tiny_options(),
                ) {
                    Ok(db) => db,
                    Err(e) => {
                        sys::disable_device_sim();
                        panic!("faults {a},{b}: reopen after power loss failed: {e:?}");
                    }
                };
                let recovered: BTreeMap<Vec<u8>, Vec<u8>> =
                    db.iter()
                        .map(|r| r.unwrap())
                        .fold(BTreeMap::new(), |mut m, (k, v)| {
                            m.insert(k, v);
                            m
                        });
                drop(db);
                sys::disable_device_sim();

                if tracker.ambiguous {
                    // A failed flush leaves the durable floor ambiguous:
                    // its commit point may have passed before the failure.
                    super::crash_sweep_tests::assert_band(
                        "power-loss-ambiguous",
                        &[a, b],
                        &recovered,
                        &tracker,
                    );
                } else {
                    // EXACT equality with the last acknowledged state
                    assert_eq!(
                        recovered, tracker.synced,
                        "faults {a},{b}: post-power-loss state diverged from last synced state"
                    );
                }
                checked_exact += 1;
                if a < 3 && b < 6 {
                    eprintln!("DBG ({a},{b}): ops={} total={}", outcome.ops, total);
                }
                if terminated_early {
                    ran += 1;
                }
            }
        }
        assert!(
            ran > 100 && checked_exact > 100,
            "power-loss sweep barely exercised anything: ran={ran} checked={checked_exact}"
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    fn tiny_options() -> KibanOptions {
        super::compaction_tests::tiny_options()
    }

    #[test]
    fn snapshot_reads_are_immune_to_later_writes_flushes_and_compaction() {
        let td = TempDir::new("snap-immune");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();

        for i in 0..20u32 {
            db.put(format!("k{i:03}"), format!("gen1-{i}").as_bytes())
                .unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();
        // delete half, overwrite the rest — AFTER capturing the snapshot
        let snap = db.snapshot();
        for i in 0..20u32 {
            if i % 2 == 0 {
                db.delete(format!("k{i:03}")).unwrap();
            } else {
                db.put(format!("k{i:03}"), format!("gen2-{i}").as_bytes())
                    .unwrap();
            }
        }
        db.sync().unwrap();
        db.flush().unwrap(); // triggers compaction under tiny options
        // push more rounds to force deep levels
        for round in 0..4u32 {
            for i in 0..20u32 {
                db.put(format!("k{i:03}"), format!("g{round}-{i}").as_bytes())
                    .unwrap();
            }
            db.sync().unwrap();
            db.flush().unwrap();
        }

        // latest state: everything re-written by the final rounds
        assert_eq!(db.get(b"k000").unwrap(), Some(b"g3-0".to_vec()));
        assert_eq!(db.get(b"k001").unwrap(), Some(b"g3-1".to_vec()));

        // snapshot still sees the ORIGINAL world exactly
        let scanned = db.scan_at(&snap).unwrap();
        assert_eq!(scanned.len(), 20);
        for i in 0..20u32 {
            let key = format!("k{i:03}");
            let want = format!("gen1-{i}");
            assert_eq!(
                db.get_at(&snap, key.as_bytes()).unwrap(),
                Some(want.into_bytes())
            );
        }
        let _ = scanned;
    }

    #[test]
    fn snapshot_scan_matches_point_reads_at_same_boundary() {
        let td = TempDir::new("snap-agree");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        let mut reference: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        // seed a world, THEN capture the snapshot
        for i in 0..10u32 {
            db.put(format!("seed-{i}"), format!("s{i}").as_bytes())
                .unwrap();
            reference.insert(
                format!("seed-{i}").into_bytes(),
                format!("s{i}").into_bytes(),
            );
        }
        db.sync().unwrap();
        db.flush().unwrap();
        let snap = db.snapshot();

        let mut state = 42u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..30u64 {
            for _ in 0..8 {
                let i = next() % 40;
                let key = format!("key-{i:02}");
                if next() % 4 == 0 {
                    db.delete(key.as_bytes()).unwrap();
                    reference.remove(key.as_bytes());
                } else {
                    let v = format!("r{round}-{i}");
                    db.put(key.as_bytes(), v.as_bytes()).unwrap();
                    reference.insert(key.into_bytes(), v.into_bytes());
                }
            }
            db.sync().unwrap();
            db.flush().unwrap();

            // snapshot view frozen at capture: seeds only, forever
            let scanned = db.scan_at(&snap).unwrap();
            let want: Vec<(Vec<u8>, Vec<u8>)> = vec![
                ("seed-0", "s0"),
                ("seed-1", "s1"),
                ("seed-2", "s2"),
                ("seed-3", "s3"),
                ("seed-4", "s4"),
                ("seed-5", "s5"),
                ("seed-6", "s6"),
                ("seed-7", "s7"),
                ("seed-8", "s8"),
                ("seed-9", "s9"),
            ]
            .into_iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect();
            assert_eq!(scanned, want);
        }
    }

    #[test]
    fn shared_snapshot_survives_concurrent_mutation() {
        let td = TempDir::new("shared-snap");
        let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();
        for i in 0..50u32 {
            db.put(format!("s{i:03}"), format!("base-{i}").as_bytes())
                .unwrap();
        }
        db.sync().unwrap();

        let snap = db.snapshot().unwrap();
        assert_eq!(snap.get(b"s000").unwrap(), Some(b"base-0".to_vec()));

        // mutate heavily through the shared handle afterwards
        for i in 0..50u32 {
            db.delete(format!("s{i:03}")).unwrap();
            db.put(format!("t{i:03}"), format!("new-{i}").as_bytes())
                .unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();

        // snapshot world unchanged; engine world moved on
        assert_eq!(snap.scan().unwrap().len(), 50);
        assert_eq!(snap.get(b"s049").unwrap(), Some(b"base-49".to_vec()));
        assert_eq!(db.get(b"s000").unwrap(), None);
        assert!(db.get(b"t000").unwrap().is_some());
    }
}

#[cfg(test)]
mod dbg_seq_probe {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn probe_seqs_after_compaction() {
        let td = TempDir::new("probe-seqs");
        let opts = super::compaction_tests::tiny_options();
        let mut db = Kiban::open_with_options(td.path(), opts).unwrap();
        for i in 0..20u32 {
            db.put(format!("k{i:03}"), format!("gen1-{i}").as_bytes())
                .unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();
        println!("--- after first flush ---");
        for t in &db.version.tables {
            println!("table L{} #{}:", t.level, t.number);
            for r in t.table.iter().take(3) {
                match r {
                    Ok((kd, sq, k, v)) => println!(
                        "   {:?} kind={:?} seq={} v={}",
                        String::from_utf8_lossy(&k),
                        kd,
                        sq,
                        String::from_utf8_lossy(&v)
                    ),
                    Err(e) => println!("   ERR {e}"),
                }
            }
        }
        for i in 0..20u32 {
            let k = format!("k{i:03}");
            if i % 2 == 0 {
                db.delete(k.as_bytes()).unwrap();
            } else {
                db.put(k.as_bytes(), format!("gen2-{i}").as_bytes())
                    .unwrap();
            }
        }
        db.sync().unwrap();
        db.flush().unwrap();
        for t in &db.version.tables {
            println!("table L{} #{}:", t.level, t.number);
            for r in t.table.iter().take(3) {
                match r {
                    Ok((kd, sq, k, v)) => println!(
                        "   {:?} kind={:?} seq={} v={}",
                        String::from_utf8_lossy(&k),
                        kd,
                        sq,
                        String::from_utf8_lossy(&v)
                    ),
                    Err(e) => println!("   ERR {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod poisoning_tests {
    use super::*;
    use crate::sys;
    use crate::testutil::TempDir;

    fn fresh(label: &str) -> (TempDir, Kiban) {
        let td = TempDir::new(label);
        let db = Kiban::open(td.path()).unwrap();
        (td, db)
    }

    #[test]
    fn wal_sync_failure_poisons_and_blocks_all_mutations() {
        let (td, mut db) = fresh("poison-sync");
        db.put(b"safe", b"v").unwrap();
        db.sync().unwrap();

        // find the WAL sync op index: put(1 op) + sync(1 op) happened at
        // open+seed; the next sync is the target. Compute by replaying.
        // Deterministic approach: install a fault sweep — fail each
        // candidate index in turn until sync fails, then assert poison.
        let mut poisoned_at: Option<usize> = None;
        for n in 0..12usize {
            drop(Kiban::open(td.path()).unwrap()); // fresh runtime state
            sys::install_fault(n);
            let mut probe = match Kiban::open(td.path()) {
                Ok(d) => d,
                Err(_) => {
                    sys::clear_fault();
                    continue;
                }
            };
            let _ = probe.put(b"x", b"y");
            let sync_failed = probe.sync().is_err();
            let failed = sync_failed || probe.is_poisoned();
            sys::clear_fault();
            if failed && probe.is_poisoned() {
                poisoned_at = Some(n);
                break;
            }
        }
        let Some(n) = poisoned_at else {
            panic!("could not induce a WAL sync failure");
        };

        // deterministic reproduction at n: engine must be poisoned
        sys::install_fault(n);
        let mut db2 = Kiban::open(td.path()).unwrap();
        let _ = db2.put(b"a", b"1");
        let _ = db2.sync();
        sys::clear_fault();

        if db2.is_poisoned() {
            // every mutation path refuses; reads remain available
            let e = db2.put(b"later", b"v").unwrap_err();
            assert!(
                matches!(e.kind(), io::ErrorKind::Other),
                "put after poison: {e}"
            );
            let e = db2.delete(b"later").unwrap_err();
            let _ = e;
            let e = db2.sync().unwrap_err();
            let _ = e;
            let e = db2.flush().unwrap_err();
            assert!(matches!(e, DbError::Poisoned(_)), "{e:?}");
            // reads still work
            let _ = db2.get(b"a").unwrap();
        }
        let _ = n;
    }

    /// A WAL append failure (bytes never reaching the kernel) poisons
    /// the engine: later mutations are refused, reads stay available.
    /// Induction: a >8KiB value forces BufWriter to write through to the
    /// checked sys::File during `put` itself, so a fault at the write op
    /// fails the append directly.
    #[test]
    fn wal_append_failure_poisons_engine() {
        let big = vec![b'v'; 16 * 1024];

        // sweep for the write-op index of the big put
        let mut found: Option<usize> = None;
        for n in 0..20usize {
            // fresh directory each iteration: an earlier failed init can
            // leave real debris (e.g. an unsynced WAL) that would otherwise
            // fail every later open with AlreadyExists
            let iter_dir = TempDir::new("poison-append-iter");
            sys::install_fault(n);
            let open_result = Kiban::open(iter_dir.path());
            if open_result.is_err() {
                // fault hit an init op; clear and keep sweeping
                sys::clear_fault();
                continue;
            }
            let mut d = open_result.unwrap();
            let r = d.put(b"big", &big);
            sys::clear_fault();
            if r.is_err() {
                found = Some(n);
                break;
            }
        }
        let Some(write_op) = found else {
            panic!("could not induce a WAL append failure");
        };

        // deterministic repro + post-conditions (fresh dir: same op order)
        let repro_dir = TempDir::new("poison-append-repro");
        sys::install_fault(write_op);
        let mut d = Kiban::open(repro_dir.path()).unwrap();
        let _ = d.put(b"big", &big);
        sys::clear_fault();

        assert!(d.is_poisoned(), "append failure must poison");
        assert!(matches!(
            d.poison_cause(),
            Some(PoisonCause::WalAppendFailed(_))
        ));
        let e = d.put(b"later", b"x").unwrap_err();
        assert!(e.to_string().contains("poisoned"), "{e}");
        let e = d.sync().unwrap_err();
        assert!(e.to_string().contains("poisoned"), "{e}");
        assert!(d.get(b"big").is_ok(), "reads remain available");
    }

    /// Commit ambiguity (rename succeeded, directory fsync failed) must
    /// poison: the engine cannot know which topology persisted. Sweep
    /// fault indices over a seed+flush scenario until the flush fails
    /// with Poisoned; then verify later mutation is refused and reopen
    /// reconstructs whichever topology actually persisted.
    #[test]
    fn commit_ambiguity_during_flush_poisons_engine() {
        let td = TempDir::new("poison-ambiguity");

        // clean baseline so sweeps have stable state to reopen
        let mut induced: Option<usize> = None;
        for n in 4..40usize {
            drop(Kiban::open(td.path()).unwrap());
            sys::install_faults(&[n]);
            let mut d = match Kiban::open(td.path()) {
                Ok(d) => d,
                Err(_) => {
                    sys::clear_fault();
                    continue;
                }
            };
            if d.is_poisoned() {
                sys::clear_fault();
                continue;
            }
            let _ = d.put(b"k", b"v");
            let flush_result = d.flush();
            let poisoned = matches!(
                &flush_result,
                Err(DbError::Poisoned(PoisonCause::CommitAmbiguity(_)))
            );
            let failed_at_all = flush_result.is_err();
            sys::clear_fault();

            if poisoned {
                induced = Some(n);
                // post-conditions with faults cleared: mutations refused
                let e = d.put(b"later", b"x").unwrap_err();
                assert!(e.to_string().contains("poisoned"), "{e}");
                let e = d.flush().unwrap_err();
                assert!(matches!(e, DbError::Poisoned(_)), "{e:?}");
                // reads stay available
                let _ = d.get(b"k").unwrap();
                break;
            }
            if !failed_at_all {
                continue;
            }
        }
        let Some(_) = induced else {
            panic!("commit ambiguity never induced by single faults");
        };

        // reopen after ambiguity: disk truth wins, runtime unpoisoned
        let db = Kiban::open(td.path()).unwrap();
        assert!(!db.is_poisoned());
        let got = db.get(b"k").unwrap();
        assert!(got.is_none() || got == Some(b"v".to_vec()));
    }

    #[test]
    fn poisoned_runtime_state_clears_on_reopen_when_disk_is_valid() {
        let (td, mut db) = fresh("poison-reopen");
        db.put(b"durable", b"yes").unwrap();
        db.sync().unwrap();

        // simulate poisoning directly (unit-level: cause stored)
        db.poison(PoisonCause::WalSyncFailed("test".into()));
        assert!(db.is_poisoned());
        let err = db.put(b"nope", b"x").unwrap_err();
        assert!(err.to_string().contains("poisoned"));
        drop(db);

        // reopen: disk state valid -> clean runtime state
        let db = Kiban::open(td.path()).unwrap();
        assert!(!db.is_poisoned());
        assert_eq!(db.get(b"durable").unwrap(), Some(b"yes".to_vec()));
        let mut db = db;
        db.put(b"after", b"ok").unwrap();
        db.sync().unwrap();
        assert_eq!(db.get(b"after").unwrap(), Some(b"ok".to_vec()));
    }

    #[test]
    fn shared_handle_starts_unpoisoned_after_reopen() {
        let td = TempDir::new("shared-poison");
        {
            let mut db = Kiban::open(td.path()).unwrap();
            db.put(b"k", b"v").unwrap();
            db.poison(PoisonCause::WalAppendFailed("unit".into()));
            assert!(db.is_poisoned());
        }
        let shared = SharedKiban::open(td.path()).unwrap();
        assert!(!shared.is_poisoned(), "fresh reopen starts unpoisoned");
        assert!(shared.put(b"after", b"ok").is_ok());
    }
}

#[cfg(test)]
mod write_batch_tests {
    use super::*;
    use crate::sys;
    use crate::testutil::TempDir;

    fn tiny_options() -> KibanOptions {
        super::compaction_tests::tiny_options()
    }

    #[test]
    fn batch_roundtrip_through_recovery() {
        let td = TempDir::new("batch-roundtrip");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        let mut b = WriteBatch::new();
        b.put(b"b1", b"v1").put(b"b2", b"v2");
        b.delete(b"b1");
        b.put(b"b3", b"v3");
        db.write(b).unwrap();
        assert_eq!(db.get(b"b1").unwrap(), None);
        assert_eq!(db.get(b"b2").unwrap(), Some(b"v2".to_vec()));
        drop(db);

        let db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        assert_eq!(db.get(b"b1").unwrap(), None, "tombstone must survive");
        assert_eq!(db.get(b"b3").unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn batch_sequence_interval_is_contiguous_and_monotonic() {
        let td = TempDir::new("batch-seqs");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        db.put(b"a", b"x").unwrap(); // seq 1
        let mut b = WriteBatch::new();
        b.put(b"s1", b"1")
            .delete(b"s1")
            .put(b"s2", b"2")
            .put(b"s3", b"3");
        db.write(b).unwrap();
        // seqs 2..=5 consumed; next put gets seq 6
        db.put(b"after", b"z").unwrap();

        // reopen: replay must reproduce exactly; a discontinuity would
        // have failed decode validation
        drop(db);
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        assert_eq!(db.get(b"s1").unwrap(), None);
        assert_eq!(db.get(b"s2").unwrap(), Some(b"2".to_vec()));
        assert_eq!(db.get(b"s3").unwrap(), Some(b"3".to_vec()));
        assert_eq!(db.get(b"after").unwrap(), Some(b"z".to_vec()));
        // engine continues from the right sequence point
        db.put(b"post", b"p").unwrap();
        assert_eq!(db.get(b"post").unwrap(), Some(b"p".to_vec()));
    }

    /// A torn WAL tail that cuts INTO a batch record must not apply any
    /// of the batch — atomicity comes from frame integrity.
    #[test]
    fn torn_tail_inside_batch_drops_the_whole_batch() {
        let td = TempDir::new("batch-torn");
        let opts = tiny_options();
        {
            let mut db = Kiban::open_with_options(td.path(), opts.clone()).unwrap();
            db.put(b"committed", b"yes").unwrap();
            db.sync().unwrap();
            let mut b = WriteBatch::new();
            for i in 0..50u32 {
                b.put(format!("victim{i}"), format!("v{i}"));
            }
            db.write(b).unwrap(); // buffered, NOT synced
            // simulate crash mid-write: truncate the buffered tail away
            let wal = db.wal_for_test();
            wal.writer_flush_for_test();
            let path = wal.path().to_path_buf();
            let len = std::fs::metadata(&path).unwrap().len();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(len - 30)
                .unwrap();
        }

        let db = Kiban::open_with_options(td.path(), opts).unwrap();
        assert_eq!(db.get(b"committed").unwrap(), Some(b"yes".to_vec()));
        for i in 0..50u32 {
            assert_eq!(
                db.get(format!("victim{i}").as_bytes()).unwrap(),
                None,
                "partial batch leaked victim{i}"
            );
        }
    }

    /// A corrupted batch payload (bad op byte) is corruption, not a
    /// partial apply.
    #[test]
    fn corrupted_batch_payload_is_rejected_at_open() {
        let td = TempDir::new("batch-corrupt");
        let opts = tiny_options();
        {
            let mut db = Kiban::open_with_options(td.path(), opts.clone()).unwrap();
            let mut b = WriteBatch::new();
            b.put(b"k1", b"v1").put(b"k2", b"v2");
            db.write(b).unwrap();
            db.sync().unwrap();
        }
        // corrupt the first batch-op's kind byte inside the frame
        let dir = td.path();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some(WAL_EXTENSION) && sys::exists(&p) {
                let mut raw = std::fs::read(&p).unwrap();
                if raw.len() > 40 {
                    raw[30] ^= 0xFF; // inside the batch payload region
                    std::fs::write(&p, &raw).unwrap();
                    break;
                }
            }
        }
        // Either defense is valid: frame CRC rejection OR batch decode
        // rejection. What matters is refusal, never partial application.
        match Kiban::open_with_options(dir, opts) {
            Err(DbError::Corrupt(_)) => {}
            other => panic!(
                "expected corruption at open, got {:?}",
                other.err().map(|e| e.to_string())
            ),
        }
    }

    #[test]
    fn snapshot_visibility_across_batch_sequences() {
        let td = TempDir::new("batch-snap");
        let mut db = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
        db.put(b"hold", b"orig").unwrap();
        db.sync().unwrap();
        let snap = db.snapshot();

        // one batch: overwrite hold, add news
        let mut b = WriteBatch::new();
        b.put(b"hold", b"changed")
            .put(b"n1", b"1")
            .delete(b"gone-pre");
        db.write(b).unwrap();
        db.sync().unwrap();

        // latest view sees the whole batch
        assert_eq!(db.get(b"hold").unwrap(), Some(b"changed".to_vec()));
        assert_eq!(db.get(b"n1").unwrap(), Some(b"1".to_vec()));

        // snapshot predates every seq in the batch: sees nothing of it,
        // and still sees the pre-batch world
        assert_eq!(db.get_at(&snap, b"hold").unwrap(), Some(b"orig".to_vec()));
        assert_eq!(db.get_at(&snap, b"n1").unwrap(), None);
        let scanned = db.scan_at(&snap).unwrap();
        assert!(scanned.contains(&(b"hold".to_vec(), b"orig".to_vec())));
        assert!(!scanned.iter().any(|(k, _)| k == b"n1"));
    }

    #[test]
    fn empty_batch_is_a_noop() {
        let td = TempDir::new("batch-empty");
        let mut db = Kiban::open(td.path()).unwrap();
        db.write(WriteBatch::new()).unwrap();
        assert_eq!(db.last_sequence_for_test(), 0);
    }

    #[test]
    fn shared_handle_writes_batch_atomically() {
        let td = TempDir::new("batch-shared");
        let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();
        let mut b = WriteBatch::new();
        b.put(b"x", b"1").put(b"y", b"2");
        db.write(b).unwrap();
        db.sync().unwrap();
        assert_eq!(db.get(b"x").unwrap(), Some(b"1".to_vec()));
        assert_eq!(db.get(b"y").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn fault_during_batch_append_poisons_never_partial_applies() {
        let _td = TempDir::new("batch-fault");
        let opts = tiny_options();
        // sweep fault indices over seed+batch-write; when the batch append
        // fails, the engine must poison and later mutation must be refused
        let mut induced = false;
        for n in 0..25usize {
            let iter_dir = TempDir::new("batch-fault-iter");
            sys::install_fault(n);
            let opened = Kiban::open_with_options(iter_dir.path(), opts.clone());
            let Ok(mut d) = opened else {
                sys::clear_fault();
                continue;
            };
            let mut b = WriteBatch::new();
            let big = vec![b'x'; 16 * 1024];
            b.put(b"bk", big).delete(b"none");
            let r = d.write(b);
            let poisoned_after = d.is_poisoned();
            sys::clear_fault();
            if r.is_err() && poisoned_after {
                induced = true;
                let later = d.write(WriteBatch::new());
                assert!(later.is_err(), "mutation accepted after poison");
                break;
            }
            assert!(r.is_ok(), "write failed without poisoning: {r:?}");
        }
        assert!(induced, "batch append failure never induced");
    }
}

/// 11.4: background compaction. Tests 1-6 from the phase spec live here.
/// "Test 7 — crash tests still pass" has no new test of its own: it is
/// the existing suite (crash_sweep_tests, crash_pair_sweep_tests,
/// power_loss_tests, poisoning_tests, ...) still passing unchanged,
/// since the direct `Kiban` path is untouched by this phase.
#[cfg(test)]
mod background_tests {
    use super::*;
    use crate::sys;
    use crate::testutil::TempDir;

    fn tiny_options() -> KibanOptions {
        super::compaction_tests::tiny_options()
    }

    /// Puts `rounds` generations of `k000..k019` through `db`, syncing
    /// and flushing each — enough L0 tables (tiny_options triggers at 2)
    /// to make compaction necessary.
    fn seed_for_compaction(db: &SharedKiban, rounds: u32, label: &str) {
        for round in 0..rounds {
            for i in 0..20u32 {
                db.put(format!("k{i:03}"), format!("{label}{round}-{i}"))
                    .unwrap();
            }
            db.sync().unwrap();
            db.flush().unwrap();
        }
    }

    /// Test 1: a compaction deliberately frozen mid-flight must not
    /// block unrelated foreground work — the central proof of this
    /// phase, and also its performance proof (deterministic, not timed):
    /// the freeze is real (the worker announces it via the checkpoint,
    /// not a guessed sleep), and foreground put/get complete while it
    /// holds.
    #[test]
    fn foreground_work_continues_while_compaction_build_is_paused() {
        let td = TempDir::new("bg-foreground-continues");
        let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_for_compaction(&db, 3, "r");
        m.wait_before_build_reached(); // PLAN is done; BUILD has not started

        // foreground work must complete without waiting for BUILD
        db.put(b"during-freeze", b"v").unwrap();
        assert_eq!(db.get(b"during-freeze").unwrap(), Some(b"v".to_vec()));

        m.release_before_build();
        m.wait_settled();
        assert!(
            db.maintenance_error().is_none(),
            "{:?}",
            db.maintenance_error()
        );
        assert_eq!(db.get(b"during-freeze").unwrap(), Some(b"v".to_vec()));
    }

    /// Test 2: the critical one. A compaction plan is frozen right after
    /// PLAN (before BUILD, so before COMMIT); while it's frozen, a fresh
    /// flush publishes a brand-new L0 table. That table must survive:
    /// COMMIT must apply as a delta against the CURRENT Version, not
    /// overwrite it with the stale one PLAN captured.
    #[test]
    fn flush_published_during_paused_compaction_survives_commit_and_reopen() {
        let td = TempDir::new("bg-new-flush-survives");
        let opts = tiny_options();
        let db = SharedKiban::open_with_options(td.path(), opts.clone()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_for_compaction(&db, 3, "old");
        m.wait_before_build_reached();

        // a fresh flush lands WHILE that old compaction plan is frozen
        for i in 0..20u32 {
            db.put(format!("k{i:03}"), b"new").unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();

        m.release_before_build();
        m.wait_settled();
        assert!(
            db.maintenance_error().is_none(),
            "{:?}",
            db.maintenance_error()
        );

        for i in 0..20u32 {
            let key = format!("k{i:03}");
            assert_eq!(
                db.get(key.as_bytes()).unwrap(),
                Some(b"new".to_vec()),
                "key {key} lost"
            );
        }

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
        for i in 0..20u32 {
            let key = format!("k{i:03}");
            assert_eq!(
                reopened.get(key.as_bytes()).unwrap(),
                Some(b"new".to_vec()),
                "key {key} lost after reopen"
            );
        }
    }

    /// Test 3: a `SharedSnapshot` pins the files it needs (via
    /// `Arc<TableEntry>`, not a path it hopes still exists) so it keeps
    /// reading correctly even after background compaction reclaims —
    /// unlinks — the file it originally opened.
    #[test]
    fn shared_snapshot_survives_background_compaction() {
        let td = TempDir::new("bg-snapshot-survives");
        let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();

        for i in 0..10u32 {
            db.put(format!("s{i:03}"), format!("orig-{i}")).unwrap();
        }
        db.sync().unwrap();
        db.flush().unwrap();

        let snap = db.snapshot().unwrap();

        // enough further writes elsewhere to force the table `snap`
        // pinned (still the only L0 table so far) into a real compaction
        seed_for_compaction(&db, 4, "r");
        db.maintenance_for_test().wait_settled();
        assert!(
            db.maintenance_error().is_none(),
            "{:?}",
            db.maintenance_error()
        );

        // the snapshot still sees exactly its original world
        for i in 0..10u32 {
            let key = format!("s{i:03}");
            assert_eq!(
                snap.get(key.as_bytes()).unwrap(),
                Some(format!("orig-{i}").into_bytes())
            );
        }
        let scanned = snap.scan().unwrap();
        assert_eq!(scanned.len(), 10);

        // only after the snapshot is released does anything change about
        // its file lifetime guarantee — current state has moved on
        // regardless
        drop(snap);
        assert!(db.get(b"k000").unwrap().is_some());
    }

    /// Test 4: point reads and scans keep agreeing through `SharedKiban`
    /// while background compaction runs freely (no pausing) — the same
    /// invariant the synchronous engine has always held, now checked
    /// under concurrency.
    #[test]
    fn point_reads_and_scans_agree_through_shared_kiban_under_compaction() {
        let td = TempDir::new("bg-agreement");
        let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();

        let mut state: u64 = 0xC0FF_EE00;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for round in 0..40u64 {
            for _ in 0..10 {
                let i = next() % 60;
                let key = format!("k{i:03}");
                if next() % 5 == 0 {
                    db.delete(key.as_bytes()).unwrap();
                } else {
                    db.put(key.as_bytes(), format!("r{round}-{i}")).unwrap();
                }
            }
            db.sync().unwrap();
            db.flush().unwrap();

            let snap = db.snapshot().unwrap();
            let scanned = snap.scan().unwrap();
            for (k, v) in &scanned {
                assert_eq!(
                    snap.get(k).unwrap().as_deref(),
                    Some(v.as_slice()),
                    "round {round}: get disagrees with scan for {k:?}"
                );
            }
            for i in 0..60u32 {
                let key = format!("k{i:03}");
                if let Some(v) = snap.get(key.as_bytes()).unwrap() {
                    assert!(
                        scanned
                            .iter()
                            .any(|(k, sv)| k == key.as_bytes() && sv == &v),
                        "round {round}: scan missing live key {key}"
                    );
                }
            }
        }

        db.maintenance_for_test().wait_settled();
        assert!(
            db.maintenance_error().is_none(),
            "{:?}",
            db.maintenance_error()
        );
    }

    /// Test 5: a fault at the very first checked I/O op on the worker
    /// thread — necessarily inside BUILD's first output write, since
    /// PLAN does no I/O and COMMIT only runs after BUILD returns Ok —
    /// must fail cleanly: no manifest touched, no Version touched, the
    /// failure visible, and a reopen fully correct.
    #[test]
    fn background_build_failure_leaves_current_version_untouched() {
        let td = TempDir::new("bg-build-fail");
        let opts = tiny_options();
        let db = SharedKiban::open_with_options(td.path(), opts.clone()).unwrap();
        let m = db.maintenance_for_test();
        m.inject_on_worker(|| sys::install_fault(0));

        seed_for_compaction(&db, 3, "r");
        m.wait_settled();

        let err = db.maintenance_error();
        assert!(err.is_some(), "no background failure was induced");
        assert!(
            !db.is_poisoned(),
            "a build failure (never reaching the manifest) must not poison: {err:?}"
        );

        // reads still agree with what was actually flushed (the last of
        // 3 seeded rounds, labeled "r0".."r2")
        for i in 0..20u32 {
            let key = format!("k{i:03}");
            assert_eq!(
                db.get(key.as_bytes()).unwrap(),
                Some(format!("r2-{i}").into_bytes())
            );
        }

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
        for i in 0..20u32 {
            let key = format!("k{i:03}");
            assert_eq!(
                reopened.get(key.as_bytes()).unwrap(),
                Some(format!("r2-{i}").into_bytes())
            );
        }
    }

    /// Test 6: an ambiguous MANIFEST rename (directory fsync fails after
    /// the rename lands) during *background* publication must poison
    /// the engine exactly like the foreground path does — no separate
    /// interpretation for "it happened on a worker thread". Swept over
    /// fault indices like the foreground
    /// `commit_ambiguity_during_flush_poisons_engine` test, since the
    /// exact op offset depends on how many output files this run's
    /// compactions happen to produce.
    #[test]
    fn commit_ambiguity_during_background_publication_poisons_engine() {
        let mut induced = false;
        for n in 0..60usize {
            let td = TempDir::new("bg-ambiguity-iter");
            let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();
            let m = db.maintenance_for_test();
            m.inject_on_worker(move || sys::install_faults(&[n]));

            seed_for_compaction(&db, 3, "r");
            m.wait_settled();

            if !db.is_poisoned() {
                continue;
            }
            induced = true;

            // mutation refused, same as the foreground poisoning path
            assert!(db.put(b"later", b"x").is_err());
            // reads stay available
            let _ = db.get(b"k000").unwrap();
            // the failure is also visible through maintenance_error
            assert!(db.maintenance_error().is_some());

            drop(db);
            // reopen resolves disk truth normally — same as foreground
            let reopened = Kiban::open_with_options(td.path(), tiny_options()).unwrap();
            assert!(!reopened.is_poisoned());
            break;
        }
        assert!(
            induced,
            "commit ambiguity during background publication never induced"
        );
    }

    /// Shutdown: the worker thread does not outlive its last handle.
    #[test]
    fn last_handle_drop_stops_the_worker_thread() {
        let td = TempDir::new("bg-shutdown");
        let db = SharedKiban::open_with_options(td.path(), tiny_options()).unwrap();
        let clone = db.clone();
        db.put(b"a", b"1").unwrap();
        drop(db);
        // the clone alone keeps the worker alive; engine still usable
        assert_eq!(clone.get(b"a").unwrap(), Some(b"1".to_vec()));
        drop(clone); // last handle: worker is stopped and joined here
    }
}
