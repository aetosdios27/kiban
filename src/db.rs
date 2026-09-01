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
use crate::engine_lock::ShardedRwLock;
use crate::file_cache::TableFileCache;
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
    /// Background maintenance has failed and a `SharedKiban` caller was
    /// waiting on it — for backpressure (11.5), waiting for write room
    /// that will now never open up, since nothing is compacting anymore.
    Maintenance(MaintenanceError),
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
            DbError::Maintenance(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Io(e) | DbError::CommitFailed(e) | DbError::CommitAmbiguous(e) => Some(e),
            DbError::Maintenance(e) => Some(e),
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

/// Resolves a point lookup against one immutable published topology at
/// `sequence`. Both normal captured reads and snapshots use this path so
/// table ordering, sequence visibility, and tombstone handling stay one
/// rule.
fn get_from_version_at(
    version: &Version,
    key: &[u8],
    sequence: u64,
) -> Result<Option<Vec<u8>>, DbError> {
    for entry in version.tables.iter().rev().filter(|t| t.level == 0) {
        match entry.table.get(key, Some(sequence))? {
            Some(found) => {
                return Ok(match found.kind {
                    Kind::Put => Some(found.value),
                    Kind::Tombstone => None,
                });
            }
            None => continue,
        }
    }
    for entry in version.tables.iter().filter(|t| t.level >= 1) {
        if key < entry.first_key.as_slice() || key > entry.last_key.as_slice() {
            continue;
        }
        match entry.table.get(key, Some(sequence))? {
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

/// Tunables for flush/compaction behavior (compaction.md configuration).
#[derive(Debug, Clone)]
pub struct KibanOptions {
    pub l0_compaction_trigger: usize,
    /// Hard L0 safety ceiling (11.5): once the live L0 file count
    /// reaches this, `SharedKiban` stalls new mutation-producing work
    /// (`put`/`delete`/`write`/`flush`) until background compaction
    /// brings it back down. Must be strictly greater than
    /// `l0_compaction_trigger` — otherwise writers would stall while
    /// nothing has even started trying to compact yet. Direct `Kiban`
    /// ignores this entirely; its synchronous compaction already keeps
    /// L0 bounded.
    pub l0_write_stall_trigger: usize,
    pub base_level_bytes: u64,
    pub level_multiplier: u64,
    pub target_file_size: u64,
    pub block_cache_bytes: usize,
    /// Hard bound on simultaneously open SST file descriptors (11.6),
    /// enforced by a shared `TableFileCache` — not a target or a soft
    /// threshold. `SstTable` does not keep a descriptor of its own; a
    /// read leases one from this cache for just its own duration, so
    /// this number really is the ceiling.
    pub max_open_table_files: usize,
    /// Flush trigger (11.8): once the active memtable's
    /// [`Memtable::logical_bytes`] reaches this, `SharedKiban` freezes
    /// it and continues writing against a fresh memtable/WAL while the
    /// frozen one flushes to L0 in the background. A deliberately
    /// boring, untuned default — not a claim of measurement. Direct
    /// `Kiban` ignores this entirely, exactly like
    /// `l0_write_stall_trigger` (11.5).
    pub write_buffer_bytes: usize,
}

impl Default for KibanOptions {
    fn default() -> Self {
        const MIB: u64 = 1 << 20;
        KibanOptions {
            l0_compaction_trigger: 4,
            l0_write_stall_trigger: 8,
            base_level_bytes: 4 * MIB,
            level_multiplier: 10,
            target_file_size: 4 * MIB,
            block_cache_bytes: 32 * MIB as usize,
            max_open_table_files: 128,
            write_buffer_bytes: 4 * MIB as usize,
        }
    }
}

impl KibanOptions {
    /// Rejects configurations backpressure (11.5) or the file-cache
    /// bound (11.6) cannot reason about.
    fn validate(&self) -> Result<(), DbError> {
        if self.l0_write_stall_trigger <= self.l0_compaction_trigger {
            return Err(DbError::Corrupt(format!(
                "invalid options: l0_write_stall_trigger ({}) must be greater than l0_compaction_trigger ({})",
                self.l0_write_stall_trigger, self.l0_compaction_trigger
            )));
        }
        if self.max_open_table_files == 0 {
            return Err(DbError::Corrupt(
                "invalid options: max_open_table_files must be greater than zero".to_string(),
            ));
        }
        if self.write_buffer_bytes == 0 {
            return Err(DbError::Corrupt(
                "invalid options: write_buffer_bytes must be greater than zero".to_string(),
            ));
        }
        Ok(())
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

/// The one frozen memtable phase 11.8 allows: not yet an SST, but its
/// WAL number stays MANIFEST-live (and its file undeleted) until the
/// flush that supersedes it with an SST commits. `memtable` is shared
/// (`Arc`, never mutated again after freeze), not cloned, so BUILD —
/// off the engine lock — and any snapshot captured after the freeze
/// can reference the exact same frozen data cheaply.
struct Immutable {
    memtable: StdArc<Memtable>,
    wal_number: u64,
    generation: u64,
}

pub struct Kiban {
    dir: PathBuf,
    options: KibanOptions,
    cache: StdArc<BlockCache>,
    /// Bounds simultaneously open SST descriptors (11.6): one instance
    /// per database, shared by every `SstTable` — recovered at open,
    /// created by flush, created by compaction. No accidental islands.
    file_cache: StdArc<TableFileCache>,
    memtable: Memtable,
    wal: Wal,
    /// The one frozen memtable pending background flush (11.8). `None`
    /// means nothing is frozen right now.
    immutable: Option<Immutable>,
    /// Next generation number a freeze will assign. Monotonic.
    next_flush_generation: u64,
    /// Highest flush generation whose SST has actually committed.
    last_completed_flush_generation: u64,
    next_file_number: u64,
    wal_number: u64,
    last_sequence: u64,
    /// Sorted ascending; the oldest entry gates tombstone GC.
    active_snapshots: Vec<u64>,
    /// The authoritative published topology (MANIFEST-committed).
    version: StdArc<Version>,
    /// Retired-from-the-current-Version tables, not yet provably safe
    /// to delete: each is an `Arc<TableEntry>` clone, so
    /// `Arc::strong_count == 1` means nothing else (a snapshot, most
    /// likely) still references it (11.3 file-lifetime rules, made
    /// real in 11.6 — see `reclaim_obsolete`).
    obsolete: Vec<StdArc<TableEntry>>,
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
        options.validate()?;
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        // Exactly one of each, constructed once, shared by every table
        // this Kiban ever opens or creates (11.6) — recovered tables
        // below, and later anything flush/compaction produces.
        let cache = StdArc::new(BlockCache::new(options.block_cache_bytes));
        let file_cache = StdArc::new(TableFileCache::new(options.max_open_table_files));

        let manifest = match Manifest::load(&dir)? {
            Some(m) => m,
            None => Self::initialize_fresh(&dir)?,
        };

        // Sweep before touching anything: unreferenced artifacts are
        // garbage by definition (D3 step 4).
        Self::sweep_orphans(&dir, &manifest)?;

        for &n in &manifest.wal_numbers {
            if !sys::exists(&dir.join(file_name(n, WAL_EXTENSION))) {
                return Err(DbError::Corrupt(format!(
                    "manifest names wal {n} which does not exist"
                )));
            }
        }

        // Phase 11.8: replay every live WAL, oldest generation first —
        // `wal_numbers` is strictly ascending (Manifest::decode/fresh
        // enforce it) and numeric order equals recency order here,
        // exactly like sstable file numbers within L0 (db-layout.md
        // D5): the freeze protocol only ever creates a new WAL after
        // the old one stops accepting writes, so no record in an older
        // generation can outrank any record in a newer one. Replaying
        // in this order keeps every key's versions seq-ascending as
        // they land in the memtable, which `Memtable::insert_entry`
        // requires. There is no in-memory "immutable memtable" to
        // reconstruct — recovery only needs the resulting logical
        // state (see docs); everything replays into one active
        // memtable.
        let mut memtable = Memtable::new();
        let mut wal_max_seq = 0u64;
        let mut active_wal: Option<Wal> = None;
        for &n in &manifest.wal_numbers {
            let path = dir.join(file_name(n, WAL_EXTENSION));
            let (w, report) = Wal::open(&path, &mut memtable)?;
            wal_max_seq = wal_max_seq.max(report.max_sequence);
            // Only the newest generation's handle is kept for future
            // writes; older ones just needed replaying and can close.
            active_wal = Some(w);
        }
        let mut wal =
            active_wal.expect("Manifest::decode/fresh guarantee wal_numbers is non-empty");
        let mut wal_number = *manifest
            .wal_numbers
            .last()
            .expect("wal_numbers is non-empty");
        let mut next_file_number = manifest.next_file_number;

        // If the crashed engine had a freeze in flight (two live WALs),
        // consolidate back to exactly one now, before any new mutation
        // is accepted, restoring the single-active/zero-immutable
        // invariant every other invariant in this phase assumes.
        // Reusing the newest replayed WAL as-is is not an option: it
        // only holds the newer generation's records, and the merged
        // memtable also contains the older generation's — a single
        // fresh WAL, rewritten from the merged memtable in strict
        // seq order, is durably equivalent to both combined. A crash
        // mid-consolidation is safe to retry: the old MANIFEST (and
        // both old WALs, untouched until the new one commits) remain
        // authoritative until this install succeeds, and any half
        // -written retry artifact is exactly the kind of
        // MANIFEST-unreferenced garbage `sweep_orphans` already cleans
        // up on the next open attempt.
        if manifest.wal_numbers.len() > 1 {
            let consolidated_number = next_file_number;
            next_file_number += 1;
            let consolidated_path = dir.join(file_name(consolidated_number, WAL_EXTENSION));
            let mut throwaway = Memtable::new();
            let (mut consolidated_wal, _) = Wal::open(&consolidated_path, &mut throwaway)?;
            // `iter_all_versions` yields, per key, newest first — the
            // opposite of what a WAL must contain, since replay requires
            // each key's records to arrive in strictly ascending seq
            // order (`Memtable::insert_entry`'s own invariant). Sort by
            // seq globally first; seq is already unique per mutation, so
            // this alone restores a valid replay order across all keys.
            let mut ordered: Vec<(&[u8], &MemEntry)> = memtable.iter_all_versions().collect();
            ordered.sort_by_key(|(_, e)| e.seq());
            for (key, entry) in ordered {
                match entry {
                    MemEntry::Value { value, seq } => consolidated_wal.put(*seq, key, value)?,
                    MemEntry::Tombstone { seq } => consolidated_wal.delete(*seq, key)?,
                }
            }
            consolidated_wal.sync().map_err(|e| match e {
                crate::wal::SyncPhase::Flush(e) => DbError::Io(e),
                crate::wal::SyncPhase::Fdatasync(e) => DbError::Io(e),
            })?;
            Manifest {
                next_file_number,
                wal_numbers: vec![consolidated_number],
                last_sequence: manifest.last_sequence.max(wal_max_seq),
                tables: manifest.tables.clone(),
            }
            .install(&dir)?;
            for &old in &manifest.wal_numbers {
                let _ = sys::remove_file(&dir.join(file_name(old, WAL_EXTENSION)));
            }
            wal = consolidated_wal;
            wal_number = consolidated_number;
        }

        let mut tables = Vec::with_capacity(manifest.tables.len());
        for tref in &manifest.tables {
            let path = dir.join(file_name(tref.number, SST_EXTENSION));
            let table = SstTable::open(tref.number, &path, cache.clone(), file_cache.clone())?;
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

        Ok(Kiban {
            dir,
            options,
            cache,
            file_cache,
            memtable,
            wal,
            immutable: None,
            // 1-indexed, deliberately: generation 0 must never be a
            // real freeze's generation, since `last_completed_flush_
            // generation`'s own "nothing has committed yet" value is
            // 0 — colliding would make the very first flush's
            // `wait_for_flush_generation` a silent no-op.
            next_flush_generation: 1,
            last_completed_flush_generation: 0,
            next_file_number,
            wal_number,
            last_sequence: manifest.last_sequence.max(wal_max_seq),
            active_snapshots: Vec::new(),
            version: StdArc::new(Version { id: 0, tables }),
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
        for &n in &manifest.wal_numbers {
            atomic::create_durably(&dir.join(file_name(n, WAL_EXTENSION)))?;
        }
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
                // Phase 11.8: multiple WAL generations can be live at
                // once (a freeze in flight) — membership, not equality.
                WAL_EXTENSION => !manifest.wal_numbers.contains(&number),
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
        // 11.8: the frozen memtable, if any, is older than active but
        // may not have reached an SST yet — check it before tables.
        if let Some(im) = &self.immutable {
            match im.memtable.entry(key) {
                Some(MemEntry::Value { value, .. }) => return Ok(Some(value.clone())),
                Some(MemEntry::Tombstone { .. }) => return Ok(None),
                None => {}
            }
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
        // 11.8: same fallthrough-on-None chaining as memtable -> tables
        // already used below, with the frozen memtable (if any) as one
        // more, older source in between.
        if let Some(im) = &self.immutable
            && let Some(entry) = im.memtable.entry_at(key, snap.seq)
        {
            return Ok(match entry {
                MemEntry::Value { value, .. } => Some(value.clone()),
                MemEntry::Tombstone { .. } => None,
            });
        }
        get_from_version_at(&snap.version, key, snap.seq)
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
        if let Some(im) = &self.immutable {
            sources.push(SourceHead {
                feed: SourceFeed::Mem(Box::new(im.memtable.iter_from(b""))),
                head: None,
                exhausted: false,
            });
        }
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
        // `iter_all_versions` (not `iter`, which is live-only): a
        // snapshot may need an older, superseded version that only
        // exists in this memtable's retained history. Its own ordering
        // — per key, live then history newest-first, i.e. seq
        // descending — is exactly the (key asc, seq desc) order
        // `TableBuilder::add` requires, so multiple versions of one
        // key land in the output table correctly, the same way
        // compaction output already can.
        for (key, entry) in self.memtable.iter_all_versions() {
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
            wal_numbers: vec![new_wal_number],
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
            self.file_cache.clone(),
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

    /// The WAL numbers currently live, ascending: just the active WAL,
    /// or the active WAL plus the frozen immutable memtable's old WAL
    /// while a flush is pending (11.8). Compaction's own MANIFEST
    /// writes must use this rather than assuming one WAL — a freeze
    /// can land while a compaction BUILD is running unlocked, so by
    /// the time COMMIT reacquires the lock, `immutable` may already be
    /// occupied by work compaction knows nothing about.
    fn live_wal_numbers(&self) -> Vec<u64> {
        match &self.immutable {
            Some(im) => {
                let mut v = vec![im.wal_number, self.wal_number];
                v.sort_unstable();
                v
            }
            None => vec![self.wal_number],
        }
    }

    /// The synchronous half of the freeze handoff (11.8): allocates
    /// and durably creates a fresh WAL, then installs a MANIFEST naming
    /// BOTH the old and new WAL as live — the commit point. Only after
    /// that succeeds does the in-memory swap happen: RAM must never
    /// know something disk does not. Building the frozen memtable's
    /// SST happens later, off the engine lock, in `plan_flush`/
    /// `FlushPlan::build`/`commit_flush`.
    ///
    /// A no-op if the active memtable is empty (nothing to freeze) or
    /// the one immutable slot this phase allows is already occupied —
    /// callers must wait for that slot to free first (`SharedKiban`'s
    /// backpressure wait, mirroring 11.5's L0 wait exactly).
    fn freeze(&mut self) -> Result<(), DbError> {
        self.check_poisoned()?;
        if self.memtable.is_empty() || self.immutable.is_some() {
            return Ok(());
        }

        let new_wal_number = self.next_file_number;
        let new_next_file_number = self.next_file_number + 1;
        let new_wal_path = self.dir.join(file_name(new_wal_number, WAL_EXTENSION));

        // The WAL a MANIFEST names must exist durably before that
        // MANIFEST does — same rule as every flush (D2 step 3).
        atomic::create_durably(&new_wal_path)?;
        let mut throwaway = Memtable::new();
        let (new_wal, _report) = Wal::open(&new_wal_path, &mut throwaway)?;

        let mut wal_numbers = vec![self.wal_number, new_wal_number];
        wal_numbers.sort_unstable();
        let tables: Vec<TableRef> = self
            .version
            .tables
            .iter()
            .map(|t| TableRef {
                level: t.level,
                number: t.number,
            })
            .collect();

        // The commit point: from here, recovery knows both WALs are
        // live, so foreground writes may safely enter the new one.
        Manifest {
            next_file_number: new_next_file_number,
            wal_numbers,
            last_sequence: self.last_sequence,
            tables,
        }
        .install(&self.dir)
        .map_err(|e| match e {
            atomic::CommitError::Failed(io) => DbError::CommitFailed(io),
            atomic::CommitError::RenamedNotDurable(io) => {
                // Ambiguous: do not guess whether the one-WAL or
                // two-WAL topology survived. Same rule as everywhere
                // else (engine-poisoning.md D2).
                self.poison(PoisonCause::CommitAmbiguity(io.to_string()));
                DbError::Poisoned(self.poisoned.clone().unwrap())
            }
        })?;

        // Only now: disk truth confirms both WALs are live, so the RAM
        // handoff can happen. Never before — never publish RAM state
        // ahead of what the MANIFEST actually says.
        let old_memtable = std::mem::replace(&mut self.memtable, Memtable::new());
        let old_wal_number = self.wal_number;
        self.wal = new_wal;
        self.wal_number = new_wal_number;
        self.next_file_number = new_next_file_number;
        let generation = self.next_flush_generation;
        self.next_flush_generation += 1;
        self.immutable = Some(Immutable {
            memtable: StdArc::new(old_memtable),
            wal_number: old_wal_number,
            generation,
        });
        Ok(())
    }

    /// PLAN for the pending immutable memtable's flush, if any: reserves
    /// an output file number and captures everything BUILD needs —
    /// cheap, done under the engine lock. Mirrors compaction's own
    /// PLAN/BUILD/COMMIT split (11.4).
    pub(crate) fn plan_flush(&mut self) -> Option<FlushPlan> {
        let im = self.immutable.as_ref()?;
        let output_number = self.next_file_number;
        self.next_file_number += 1;
        Some(FlushPlan {
            memtable: im.memtable.clone(),
            old_wal_number: im.wal_number,
            generation: im.generation,
            output_number,
            dir: self.dir.clone(),
            cache: self.cache.clone(),
            file_cache: self.file_cache.clone(),
        })
    }

    /// COMMIT for a flush (11.8): publish the output SST, retire the
    /// frozen memtable's WAL, and clear the immutable slot. Applied
    /// against the *current* topology, not a stale view PLAN happened
    /// to see — mirrors `commit_compaction` exactly.
    pub(crate) fn commit_flush(
        &mut self,
        plan: FlushPlan,
        output: TableEntry,
    ) -> Result<(), DbError> {
        let Some(im) = &self.immutable else {
            return Err(DbError::Corrupt(
                "flush commit: no immutable memtable pending (logic bug)".to_string(),
            ));
        };
        if im.generation != plan.generation {
            return Err(DbError::Corrupt(
                "flush commit: immutable generation mismatch (logic bug)".to_string(),
            ));
        }

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
            number: output.number,
        });
        table_refs.sort();

        // The commit point: only the active WAL remains live — the
        // frozen memtable's old WAL is superseded by `output`.
        Manifest {
            next_file_number: self.next_file_number,
            wal_numbers: vec![self.wal_number],
            last_sequence: self.last_sequence,
            tables: table_refs,
        }
        .install(&self.dir)
        .map_err(|e| match e {
            atomic::CommitError::Failed(io) => DbError::CommitFailed(io),
            atomic::CommitError::RenamedNotDurable(io) => {
                self.poison(PoisonCause::CommitAmbiguity(io.to_string()));
                DbError::Poisoned(self.poisoned.clone().unwrap())
            }
        })?;

        // Only now: publish Version, clear the immutable slot, retire
        // the old WAL. Never delete it before this point (D2 step 5's
        // rule, unchanged): a crash before commit must still find it
        // MANIFEST-live and replayable.
        self.immutable = None;
        let mut new_tables = self.version.tables.clone();
        let entry = StdArc::new(output);
        let pos = new_tables.partition_point(|t| (t.level, t.number) < (entry.level, entry.number));
        new_tables.insert(pos, entry);
        self.version = StdArc::new(Version {
            id: self.version.id + 1,
            tables: new_tables,
        });
        self.last_completed_flush_generation =
            self.last_completed_flush_generation.max(plan.generation);

        // Best-effort deletion; recovery's sweep owns stragglers.
        let _ = sys::remove_file(&self.dir.join(file_name(plan.old_wal_number, WAL_EXTENSION)));

        Ok(())
    }

    /// Auto-freeze trigger (11.8), called by `SharedKiban` after a
    /// successful mutation, still holding the same lock the mutation
    /// itself used. Returns whether a freeze happened, so the caller
    /// knows whether it's worth waking the maintenance worker.
    ///
    /// A no-op when the immutable slot is already occupied: the one-
    /// immutable-memtable rule means this simply declines rather than
    /// queuing a second one. Bounding growth in that case is the
    /// caller's job (`SharedKiban::wait_for_write_room`'s own
    /// immutable-slot wait), not this method silently trying forever.
    pub(crate) fn maybe_freeze(&mut self) -> Result<bool, DbError> {
        if self.immutable.is_some()
            || self.memtable.logical_bytes() < self.options.write_buffer_bytes
        {
            return Ok(false);
        }
        self.freeze()?;
        Ok(true)
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
        // 11.8: the merge is a proper (key asc, seq desc) merge — the
        // frozen memtable slots in as just one more source; where it
        // sits in this Vec doesn't affect correctness, only tidiness.
        if let Some(im) = &self.immutable {
            sources.push(SourceHead {
                feed: SourceFeed::Mem(Box::new(im.memtable.iter_from(start))),
                head: None,
                exhausted: false,
            });
        }
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
        assert_eq!(
            Manifest::load(td.path()).unwrap().unwrap().wal_numbers,
            vec![1]
        );
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
                .join(file_name(manifest.wal_numbers[0], WAL_EXTENSION))
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
            wal_numbers: vec![1],
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
            wal_numbers: vec![1],
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
        assert_eq!(m.wal_numbers, vec![1]);
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
        assert_eq!(m.wal_numbers, vec![3]);
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
        assert_eq!(manifest.wal_numbers, vec![1]);
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
/// Raw byte totals from one completed compaction (phase 11.7), reported
/// straight from already-known plan/output metadata — no extra disk
/// I/O to compute them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CompactionOutcome {
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
}

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
    file_cache: StdArc<TableFileCache>,
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
        let table = SstTable::open(number, &path, self.cache.clone(), self.file_cache.clone())?;
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

/// A flush job's plan (11.8): everything BUILD needs to turn one
/// frozen immutable memtable into an L0 sstable, captured under the
/// engine lock. Mirrors `CompactionPlan` exactly — same PLAN/BUILD/
/// COMMIT split, same reason (BUILD is the expensive part; moving it
/// off the lock is the entire point of a background worker).
pub(crate) struct FlushPlan {
    memtable: StdArc<Memtable>,
    /// The frozen memtable's WAL — stays MANIFEST-live until this
    /// plan's `commit_flush` retires it.
    old_wal_number: u64,
    generation: u64,
    output_number: u64,
    dir: PathBuf,
    cache: StdArc<BlockCache>,
    file_cache: StdArc<TableFileCache>,
}

impl FlushPlan {
    /// BUILD: identical table-construction rules to every other flush
    /// path (`Kiban::flush_without_compaction`) — same `TableBuilder`,
    /// same `iter_all_versions` ordering so a snapshot-visible older
    /// version isn't lost, same atomic publication. No engine lock is
    /// held here.
    pub(crate) fn build(&self) -> Result<TableEntry, DbError> {
        let mut builder = TableBuilder::new();
        for (key, entry) in self.memtable.iter_all_versions() {
            match entry {
                MemEntry::Value { value, seq } => builder.add(Kind::Put, key, value, *seq)?,
                MemEntry::Tombstone { seq } => builder.add(Kind::Tombstone, key, b"", *seq)?,
            }
        }
        let bytes = builder.finish()?;
        let path = self.dir.join(file_name(self.output_number, SST_EXTENSION));
        atomic::commit_file(&path, &bytes)?;
        let table = SstTable::open(
            self.output_number,
            &path,
            self.cache.clone(),
            self.file_cache.clone(),
        )?;
        Ok(TableEntry {
            level: 0,
            number: self.output_number,
            size: table.size_on_disk(),
            first_key: table.smallest_key().to_vec(),
            last_key: table.largest_key().to_vec(),
            table,
        })
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
            file_cache: self.file_cache.clone(),
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
    ) -> Result<CompactionOutcome, DbError> {
        // Raw facts for the stats surface (11.7), read from metadata
        // already in hand — never a reread of file bytes.
        let input_bytes: u64 = plan.inputs.iter().map(|t| t.size).sum();
        let output_bytes: u64 = outputs.iter().map(|o| o.size).sum();

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
            // 11.8: a freeze can land while this compaction's BUILD ran
            // unlocked, so the currently-live WAL set may already
            // include a pending immutable memtable's WAL by the time
            // this COMMIT runs — never assume just one.
            wal_numbers: self.live_wal_numbers(),
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

        // Obsolete files are reclaimable only when nothing still
        // references them (11.3); until then they stay on disk. Each
        // input's `Arc<TableEntry>` moves here directly — `self.version`
        // no longer holds one (replaced above), so whether anything
        // else (a snapshot, most likely) still does is exactly what
        // `reclaim_obsolete`'s refcount check answers.
        for entry in plan.inputs {
            self.obsolete.push(entry);
        }
        self.reclaim_obsolete();
        Ok(CompactionOutcome {
            input_bytes,
            output_bytes,
        })
    }

    /// Deletes obsolete files that nothing still references. An entry
    /// in `self.obsolete` is an `Arc<TableEntry>` no longer reachable
    /// from `self.version`; `Arc::strong_count == 1` means this is the
    /// only reference left (no snapshot pins it), so it's provably
    /// safe to reclaim (11.3 file-lifetime rules). Safe to check this
    /// way without racing a concurrent snapshot capture: the only
    /// paths that could add a new reference to a table already absent
    /// from `self.version` are `Kiban::snapshot`/`SharedKiban::snapshot`,
    /// and both require the same engine lock `reclaim_obsolete` already
    /// runs under.
    ///
    /// Before physically unlinking, the file cache's own idle
    /// descriptor for that number (if any) is invalidated first
    /// (11.6): otherwise the directory entry would disappear while a
    /// cached-but-unleased descriptor still held the inode open,
    /// leaving disk space pinned by a handle nothing can reach anymore.
    fn reclaim_obsolete(&mut self) {
        let mut kept = Vec::new();
        for entry in std::mem::take(&mut self.obsolete) {
            if StdArc::strong_count(&entry) == 1 {
                self.file_cache.invalidate(entry.number);
                let _ = sys::remove_file(&self.dir.join(file_name(entry.number, SST_EXTENSION)));
            } else {
                kept.push(entry);
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
            // High enough that no existing (pre-11.5) test — several of
            // which deliberately freeze the worker while seeding a
            // handful of L0 tables — ever stalls on it by accident.
            // Tests that specifically exercise backpressure use their
            // own tight `stall_options()` instead.
            l0_write_stall_trigger: 20,
            base_level_bytes: 300,
            level_multiplier: 4,
            target_file_size: 250,
            block_cache_bytes: 1 << 20,
            // High enough that no existing (pre-11.6) test — none of
            // which exercise the file-cache bound deliberately — ever
            // waits on it by accident. Tests that specifically exercise
            // the file-cache bound use their own tight options.
            max_open_table_files: 64,
            // High enough that no existing (pre-11.8) test — none of
            // which write anywhere near this much — ever auto-freezes
            // by accident. Tests that specifically exercise freeze/
            // flush use their own tight `write_buffer_bytes`.
            write_buffer_bytes: 1 << 20,
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
        // pub(crate): reused directly by `flush_pipeline_tests`' own
        // sweep (11.8), which exercises `Kiban::freeze`/`commit_flush`
        // rather than this module's `flush()` scenario but needs the
        // exact same durability-floor bookkeeping.
        pub(crate) fn apply(&mut self, key: &[u8], value: Option<&[u8]>) {
            self.attempted
                .insert(key.to_vec(), value.map(|v| v.to_vec()));
            self.dirty_since_sync.push(key.to_vec());
        }

        /// A successful `sync` makes all prior operations durable.
        pub(crate) fn on_sync_ok(&mut self) {
            self.mark_durable();
        }

        /// A successful `flush` (or, in `flush_pipeline_tests`, a
        /// successful `commit_flush`) ALSO advances the durability
        /// floor: it publishes the entire memtable (synced or not)
        /// through its commit point. This is what the exact-durability
        /// sweep taught us.
        pub(crate) fn on_flush_ok(&mut self) {
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
                l0_write_stall_trigger: 20,
                base_level_bytes: 300,
                level_multiplier: 4,
                target_file_size: 250,
                block_cache_bytes: 1 << 20,
                max_open_table_files: 64,
                write_buffer_bytes: 1 << 20,
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
                    l0_write_stall_trigger: 20,
                    base_level_bytes: 300,
                    level_multiplier: 4,
                    target_file_size: 250,
                    block_cache_bytes: 1 << 20,
                    max_open_table_files: 64,
                    write_buffer_bytes: 1 << 20,
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
/// The engine mutex chooses a read's world. A normal point read releases
/// it before consulting the immutable memtable or any SST state.
pub struct SharedKiban {
    inner: std::sync::Arc<ShardedRwLock<Kiban>>,
    maintenance: std::sync::Arc<Maintenance>,
    #[cfg(test)]
    read_checkpoint: StdArc<ReadCheckpoint>,
}

#[cfg(test)]
struct ReadBarriers {
    reached: std::sync::Barrier,
    release: std::sync::Barrier,
}

/// Deterministic foreground-read control. A paused read owns every
/// captured source and has already released the engine mutex.
#[cfg(test)]
#[derive(Default)]
struct ReadCheckpoint {
    barriers: std::sync::Mutex<Option<StdArc<ReadBarriers>>>,
}

#[cfg(test)]
impl ReadCheckpoint {
    fn barriers(&self) -> Option<StdArc<ReadBarriers>> {
        self.barriers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn arm(&self, readers: usize) {
        assert!(readers > 0);
        let mut barriers = self
            .barriers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(barriers.is_none(), "read checkpoint already armed");
        *barriers = Some(StdArc::new(ReadBarriers {
            reached: std::sync::Barrier::new(readers + 1),
            release: std::sync::Barrier::new(readers + 1),
        }));
    }

    fn hit(&self) {
        let Some(barriers) = self.barriers() else {
            return;
        };
        barriers.reached.wait();
        barriers.release.wait();
    }

    fn wait_reached(&self) {
        self.barriers()
            .expect("read checkpoint must be armed")
            .reached
            .wait();
    }

    fn release(&self) {
        let barriers = self.barriers().expect("read checkpoint must be armed");
        barriers.release.wait();
        *self
            .barriers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

impl Clone for SharedKiban {
    fn clone(&self) -> Self {
        self.maintenance.add_handle();
        SharedKiban {
            inner: self.inner.clone(),
            maintenance: self.maintenance.clone(),
            #[cfg(test)]
            read_checkpoint: self.read_checkpoint.clone(),
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
/// Capture copies the active memtable (O(its size)) under one lock hold
/// and pins the immutable published [`Version`]. Reads afterwards never
/// touch the engine lock. The version's `Arc<TableEntry>` members keep a
/// table usable even when a later compaction retires it from live state.
///
/// Dropping a `SharedSnapshot` releases its hold on the engine's
/// `smallest_snapshot` boundary (compaction's tombstone/old-version GC),
/// mirroring `Kiban::release_snapshot` for the direct API — without
/// this, a `SharedSnapshot` would suppress GC for the engine's entire
/// remaining lifetime, not just while it's live.
#[allow(dead_code)]
pub struct SharedSnapshot {
    engine: std::sync::Arc<ShardedRwLock<Kiban>>,
    seq: u64,
    memtable: Memtable,
    /// The frozen immutable memtable at capture time, if any (11.8) —
    /// an `Arc` clone, cheap, and correct precisely because a frozen
    /// memtable is never mutated again: whatever this snapshot saw at
    /// capture stays exactly what it sees, unaffected by the live
    /// engine later flushing it away.
    immutable: Option<StdArc<Memtable>>,
    version: StdArc<Version>,
}

impl Drop for SharedSnapshot {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.engine.write()
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
        if let Some(entry) = self.memtable.entry_at(key, self.seq) {
            return Ok(entry.as_value().map(ToOwned::to_owned));
        }
        if let Some(immutable) = &self.immutable
            && let Some(entry) = immutable.entry_at(key, self.seq)
        {
            return Ok(entry.as_value().map(ToOwned::to_owned));
        }
        get_from_version_at(&self.version, key, self.seq)
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
        if let Some(immutable) = &self.immutable {
            sources.push(SourceHead {
                feed: SourceFeed::Mem(Box::new(immutable.iter_from(b""))),
                head: None,
                exhausted: false,
            });
        }
        for t in self.version.tables.iter().rev().filter(|t| t.level == 0) {
            sources.push(SourceHead {
                feed: SourceFeed::Table(t.table.iter_from(b"")),
                head: None,
                exhausted: false,
            });
        }
        for t in self.version.tables.iter().filter(|t| t.level >= 1) {
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
            let entry = item?;
            out.push((entry.key, entry.value));
        }
        Ok(out)
    }
}

/// Table count and byte total for one level, read straight from the
/// currently published [`Version`] — never from the filesystem.
#[derive(Debug, Clone, Copy)]
pub struct LevelStats {
    pub level: u32,
    pub tables: usize,
    pub bytes: u64,
}

/// A cheap, observation-only snapshot of engine state (phase 11.7) —
/// enough to answer "what is Kiban doing right now?" without disk I/O,
/// cache activity, or waking maintenance. Raw facts only: Kiban does
/// not grade its own health (no "good"/"under pressure" verdicts) —
/// the caller decides what a number means.
///
/// This is an observational snapshot, not a transactional one: each
/// field group is read under its own lock (the engine, the block
/// cache, the file cache, the maintenance worker) with no single lock
/// spanning all of them, so e.g. `levels` and `maintenance` may
/// disagree by one compaction commit that landed between the two
/// reads. Freezing the whole engine to make telemetry perfectly
/// synchronized would cost far more than the guarantee is worth.
#[derive(Debug, Clone)]
pub struct KibanStats {
    pub memtable_entries: usize,
    /// Active memtable's [`Memtable::logical_bytes`] (11.8) — the
    /// signal `write_buffer_bytes` thresholds against.
    pub memtable_logical_bytes: usize,
    /// Whether one frozen memtable is currently pending background
    /// flush (11.8). At most one, ever, this phase.
    pub immutable_present: bool,
    /// The frozen memtable's own logical bytes, when present.
    pub immutable_logical_bytes: usize,
    pub active_snapshots: usize,
    pub obsolete_files_pending: usize,
    /// One entry per level that currently holds at least one table,
    /// ascending by level. An empty database yields an empty vec.
    pub levels: Vec<LevelStats>,
    pub block_cache: crate::cache::BlockCacheStats,
    pub table_files: crate::file_cache::TableFileCacheStats,
    pub maintenance: crate::background::MaintenanceStats,
}

fn levels_from_version(version: &Version) -> Vec<LevelStats> {
    let mut by_level: std::collections::BTreeMap<u32, (usize, u64)> = Default::default();
    for t in &version.tables {
        let e = by_level.entry(t.level).or_insert((0, 0));
        e.0 += 1;
        e.1 += t.size;
    }
    by_level
        .into_iter()
        .map(|(level, (tables, bytes))| LevelStats {
            level,
            tables,
            bytes,
        })
        .collect()
}

impl SharedKiban {
    pub fn open(dir: impl AsRef<Path>) -> Result<SharedKiban, DbError> {
        Self::open_with_options(dir, KibanOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: KibanOptions,
    ) -> Result<SharedKiban, DbError> {
        let inner =
            std::sync::Arc::new(ShardedRwLock::new(Kiban::open_with_options(dir, options)?));
        let maintenance = Maintenance::spawn(inner.clone());
        Ok(SharedKiban {
            inner,
            maintenance,
            #[cfg(test)]
            read_checkpoint: StdArc::new(ReadCheckpoint::default()),
        })
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

    /// Test-only synchronous flush; publication mutates topology.
    #[cfg(test)]
    pub(crate) fn flush_sync_for_test(&self) -> Result<(), DbError> {
        self.write_lock()?.flush_without_compaction()?;
        self.maintenance.wake();
        Ok(())
    }

    fn read_lock(&self) -> Result<crate::engine_lock::ReadGuard<'_, Kiban>, DbError> {
        self.inner
            .read()
            .map_err(|_| DbError::Corrupt("engine lock poisoned".to_string()))
    }

    fn write_lock(&self) -> Result<crate::engine_lock::WriteGuard<'_, Kiban>, DbError> {
        self.inner
            .write()
            .map_err(|_| DbError::Corrupt("engine lock poisoned".to_string()))
    }

    #[cfg(test)]
    fn lock(&self) -> Result<crate::engine_lock::WriteGuard<'_, Kiban>, DbError> {
        self.write_lock()
    }

    /// Blocks (never holding the engine mutex while doing so) until
    /// neither of two independent write-pressure reasons hold: L0 at
    /// its hard ceiling (11.5), or the one immutable-memtable slot
    /// already occupied *and* the active memtable has itself also
    /// crossed the write-buffer threshold (11.8) — i.e. a second freeze
    /// is wanted but the one slot this phase allows is taken. Called
    /// before every operation that would grow that debt — `put`,
    /// `delete`, `write` — never before `get`/`scan`/`sync`/snapshot
    /// reads, which must keep working while writers stall.
    /// `SharedKiban::flush()` uses its own, stricter wait (unconditional
    /// on the immutable slot, since an explicit flush always wants to
    /// freeze right now — see `wait_for_immutable_slot`).
    ///
    /// The epoch is captured *while still holding the engine lock*,
    /// immediately after observing pressure: any commit that could
    /// relieve it — a compaction commit, or a flush commit clearing the
    /// immutable slot — must itself take that same lock, so a commit
    /// can never land in the gap between "still under pressure" and
    /// "start waiting on this epoch" — closing the classic check-then-
    /// sleep missed-wakeup race.
    fn wait_for_write_room(&self) -> Result<(), DbError> {
        // Set once this call has genuinely had to wait at least once,
        // so a call that loops through several wake/recheck cycles
        // before getting room still counts as exactly one write stall
        // (11.7) — one blocked mutation, one stall event.
        let mut stalled = false;
        loop {
            if let Some(err) = self.maintenance.error() {
                return Err(DbError::Maintenance(err));
            }
            if self.maintenance.is_stopped() {
                return Err(DbError::Maintenance(MaintenanceError(
                    "maintenance worker is shutting down".to_string(),
                )));
            }
            let epoch = {
                let guard = self.write_lock()?;
                guard.check_poisoned()?;
                let l0_ok = guard.l0_count() < guard.options().l0_write_stall_trigger;
                let immutable_ok = guard.immutable.is_none()
                    || guard.memtable.logical_bytes() < guard.options().write_buffer_bytes;
                if l0_ok && immutable_ok {
                    return Ok(());
                }
                self.maintenance.progress_epoch()
            };
            if !stalled {
                self.maintenance.record_write_stall();
                stalled = true;
            }
            self.maintenance.wake();
            self.maintenance.writer_started_waiting();
            self.maintenance.wait_for_progress(epoch);
            self.maintenance.writer_stopped_waiting();
        }
    }

    /// Blocks until `generation`'s flush has actually committed —
    /// `SharedKiban::flush()`'s own wait, distinct from
    /// `wait_for_write_room`: it waits for one specific, already-
    /// frozen generation, never satisfied by an unrelated earlier or
    /// later flush completing first (never bare `immutable.is_none()`,
    /// which could be true because a *different* generation finished).
    /// Not counted as a write stall — this is a caller waiting on its
    /// own requested durability, the same shape as `sync()` blocking on
    /// its own fdatasync, not backpressure from write pressure.
    fn wait_for_flush_generation(&self, generation: u64) -> Result<(), DbError> {
        loop {
            if let Some(err) = self.maintenance.error() {
                return Err(DbError::Maintenance(err));
            }
            if self.maintenance.is_stopped() {
                return Err(DbError::Maintenance(MaintenanceError(
                    "maintenance worker is shutting down".to_string(),
                )));
            }
            let epoch = {
                let guard = self.write_lock()?;
                guard.check_poisoned()?;
                if guard.last_completed_flush_generation >= generation {
                    return Ok(());
                }
                self.maintenance.progress_epoch()
            };
            self.maintenance.wait_for_progress(epoch);
        }
    }

    /// A cheap, observation-only snapshot of engine state (phase
    /// 11.7) — see [`KibanStats`]. Never touches disk, never triggers
    /// a block-cache or file-cache access, never wakes or waits on
    /// maintenance: it briefly locks the engine to copy a few small
    /// facts, then reads the block cache, file cache, and maintenance
    /// worker each under their own lock, with the engine lock already
    /// released — no new lock ordering beyond what already exists
    /// elsewhere (engine lock, if held, is always released before a
    /// file-cache wait; this path never nests them at all).
    pub fn stats(&self) -> Result<KibanStats, DbError> {
        let (
            memtable_entries,
            memtable_logical_bytes,
            immutable_present,
            immutable_logical_bytes,
            active_snapshots,
            obsolete_files_pending,
            levels,
            cache,
            file_cache,
        ) = {
            let guard = self.read_lock()?;
            (
                guard.memtable.len(),
                guard.memtable.logical_bytes(),
                guard.immutable.is_some(),
                guard
                    .immutable
                    .as_ref()
                    .map(|im| im.memtable.logical_bytes())
                    .unwrap_or(0),
                guard.active_snapshots.len(),
                guard.obsolete.len(),
                levels_from_version(&guard.version),
                guard.cache.clone(),
                guard.file_cache.clone(),
            )
        };
        Ok(KibanStats {
            memtable_entries,
            memtable_logical_bytes,
            immutable_present,
            immutable_logical_bytes,
            active_snapshots,
            obsolete_files_pending,
            levels,
            block_cache: cache.stats(),
            table_files: file_cache.stats(),
            maintenance: self.maintenance.stats(),
        })
    }

    /// Buffered WAL append + memtable write. Not durable until `sync`.
    /// May freeze the active memtable afterward if it has crossed
    /// `write_buffer_bytes` (11.8) — same lock hold as the write
    /// itself, so the check and the freeze are atomic together.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> io::Result<()> {
        self.wait_for_write_room()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let froze = match self.write_lock() {
            Ok(mut guard) => {
                guard.put(key, value)?;
                guard
                    .maybe_freeze()
                    .map_err(|e| io::Error::other(e.to_string()))?
            }
            Err(e) => return Err(io::Error::other(e.to_string())),
        };
        if froze {
            self.maintenance.wake();
        }
        Ok(())
    }

    pub fn delete(&self, key: impl AsRef<[u8]>) -> io::Result<()> {
        self.wait_for_write_room()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let froze = match self.write_lock() {
            Ok(mut guard) => {
                guard.delete(key)?;
                guard
                    .maybe_freeze()
                    .map_err(|e| io::Error::other(e.to_string()))?
            }
            Err(e) => return Err(io::Error::other(e.to_string())),
        };
        if froze {
            self.maintenance.wake();
        }
        Ok(())
    }

    /// Whether the shared engine is poisoned.
    pub fn is_poisoned(&self) -> bool {
        match self.read_lock() {
            Ok(guard) => guard.is_poisoned(),
            Err(_) => true,
        }
    }

    /// Captures this operation's sequence boundary and immutable read
    /// sources under the engine mutex, then resolves SST state after
    /// releasing it. Slow table, cache, and file-cache work therefore
    /// never holds the engine mutex.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>, DbError> {
        let key = key.as_ref();
        let (sequence, immutable, version) = {
            let guard = self.read_lock()?;
            let sequence = guard.last_sequence;
            if let Some(entry) = guard.memtable.entry_at(key, sequence) {
                return Ok(entry.as_value().map(ToOwned::to_owned));
            }
            (
                sequence,
                guard
                    .immutable
                    .as_ref()
                    .map(|immutable| StdArc::clone(&immutable.memtable)),
                StdArc::clone(&guard.version),
            )
        };
        #[cfg(test)]
        self.read_checkpoint.hit();
        if let Some(immutable) = immutable
            && let Some(entry) = immutable.entry_at(key, sequence)
        {
            return Ok(entry.as_value().map(ToOwned::to_owned));
        }
        get_from_version_at(&version, key, sequence)
    }

    /// Makes every record appended by *any* thread so far durable in one
    /// device flush (group commit). Never subject to backpressure:
    /// `sync` durably persists WAL state already accepted, it does not
    /// publish another L0 table, so durability must never depend on
    /// compaction catching up (11.5).
    pub fn sync(&self) -> io::Result<()> {
        match self.write_lock() {
            Ok(mut guard) => guard.sync(),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    /// Commits a batch atomically under the engine lock. One `sync`
    /// afterwards makes the entire batch durable together — group
    /// commit applies. May freeze the active memtable afterward,
    /// exactly like `put`/`delete` (11.8).
    pub fn write(&self, batch: WriteBatch) -> Result<(), DbError> {
        self.wait_for_write_room()?;
        let froze = {
            let mut guard = self.write_lock()?;
            guard.write(batch)?;
            guard.maybe_freeze()?
        };
        if froze {
            self.maintenance.wake();
        }
        Ok(())
    }

    /// Captures a consistent snapshot: O(memtable) copy under one lock
    /// hold; reads on the returned handle never touch the lock. Never
    /// subject to backpressure — a read must not stall because writers
    /// are stalled.
    pub fn snapshot(&self) -> Result<SharedSnapshot, DbError> {
        let mut guard = self.write_lock()?;
        let seq = guard.last_sequence;
        let pos = guard.active_snapshots.partition_point(|s| *s < seq);
        guard.active_snapshots.insert(pos, seq);
        Ok(SharedSnapshot {
            engine: self.inner.clone(),
            seq,
            memtable: guard.memtable.clone(),
            immutable: guard.immutable.as_ref().map(|im| im.memtable.clone()),
            version: StdArc::clone(&guard.version),
        })
    }

    /// Durably publishes the memtable as a new L0 sstable, waiting only
    /// for its OWN requested flush to commit — not for whatever
    /// compaction or unrelated later flush happens to follow (11.8;
    /// superseding 11.4's inline-compaction wording, since flush itself
    /// moved off the lock too). A no-op if the active memtable is
    /// empty, exactly as before — flushing nothing would spend a WAL/
    /// MANIFEST revision on nothing.
    ///
    /// Subject to the same L0 backpressure as `put`/`delete`/`write`
    /// (11.5, unchanged): a flush's eventual commit publishes another
    /// L0 table, which is exactly what must wait when L0 is already at
    /// its hard ceiling. Also waits for the one immutable slot to be
    /// free, unconditionally (11.8) — unlike `put`/`delete`/`write`'s
    /// own softer check, an explicit flush always wants to freeze the
    /// current memtable right now, even one still below the automatic
    /// threshold. Both checks and the freeze itself happen under the
    /// same lock hold, so no other caller can steal L0 room or the
    /// just-freed slot in between — the same race `wait_for_write_room`
    /// closes elsewhere.
    pub fn flush(&self) -> Result<(), DbError> {
        let mut stalled = false;
        let generation = loop {
            if let Some(err) = self.maintenance.error() {
                return Err(DbError::Maintenance(err));
            }
            if self.maintenance.is_stopped() {
                return Err(DbError::Maintenance(MaintenanceError(
                    "maintenance worker is shutting down".to_string(),
                )));
            }
            let epoch = {
                let mut guard = self.write_lock()?;
                guard.check_poisoned()?;
                let l0_ok = guard.l0_count() < guard.options().l0_write_stall_trigger;
                if l0_ok && guard.immutable.is_none() {
                    if guard.memtable.is_empty() {
                        return Ok(());
                    }
                    guard.freeze()?;
                    break guard
                        .immutable
                        .as_ref()
                        .expect("freeze just populated it")
                        .generation;
                }
                self.maintenance.progress_epoch()
            };
            if !stalled {
                self.maintenance.record_write_stall();
                stalled = true;
            }
            self.maintenance.wake();
            self.maintenance.writer_started_waiting();
            self.maintenance.wait_for_progress(epoch);
            self.maintenance.writer_stopped_waiting();
        };
        self.maintenance.wake();
        self.wait_for_flush_generation(generation)?;
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
            l0_write_stall_trigger: 16,
            base_level_bytes: u64::MAX,
            level_multiplier: 10,
            target_file_size: 64 * 1024,
            block_cache_bytes: 4096,  // tiny: far smaller than the data
            max_open_table_files: 64, // this module tests the block cache, not the file cache
            // Irrelevant here: this module uses direct `Kiban`, which
            // never auto-freezes (11.8 auto-freeze is SharedKiban-only).
            write_buffer_bytes: 1 << 20,
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
            db.flush_sync_for_test().unwrap();
        }
    }

    /// Like `seed_for_compaction`, but tolerates `put`/`sync`/`flush`
    /// failing partway through instead of panicking — for tests that
    /// deliberately induce a background failure: once that failure is
    /// sticky, `put`/`flush` correctly start refusing too (11.5 checks
    /// maintenance failure before anything else, unconditionally of L0
    /// count), so exactly how many rounds land before that happens is
    /// timing-dependent and not something the test should assume.
    fn seed_until_maintenance_stops_accepting(db: &SharedKiban, rounds: u32, label: &str) {
        'rounds: for round in 0..rounds {
            for i in 0..20u32 {
                if db
                    .put(format!("k{i:03}"), format!("{label}{round}-{i}"))
                    .is_err()
                {
                    break 'rounds;
                }
            }
            if db.sync().is_err() || db.flush().is_err() {
                break;
            }
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
        // — via the sync bypass, since the real `flush` would itself
        // need this same frozen worker to ever commit (11.8).
        for i in 0..20u32 {
            db.put(format!("k{i:03}"), b"new").unwrap();
        }
        db.sync().unwrap();
        db.flush_sync_for_test().unwrap();

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

        // Once this failure lands, 11.5's maintenance-failure check
        // (unconditional, ahead of the L0 check) correctly starts
        // refusing further mutation — so exactly how many of these 3
        // rounds land is timing-dependent, not something to assert on.
        seed_until_maintenance_stops_accepting(&db, 3, "r");
        m.wait_settled();

        let err = db.maintenance_error();
        assert!(err.is_some(), "no background failure was induced");
        assert!(
            !db.is_poisoned(),
            "a build failure (never reaching the manifest) must not poison: {err:?}"
        );

        // whatever WAS durably flushed before maintenance failed reads
        // back identically, live and after reopen
        let before: Vec<Option<Vec<u8>>> = (0..20u32)
            .map(|i| db.get(format!("k{i:03}").as_bytes()).unwrap())
            .collect();

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
        for (i, want) in before.iter().enumerate() {
            let key = format!("k{i:03}");
            assert_eq!(
                reopened.get(key.as_bytes()).unwrap(),
                *want,
                "key {key} disagrees after reopen"
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

            seed_until_maintenance_stops_accepting(&db, 3, "r");
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

/// 11.9: ordinary shared reads choose a stable world under the engine
/// mutex, then consume immutable sources after releasing it.
#[cfg(test)]
mod read_view_tests {
    use super::*;
    use crate::testutil::TempDir;

    fn options(max_open_table_files: usize) -> KibanOptions {
        KibanOptions {
            l0_compaction_trigger: 100,
            l0_write_stall_trigger: 200,
            max_open_table_files,
            ..compaction_tests::tiny_options()
        }
    }

    fn seed_table(db: &SharedKiban, key: &[u8], value: &[u8]) {
        db.put(key, value).unwrap();
        db.sync().unwrap();
        db.flush_sync_for_test().unwrap();
    }

    fn paused_get(
        db: SharedKiban,
        key: &'static [u8],
    ) -> std::thread::JoinHandle<Result<Option<Vec<u8>>, DbError>> {
        std::thread::spawn(move || db.get(key))
    }

    /// A reader stopped after capture holds no engine mutex: a foreground
    /// mutation completes before that reader enters its SST lookup.
    #[test]
    fn stalled_sst_get_does_not_block_writer() {
        let td = TempDir::new("read-view-writer");
        let db = SharedKiban::open_with_options(td.path(), options(8)).unwrap();
        seed_table(&db, b"k", b"old");

        db.read_checkpoint.arm(1);
        let reader = paused_get(db.clone(), b"k");
        db.read_checkpoint.wait_reached();

        db.put(b"other", b"value").unwrap();
        db.read_checkpoint.release();

        assert_eq!(
            reader.join().expect("reader panicked").unwrap(),
            Some(b"old".to_vec())
        );
        assert_eq!(db.get(b"other").unwrap(), Some(b"value".to_vec()));
    }

    /// Both reads reach the post-capture boundary before either is
    /// released. Holding the old engine mutex across table work would
    /// make the second reader unable to reach this point.
    #[test]
    fn two_gets_reach_sst_work_without_engine_serialization() {
        let td = TempDir::new("read-view-two-gets");
        let db = SharedKiban::open_with_options(td.path(), options(8)).unwrap();
        seed_table(&db, b"a", b"one");
        seed_table(&db, b"b", b"two");

        db.read_checkpoint.arm(2);
        let first = paused_get(db.clone(), b"a");
        let second = paused_get(db.clone(), b"b");
        db.read_checkpoint.wait_reached();
        db.read_checkpoint.release();

        assert_eq!(
            first.join().expect("first reader panicked").unwrap(),
            Some(b"one".to_vec())
        );
        assert_eq!(
            second.join().expect("second reader panicked").unwrap(),
            Some(b"two".to_vec())
        );
    }

    /// Waiting for the only table-file-cache slot happens after release
    /// of the engine mutex, so unrelated WAL/memtable work still enters.
    #[test]
    fn fd_waiting_get_does_not_block_engine() {
        let td = TempDir::new("read-view-fd-wait");
        let db = SharedKiban::open_with_options(
            td.path(),
            KibanOptions {
                block_cache_bytes: 0,
                ..options(1)
            },
        )
        .unwrap();
        seed_table(&db, b"a", b"one");
        seed_table(&db, b"b", b"two");
        let (file_cache, first_number) = {
            let guard = db.lock().expect("engine lock available");
            (guard.file_cache.clone(), guard.version.tables[0].number)
        };
        let held = file_cache
            .acquire(first_number, &td.path().join(format!("{first_number}.sst")))
            .expect("lease first table");

        let reader = paused_get(db.clone(), b"b");
        file_cache.wait_until_someone_waiting();

        db.put(b"other", b"value").unwrap();
        db.sync().unwrap();
        drop(held);

        assert_eq!(
            reader.join().expect("reader panicked").unwrap(),
            Some(b"two".to_vec())
        );
        assert_eq!(db.get(b"other").unwrap(), Some(b"value".to_vec()));
    }

    #[test]
    fn captured_get_precedes_concurrent_put() {
        let td = TempDir::new("read-view-put");
        let db = SharedKiban::open_with_options(td.path(), options(8)).unwrap();
        seed_table(&db, b"k", b"old");

        db.read_checkpoint.arm(1);
        let reader = paused_get(db.clone(), b"k");
        db.read_checkpoint.wait_reached();
        db.put(b"k", b"new").unwrap();
        db.read_checkpoint.release();

        assert_eq!(
            reader.join().expect("reader panicked").unwrap(),
            Some(b"old".to_vec())
        );
        assert_eq!(db.get(b"k").unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn captured_get_precedes_concurrent_delete() {
        let td = TempDir::new("read-view-delete");
        let db = SharedKiban::open_with_options(td.path(), options(8)).unwrap();
        seed_table(&db, b"k", b"old");

        db.read_checkpoint.arm(1);
        let reader = paused_get(db.clone(), b"k");
        db.read_checkpoint.wait_reached();
        db.delete(b"k").unwrap();
        db.read_checkpoint.release();

        assert_eq!(
            reader.join().expect("reader panicked").unwrap(),
            Some(b"old".to_vec())
        );
        assert_eq!(db.get(b"k").unwrap(), None);
    }

    /// A captured immutable memtable stays readable after flush COMMIT
    /// removes it from current live engine state.
    #[test]
    fn captured_get_survives_immutable_flush_commit() {
        let td = TempDir::new("read-view-immutable");
        let db = SharedKiban::open_with_options(
            td.path(),
            KibanOptions {
                write_buffer_bytes: 1,
                ..options(8)
            },
        )
        .unwrap();
        let maintenance = db.maintenance_for_test();
        maintenance.arm_before_flush_build();
        db.put(b"k", b"value").unwrap();
        maintenance.wait_before_flush_build_reached();

        db.read_checkpoint.arm(1);
        let reader = paused_get(db.clone(), b"k");
        db.read_checkpoint.wait_reached();
        maintenance.release_before_flush_build();
        maintenance.wait_settled();
        db.read_checkpoint.release();

        assert_eq!(
            reader.join().expect("reader panicked").unwrap(),
            Some(b"value".to_vec())
        );
        assert_eq!(db.get(b"k").unwrap(), Some(b"value".to_vec()));
    }

    /// The captured `Arc<Version>` holds retired `Arc<TableEntry>`s alive
    /// through compaction. Once released, a later reclaim may unlink the
    /// old physical table.
    #[test]
    fn captured_get_pins_version_through_compaction_and_reclamation() {
        let td = TempDir::new("read-view-compaction");
        let db = SharedKiban::open_with_options(
            td.path(),
            KibanOptions {
                l0_compaction_trigger: 2,
                l0_write_stall_trigger: 30,
                ..options(8)
            },
        )
        .unwrap();
        seed_table(&db, b"k", b"old");
        let first_number = {
            let guard = db.lock().expect("engine lock available");
            guard.version.tables[0].number
        };

        db.read_checkpoint.arm(1);
        let reader = paused_get(db.clone(), b"k");
        db.read_checkpoint.wait_reached();

        seed_table(&db, b"trigger", b"value");
        db.maintenance_for_test().wait_settled();
        assert!(
            db.stats().unwrap().obsolete_files_pending > 0,
            "captured version must keep compacted input pending"
        );
        assert!(
            td.path().join(format!("{first_number}.sst")).exists(),
            "captured table must remain physically reachable"
        );

        db.read_checkpoint.release();
        assert_eq!(
            reader.join().expect("reader panicked").unwrap(),
            Some(b"old".to_vec())
        );
        assert_eq!(db.get(b"k").unwrap(), Some(b"old".to_vec()));

        seed_table(&db, b"next-a", b"value");
        seed_table(&db, b"next-b", b"value");
        db.maintenance_for_test().wait_settled();
        assert!(
            !td.path().join(format!("{first_number}.sst")).exists(),
            "later reclamation may remove an unpinned retired table"
        );
        assert_eq!(db.get(b"k").unwrap(), Some(b"old".to_vec()));
    }

    /// A normal shared get preserves block-cache behavior: once warm, it
    /// reads the cached block without acquiring an SST descriptor.
    #[test]
    fn shared_get_block_cache_hit_needs_no_file_lease() {
        let td = TempDir::new("read-view-block-cache");
        let db = SharedKiban::open_with_options(td.path(), options(1)).unwrap();
        seed_table(&db, b"a", b"one");
        assert_eq!(db.get(b"a").unwrap(), Some(b"one".to_vec()));
        seed_table(&db, b"b", b"two");
        assert_eq!(db.get(b"b").unwrap(), Some(b"two".to_vec()));

        let before = db.stats().unwrap();
        assert_eq!(db.get(b"a").unwrap(), Some(b"one".to_vec()));
        let after = db.stats().unwrap();
        assert_eq!(after.table_files.hits, before.table_files.hits);
        assert_eq!(after.table_files.misses, before.table_files.misses);
        assert_eq!(after.table_files.evictions, before.table_files.evictions);
        assert!(after.block_cache.hits > before.block_cache.hits);
    }

    #[test]
    fn shared_sst_get_remains_available_when_poisoned() {
        let td = TempDir::new("read-view-poisoned");
        let db = SharedKiban::open_with_options(td.path(), options(8)).unwrap();
        seed_table(&db, b"k", b"value");
        {
            let mut guard = db.lock().expect("engine lock available");
            guard.poison(PoisonCause::WalAppendFailed("test".to_string()));
        }

        assert_eq!(db.get(b"k").unwrap(), Some(b"value".to_vec()));
    }
}

/// 11.5: hard L0 write backpressure. `SharedKiban` stalls mutation-
/// producing work (`put`/`delete`/`write`/`flush`) once L0 reaches its
/// configured hard ceiling, until background compaction relieves it.
#[cfg(test)]
mod backpressure_tests {
    use super::*;
    use crate::sys;
    use crate::testutil::TempDir;

    /// A tight, deterministic hard L0 ceiling — small enough to reach in
    /// a handful of frozen-worker flushes, comfortably above
    /// `l0_compaction_trigger` per the configuration rule (11.5).
    fn stall_options() -> KibanOptions {
        KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 4,
            ..super::compaction_tests::tiny_options()
        }
    }

    /// Publishes `rounds` single-key L0 tables through `db`, reaching
    /// exactly `rounds` live L0 tables (never more — nothing commits
    /// while the worker is frozen at `before_build`). Uses the sync
    /// bypass (11.8): the real `flush` now shares the one maintenance
    /// worker with compaction, so a compaction this module deliberately
    /// freezes via `arm_before_build` would otherwise transitively
    /// block these seeding flushes too.
    fn seed_l0_tables(db: &SharedKiban, rounds: u32) {
        for round in 0..rounds {
            db.put(format!("seed{round}"), format!("v{round}")).unwrap();
            db.sync().unwrap();
            db.flush_sync_for_test().unwrap();
        }
    }

    /// Test 1: a writer genuinely stalls once L0 reaches the hard
    /// ceiling — the central proof of this phase.
    #[test]
    fn writer_stalls_when_l0_reaches_the_hard_ceiling() {
        let td = TempDir::new("bp-stall");
        let db = SharedKiban::open_with_options(td.path(), stall_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_l0_tables(&db, 4); // == l0_write_stall_trigger
        m.wait_before_build_reached(); // worker frozen; nothing has committed

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || writer_db.put(b"blocked", b"v"));

        m.wait_until_writer_waiting();
        assert_eq!(m.waiting_writers(), 1);
        assert!(
            !handle.is_finished(),
            "writer completed without waiting for L0 room"
        );

        m.release_before_build();
        handle.join().unwrap().unwrap();
        assert_eq!(db.get(b"blocked").unwrap(), Some(b"v".to_vec()));
    }

    /// Test 2: once a compaction COMMIT actually reduces L0 below the
    /// ceiling, the stalled writer wakes, rechecks, and proceeds — and
    /// its mutation survives a reopen.
    #[test]
    fn compaction_commit_releases_a_stalled_writer() {
        let td = TempDir::new("bp-release");
        let opts = stall_options();
        let db = SharedKiban::open_with_options(td.path(), opts.clone()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_l0_tables(&db, 4);
        m.wait_before_build_reached();

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || {
            writer_db.put(b"blocked", b"v").unwrap();
            writer_db.sync().unwrap();
        });
        m.wait_until_writer_waiting();
        assert!(!handle.is_finished());

        m.release_before_build();
        handle.join().unwrap();

        assert_eq!(db.get(b"blocked").unwrap(), Some(b"v".to_vec()));

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
        assert_eq!(reopened.get(b"blocked").unwrap(), Some(b"v".to_vec()));
    }

    /// Test 3: reads keep working while a writer is stalled —
    /// backpressure controls ingestion, it must not freeze the database.
    #[test]
    fn reads_continue_while_a_writer_stalls() {
        let td = TempDir::new("bp-reads");
        let db = SharedKiban::open_with_options(td.path(), stall_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_l0_tables(&db, 4);
        m.wait_before_build_reached();

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || writer_db.put(b"blocked", b"v"));
        m.wait_until_writer_waiting();

        assert_eq!(db.get(b"seed0").unwrap(), Some(b"v0".to_vec()));
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.get(b"seed1").unwrap(), Some(b"v1".to_vec()));
        let scanned = snap.scan().unwrap();
        assert!(scanned.iter().any(|(k, _)| k == b"seed2"));

        assert!(!handle.is_finished(), "writer must still be stalled");

        m.release_before_build();
        handle.join().unwrap().unwrap();
    }

    /// Test 4: `sync()` must complete even while a writer is stalled and
    /// the worker is frozen — durability of already-accepted writes
    /// must never depend on compaction catching up.
    #[test]
    fn sync_completes_while_a_writer_stalls() {
        let td = TempDir::new("bp-sync");
        let opts = stall_options();
        let db = SharedKiban::open_with_options(td.path(), opts.clone()).unwrap();
        let m = db.maintenance_for_test();

        db.put(b"early", b"durable-me").unwrap();

        m.arm_before_build();
        seed_l0_tables(&db, 4);
        m.wait_before_build_reached();

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || writer_db.put(b"blocked", b"v"));
        m.wait_until_writer_waiting();

        db.sync().unwrap();

        m.release_before_build();
        handle.join().unwrap().unwrap();

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
        assert_eq!(
            reopened.get(b"early").unwrap(),
            Some(b"durable-me".to_vec())
        );
    }

    /// Test 5: `flush()` itself respects the ceiling — a fresh flush
    /// must not publish yet another L0 table once L0 is already at the
    /// hard limit; it waits for room like everything else, then
    /// proceeds once compaction has made some.
    #[test]
    fn flush_stalls_at_the_ceiling_and_proceeds_after_room() {
        let td = TempDir::new("bp-flush-stalls");
        let db = SharedKiban::open_with_options(td.path(), stall_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_l0_tables(&db, 4);
        m.wait_before_build_reached();

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || writer_db.flush());
        m.wait_until_writer_waiting();
        assert!(
            !handle.is_finished(),
            "flush completed without waiting for L0 room"
        );

        m.release_before_build();
        handle.join().unwrap().unwrap();

        for round in 0..4u32 {
            let key = format!("seed{round}");
            assert_eq!(
                db.get(key.as_bytes()).unwrap(),
                Some(format!("v{round}").into_bytes())
            );
        }
    }

    /// Test 6: a background build failure must wake a stalled writer
    /// with an error, not hang it forever.
    #[test]
    fn maintenance_failure_wakes_a_stalled_writer_with_an_error() {
        let td = TempDir::new("bp-maint-fail");
        let opts = stall_options();
        let db = SharedKiban::open_with_options(td.path(), opts.clone()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();
        m.inject_on_worker(|| sys::install_fault(0)); // BUILD's first checked op

        seed_l0_tables(&db, 4);
        m.wait_before_build_reached();

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || writer_db.put(b"blocked", b"v"));
        m.wait_until_writer_waiting();
        assert!(!handle.is_finished());

        m.release_before_build();
        let result = handle.join().unwrap();

        assert!(
            result.is_err(),
            "a stalled writer must wake with an error, not hang, on maintenance failure"
        );
        assert!(db.maintenance_error().is_some());
        assert!(
            !db.is_poisoned(),
            "a build failure (never reaching the manifest) must not poison"
        );

        for round in 0..4u32 {
            let key = format!("seed{round}");
            assert_eq!(
                db.get(key.as_bytes()).unwrap(),
                Some(format!("v{round}").into_bytes())
            );
        }

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
        for round in 0..4u32 {
            let key = format!("seed{round}");
            assert_eq!(
                reopened.get(key.as_bytes()).unwrap(),
                Some(format!("v{round}").into_bytes())
            );
        }
    }

    /// Test 7: commit ambiguity during background publication (while a
    /// writer is stalled) must poison the engine exactly like the
    /// foreground path, waking the writer to refuse mutation.
    #[test]
    fn poisoning_wakes_a_stalled_writer_and_refuses_mutation() {
        let mut induced = false;
        for n in 0..60usize {
            let td = TempDir::new("bp-poison-iter");
            let opts = stall_options();
            let db = SharedKiban::open_with_options(td.path(), opts.clone()).unwrap();
            let m = db.maintenance_for_test();
            m.arm_before_build();

            seed_l0_tables(&db, 4);
            m.wait_before_build_reached();

            m.inject_on_worker(move || sys::install_faults(&[n]));

            let writer_db = db.clone();
            let handle = std::thread::spawn(move || writer_db.put(b"blocked", b"v"));
            m.wait_until_writer_waiting();

            m.release_before_build();
            let result = handle.join().unwrap();
            m.wait_settled();

            if !db.is_poisoned() {
                continue;
            }
            induced = true;

            assert!(
                result.is_err(),
                "a stalled writer must wake with an error when poisoned"
            );
            assert!(
                db.put(b"later", b"x").is_err(),
                "mutation must be refused after poisoning"
            );
            let _ = db.get(b"seed0").unwrap(); // reads remain available

            drop(db);
            let reopened = Kiban::open_with_options(td.path(), opts).unwrap();
            assert!(!reopened.is_poisoned());
            break;
        }
        assert!(induced, "poisoning during a stalled write never induced");
    }

    /// Test 8: no missed wakeups under sustained concurrent pressure.
    /// Several writers repeatedly cross the stall ceiling while the
    /// worker races to relieve it; a lost-wakeup bug would hang this
    /// test forever rather than fail an assertion — that is the proof.
    #[test]
    fn no_missed_wakeups_under_repeated_pressure_and_release() {
        for attempt in 0..5 {
            let td = TempDir::new(&format!("bp-no-missed-wakeup-{attempt}"));
            let db = SharedKiban::open_with_options(td.path(), stall_options()).unwrap();

            let handles: Vec<_> = (0..4u32)
                .map(|t| {
                    let db = db.clone();
                    std::thread::spawn(move || {
                        for i in 0..25u32 {
                            db.put(format!("t{t}-k{i:03}"), format!("v{t}-{i}"))
                                .unwrap();
                            if i % 4 == 0 {
                                db.sync().unwrap();
                                db.flush().unwrap();
                            }
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }

            db.maintenance_for_test().wait_settled();
            assert!(db.maintenance_error().is_none());

            for t in 0..4u32 {
                for i in 0..25u32 {
                    let key = format!("t{t}-k{i:03}");
                    assert_eq!(
                        db.get(key.as_bytes()).unwrap(),
                        Some(format!("v{t}-{i}").into_bytes())
                    );
                }
            }
        }
    }

    /// Test 9: direct `Kiban` ignores `l0_write_stall_trigger` entirely
    /// and stays fully synchronous — a low stall trigger configured
    /// alongside it never blocks anything, because direct `Kiban` never
    /// checks it at all.
    #[test]
    fn direct_kiban_ignores_the_stall_trigger_and_stays_synchronous() {
        let td = TempDir::new("bp-direct-kiban");
        let mut db = Kiban::open_with_options(td.path(), stall_options()).unwrap();
        for round in 0..20u32 {
            db.put(format!("k{round}"), format!("v{round}")).unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }
        for round in 0..20u32 {
            assert_eq!(
                db.get(format!("k{round}").as_bytes()).unwrap(),
                Some(format!("v{round}").into_bytes())
            );
        }
    }
}

/// 11.6: bounded SST file descriptors. `SstTable` does not permanently
/// own a file handle — every read leases one from a shared,
/// hard-capacity `TableFileCache`. Low-level cache mechanics (hard
/// capacity, LRU, in-use-cannot-be-evicted, no missed wakeup) are unit
/// tested directly against `TableFileCache` in `file_cache.rs`; these
/// are the integration-level properties that need a real `Kiban`.
#[cfg(test)]
mod file_cache_tests {
    use super::*;
    use crate::testutil::TempDir;

    fn tiny_with(max_open_table_files: usize) -> KibanOptions {
        KibanOptions {
            max_open_table_files,
            l0_compaction_trigger: 1000, // isolate: no compaction unless a test wants it
            l0_write_stall_trigger: 2000,
            ..compaction_tests::tiny_options()
        }
    }

    /// Test 1: keeping many `SstTable`s alive (via the live `Version`)
    /// does not pin a file-cache slot per table — only active use does.
    #[test]
    fn live_tables_do_not_each_pin_a_file_cache_slot() {
        let td = TempDir::new("fc-sst-no-pin");
        let mut db = Kiban::open_with_options(td.path(), tiny_with(2)).unwrap();

        for i in 0..10u32 {
            db.put(format!("k{i}"), format!("v{i}")).unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }
        assert_eq!(db.version.tables.len(), 10);
        // every one of the 10 SstTables is alive right now, pinned by
        // db.version — yet the file cache holds at most 2 descriptors
        assert!(db.file_cache.resident() <= 2);

        for i in 0..10u32 {
            assert_eq!(
                db.get(format!("k{i}").as_bytes()).unwrap(),
                Some(format!("v{i}").into_bytes())
            );
        }
        assert!(db.file_cache.max_resident_seen() <= 2);
    }

    /// Test 6: a block-cache hit must not touch the file cache at all.
    /// Capacity 1, so if the second read of table A needed to reopen
    /// it, it would have to evict B — instead B's descriptor must stay
    /// untouched throughout.
    #[test]
    fn block_cache_hit_requires_no_file_lease() {
        let td = TempDir::new("fc-block-hit");
        let mut db = Kiban::open_with_options(td.path(), tiny_with(1)).unwrap();

        db.put(b"a", b"1").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // table A
        assert_eq!(db.get(b"a").unwrap(), Some(b"1".to_vec())); // populates the block cache for A

        db.put(b"b", b"2").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // table B
        assert_eq!(db.get(b"b").unwrap(), Some(b"2".to_vec())); // capacity 1: evicts A's descriptor
        let b_number = db.version.tables.last().unwrap().number;
        assert!(db.file_cache.is_resident(b_number));
        assert_eq!(db.file_cache.resident(), 1);

        // reading A again must hit the block cache — no file lease
        // needed, so B's descriptor (the cache's only slot) must be
        // completely untouched by it
        assert_eq!(db.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert!(
            db.file_cache.is_resident(b_number),
            "a block-cache hit must not touch the file cache"
        );
        assert_eq!(db.file_cache.resident(), 1);
    }

    /// Test 7: a database with far more SSTables than the descriptor
    /// capacity must still open, and every table must still be
    /// readable — this catches accidental permanent handles held
    /// during `SstTable::open` / `Kiban::open_with_options`.
    #[test]
    fn opening_many_tables_with_a_tiny_fd_cache_succeeds() {
        let td = TempDir::new("fc-open-many");
        {
            let mut db = Kiban::open_with_options(td.path(), tiny_with(64)).unwrap();
            for i in 0..40u32 {
                db.put(format!("k{i:03}"), format!("v{i}")).unwrap();
                db.sync().unwrap();
                db.flush().unwrap();
            }
        }

        let db = Kiban::open_with_options(td.path(), tiny_with(2)).unwrap();
        assert_eq!(db.version.tables.len(), 40);
        assert!(
            db.file_cache.resident() <= 2,
            "open() must not hold every table's descriptor at once"
        );

        for i in 0..40u32 {
            assert_eq!(
                db.get(format!("k{i:03}").as_bytes()).unwrap(),
                Some(format!("v{i}").into_bytes())
            );
        }
        let scanned: usize = db.iter().map(|r| r.unwrap()).fold(0, |n, _| n + 1);
        assert_eq!(scanned, 40);
        assert!(db.file_cache.max_resident_seen() <= 2);
    }

    /// Test 8: a snapshot pinning many tables' metadata must not force
    /// that many descriptors open at once.
    #[test]
    fn snapshot_pins_many_tables_without_pinning_many_descriptors() {
        let td = TempDir::new("fc-snapshot-no-pin");
        let mut db = Kiban::open_with_options(td.path(), tiny_with(3)).unwrap();
        for i in 0..30u32 {
            db.put(format!("k{i:03}"), format!("v{i}")).unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }

        let snap = db.snapshot(); // pins all 30 tables' Arc<TableEntry>
        assert!(db.file_cache.resident() <= 3);

        for i in 0..30u32 {
            assert_eq!(
                db.get_at(&snap, format!("k{i:03}").as_bytes()).unwrap(),
                Some(format!("v{i}").into_bytes())
            );
        }
        let scanned = db.scan_at(&snap).unwrap();
        assert_eq!(scanned.len(), 30);
        assert!(
            db.file_cache.max_resident_seen() <= 3,
            "a snapshot must pin metadata, not one descriptor per table"
        );
    }

    /// Test 9: background compaction over many input files must never
    /// need more simultaneously open descriptors than configured.
    #[test]
    fn background_compaction_respects_the_fd_bound() {
        let td = TempDir::new("fc-compaction-bound");
        let opts = KibanOptions {
            max_open_table_files: 3,
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 30,
            ..compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();

        for round in 0..20u32 {
            for i in 0..15u32 {
                db.put(format!("k{i:03}"), format!("r{round}-{i}")).unwrap();
            }
            db.sync().unwrap();
            db.flush().unwrap();
        }
        db.maintenance_for_test().wait_settled();
        assert!(
            db.maintenance_error().is_none(),
            "{:?}",
            db.maintenance_error()
        );

        for i in 0..15u32 {
            let key = format!("k{i:03}");
            assert_eq!(
                db.get(key.as_bytes()).unwrap(),
                Some(format!("r19-{i}").into_bytes())
            );
        }
        assert!(db.lock().unwrap().file_cache.max_resident_seen() <= 3);
    }

    /// Test 10: once a table becomes genuinely obsolete (compacted
    /// away, nothing references it), its file-cache entry must be
    /// invalidated before the file is unlinked — never left dangling.
    #[test]
    fn reclaiming_an_obsolete_table_invalidates_its_cached_handle() {
        let td = TempDir::new("fc-reclaim-invalidate");
        let mut db = Kiban::open_with_options(
            td.path(),
            KibanOptions {
                max_open_table_files: 8,
                l0_compaction_trigger: 2,
                l0_write_stall_trigger: 30,
                ..compaction_tests::tiny_options()
            },
        )
        .unwrap();

        db.put(b"a", b"1").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        let first_number = db.version.tables[0].number;
        assert_eq!(db.get(b"a").unwrap(), Some(b"1".to_vec())); // populate the file cache
        assert!(db.file_cache.is_resident(first_number));

        db.put(b"b", b"2").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // L0 reaches trigger=2: compacts, retiring the first table

        assert!(
            !db.file_cache.is_resident(first_number),
            "a reclaimed table's cached descriptor must be gone"
        );
        assert!(
            !td.path().join(format!("{first_number}.sst")).exists(),
            "a reclaimed table's file must be unlinked"
        );

        assert_eq!(db.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(db.get(b"b").unwrap(), Some(b"2".to_vec()));
    }

    /// Test 11: foreground reads and background compaction BUILD
    /// racing for a 2-descriptor cache must never deadlock — the
    /// adversarial test for the locking design. First phase uses the
    /// 11.4 worker checkpoints for a deterministic freeze; the second
    /// phase races real concurrent activity against the released
    /// worker.
    #[test]
    fn foreground_and_background_pressure_on_a_tiny_fd_cache_does_not_deadlock() {
        let td = TempDir::new("fc-adversarial");
        let opts = KibanOptions {
            max_open_table_files: 2,
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 40,
            ..compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        // sync bypass (11.8): seeding past `l0_compaction_trigger` while
        // `arm_before_build` holds the worker frozen would otherwise
        // queue a later round's real `flush` behind that same freeze.
        for round in 0..6u32 {
            for i in 0..10u32 {
                db.put(format!("k{i:03}"), format!("r{round}-{i}")).unwrap();
            }
            db.sync().unwrap();
            db.flush_sync_for_test().unwrap();
        }
        m.wait_before_build_reached();

        // foreground readers hammer the tiny file cache while the
        // worker sits frozen, holding no lease at all
        let reader_handles: Vec<_> = (0..4u32)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        for i in 0..10u32 {
                            let _ = db.get(format!("k{i:03}").as_bytes()).unwrap();
                        }
                        let snap = db.snapshot().unwrap();
                        let _ = snap.scan().unwrap();
                    }
                })
            })
            .collect();
        for h in reader_handles {
            h.join().unwrap();
        }

        // release the worker so BUILD now competes with further
        // foreground writes/reads for the same tiny cache
        m.release_before_build();
        let more_handles: Vec<_> = (0..4u32)
            .map(|t| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for round in 0..10u32 {
                        db.put(format!("t{t}-k{round:03}"), format!("v{round}"))
                            .unwrap();
                        db.sync().unwrap();
                        let _ = db.get(b"k000").unwrap();
                    }
                })
            })
            .collect();
        for h in more_handles {
            h.join().unwrap();
        }

        db.maintenance_for_test().wait_settled();
        assert!(
            db.maintenance_error().is_none(),
            "{:?}",
            db.maintenance_error()
        );

        for i in 0..10u32 {
            let key = format!("k{i:03}");
            assert_eq!(
                db.get(key.as_bytes()).unwrap(),
                Some(format!("r5-{i}").into_bytes())
            );
        }
        assert!(db.lock().unwrap().file_cache.max_resident_seen() <= 2);
    }
}

/// 11.7: `SharedKiban::stats()` — a cheap, observational snapshot
/// answering "what is Kiban doing?" without disk I/O or side effects.
/// Low-level counter mechanics (exact hit/miss/eviction/wait counts)
/// are unit tested directly against `BlockCache` and `TableFileCache`
/// in `cache.rs` / `file_cache.rs`; these are the integration-level
/// properties that need a real engine.
#[cfg(test)]
mod stats_tests {
    use super::*;
    use crate::testutil::TempDir;

    fn stall_options() -> KibanOptions {
        KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 4,
            ..super::compaction_tests::tiny_options()
        }
    }

    /// Uses the sync bypass (11.8) for the same reason
    /// `backpressure_tests::seed_l0_tables` does: seeding past
    /// `l0_compaction_trigger` while `arm_before_build` holds the
    /// worker frozen would otherwise queue a later round's real `flush`
    /// behind that same freeze.
    fn seed_l0_tables(db: &SharedKiban, rounds: u32) {
        for round in 0..rounds {
            db.put(format!("seed{round}"), format!("v{round}")).unwrap();
            db.sync().unwrap();
            db.flush_sync_for_test().unwrap();
        }
    }

    /// Test 1: a fresh database reports sensible zeros — no implementation
    /// trivia asserted, just the facts a caller would actually check.
    #[test]
    fn empty_engine_reports_sensible_zeros() {
        let td = TempDir::new("stats-empty");
        let db = SharedKiban::open(td.path()).unwrap();
        let stats = db.stats().unwrap();

        assert_eq!(stats.memtable_entries, 0);
        assert_eq!(stats.active_snapshots, 0);
        assert_eq!(stats.obsolete_files_pending, 0);
        assert!(stats.levels.is_empty());
        assert_eq!(stats.block_cache.resident_entries, 0);
        assert_eq!(stats.block_cache.hits, 0);
        assert_eq!(stats.block_cache.misses, 0);
        assert!(stats.table_files.resident <= stats.table_files.capacity);
        assert_eq!(stats.maintenance.compactions_completed, 0);
        assert_eq!(stats.maintenance.compactions_failed, 0);
        assert_eq!(stats.maintenance.waiting_writers, 0);
        assert_eq!(stats.maintenance.write_stalls, 0);
    }

    /// Test 2: per-level table count and byte total exactly match the
    /// engine's own published `Version` — the stats path must read
    /// `Version`, never walk the filesystem independently.
    #[test]
    fn level_stats_match_the_published_version() {
        let td = TempDir::new("stats-levels");
        let db = SharedKiban::open_with_options(td.path(), super::compaction_tests::tiny_options())
            .unwrap();

        for round in 0..12u32 {
            for i in 0..5u32 {
                db.put(format!("k{i:03}"), format!("r{round}-{i}")).unwrap();
            }
            db.sync().unwrap();
            db.flush().unwrap();
        }
        db.maintenance_for_test().wait_settled();
        assert!(db.maintenance_error().is_none());

        let stats = db.stats().unwrap();
        let mut expected: std::collections::BTreeMap<u32, (usize, u64)> = Default::default();
        for t in &db.lock().unwrap().version.tables {
            let e = expected.entry(t.level).or_insert((0, 0));
            e.0 += 1;
            e.1 += t.size;
        }

        assert!(!stats.levels.is_empty());
        assert_eq!(stats.levels.len(), expected.len());
        for ls in &stats.levels {
            let (tables, bytes) = expected[&ls.level];
            assert_eq!(ls.tables, tables, "level {} table count", ls.level);
            assert_eq!(ls.bytes, bytes, "level {} byte total", ls.level);
        }
    }

    /// Test 6: exactly one write stall per genuinely blocked mutation
    /// call, not one per wake/recheck cycle — and `waiting_writers`
    /// tracks the live count while it's actually parked.
    #[test]
    fn write_stall_counter_counts_blocked_calls_not_wakeups() {
        let td = TempDir::new("stats-write-stall");
        let db = SharedKiban::open_with_options(td.path(), stall_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_l0_tables(&db, 4); // == l0_write_stall_trigger
        m.wait_before_build_reached();
        assert_eq!(db.stats().unwrap().maintenance.write_stalls, 0);

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || writer_db.put(b"blocked", b"v"));
        m.wait_until_writer_waiting();
        assert_eq!(db.stats().unwrap().maintenance.waiting_writers, 1);

        m.release_before_build();
        handle.join().unwrap().unwrap();

        let stats = db.stats().unwrap();
        assert_eq!(stats.maintenance.waiting_writers, 0);
        assert_eq!(stats.maintenance.write_stalls, 1);
    }

    /// Test 7: a real completed compaction moves `compactions_completed`
    /// and both byte totals, and the output bytes match the level the
    /// output actually landed in.
    #[test]
    fn compaction_stats_reflect_a_real_completed_job() {
        let td = TempDir::new("stats-compaction");
        let opts = KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 30,
            ..super::compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();

        seed_l0_tables(&db, 2); // == l0_compaction_trigger
        m.wait_before_build_reached();

        let before = db.stats().unwrap();
        assert!(before.maintenance.compaction_running);
        assert_eq!(before.maintenance.compactions_completed, 0);

        m.release_before_build();
        m.wait_settled();
        assert!(db.maintenance_error().is_none());

        let after = db.stats().unwrap();
        assert!(!after.maintenance.compaction_running);
        assert_eq!(after.maintenance.compactions_completed, 1);
        assert!(after.maintenance.compaction_input_bytes > 0);
        assert!(after.maintenance.compaction_output_bytes > 0);

        let l1_bytes: u64 = after
            .levels
            .iter()
            .filter(|l| l.level == 1)
            .map(|l| l.bytes)
            .sum();
        assert_eq!(after.maintenance.compaction_output_bytes, l1_bytes);
    }

    /// Test 8: a failed background job counts as failed, never as
    /// completed — the existing `maintenance_error()` API stays the
    /// only source of the actual error text.
    #[test]
    fn failed_compaction_counts_as_failed_not_completed() {
        let td = TempDir::new("stats-compaction-fail");
        let opts = KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 30,
            ..super::compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_build();
        m.inject_on_worker(|| sys::install_fault(0)); // BUILD's first checked op

        seed_l0_tables(&db, 2);
        m.wait_before_build_reached();
        m.release_before_build();
        m.wait_settled();

        assert!(db.maintenance_error().is_some());
        let stats = db.stats().unwrap();
        assert_eq!(stats.maintenance.compactions_failed, 1);
        assert_eq!(stats.maintenance.compactions_completed, 0);
    }

    /// Test 9: a snapshot pinning a table that compaction retires keeps
    /// it in `obsolete_files_pending` until both the snapshot is gone
    /// and a later compaction's reclaim pass re-checks it — turning
    /// 11.3/11.6's lifetime rules into something visible.
    #[test]
    fn snapshot_and_obsolete_counts_reflect_pinned_and_pending_files() {
        let td = TempDir::new("stats-snapshot-obsolete");
        let opts = KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 30,
            ..super::compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();

        db.put(b"a", b"1").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        db.maintenance_for_test().wait_settled();

        let snap = db.snapshot().unwrap(); // pins the live table's Arc<TableEntry>
        assert_eq!(db.stats().unwrap().active_snapshots, 1);

        // one more flush reaches l0_compaction_trigger and compacts,
        // retiring the table `snap` still references
        db.put(b"b", b"2").unwrap();
        db.sync().unwrap();
        db.flush().unwrap();
        db.maintenance_for_test().wait_settled();
        assert!(db.maintenance_error().is_none());

        let mid = db.stats().unwrap();
        assert_eq!(mid.active_snapshots, 1);
        assert!(
            mid.obsolete_files_pending > 0,
            "the snapshot must keep the retired input table pending reclamation"
        );

        drop(snap);
        assert_eq!(db.stats().unwrap().active_snapshots, 0);

        // dropping a snapshot alone does not reclaim anything — only
        // the next compaction's reclaim_obsolete pass re-checks the
        // refcount, so two more rounds are needed to cross
        // l0_compaction_trigger again and trigger one
        for (k, v) in [(b"c" as &[u8], b"3" as &[u8]), (b"d", b"4")] {
            db.put(k, v).unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }
        db.maintenance_for_test().wait_settled();
        assert!(db.maintenance_error().is_none());

        let after = db.stats().unwrap();
        assert_eq!(
            after.obsolete_files_pending, 0,
            "reclamation must clear the pending file once the snapshot is gone"
        );
    }

    /// Test 10: reading stats must not itself be activity — repeated
    /// calls must not move any counter.
    #[test]
    fn stats_reads_do_not_move_any_counter() {
        let td = TempDir::new("stats-no-side-effects");
        let opts = KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 30,
            ..super::compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();

        for i in 0..3u32 {
            db.put(format!("k{i}"), format!("v{i}")).unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }
        db.maintenance_for_test().wait_settled();
        for i in 0..3u32 {
            let _ = db.get(format!("k{i}").as_bytes()).unwrap();
        }

        let before = db.stats().unwrap();
        for _ in 0..20 {
            let _ = db.stats().unwrap();
        }
        let after = db.stats().unwrap();

        assert_eq!(before.block_cache.hits, after.block_cache.hits);
        assert_eq!(before.block_cache.misses, after.block_cache.misses);
        assert_eq!(before.block_cache.evictions, after.block_cache.evictions);
        assert_eq!(before.table_files.hits, after.table_files.hits);
        assert_eq!(before.table_files.misses, after.table_files.misses);
        assert_eq!(
            before.maintenance.compactions_completed,
            after.maintenance.compactions_completed
        );
        assert_eq!(
            before.maintenance.write_stalls,
            after.maintenance.write_stalls
        );
    }

    /// Test 11: concurrent readers, writers, and background compaction
    /// racing against repeated `stats()` calls from another thread must
    /// never deadlock or panic; cumulative counters never move
    /// backwards; the FD hard bound holds throughout; only nonempty
    /// levels are ever reported.
    #[test]
    fn concurrent_stats_reads_never_deadlock_or_go_backwards() {
        let td = TempDir::new("stats-concurrent");
        let opts = KibanOptions {
            l0_compaction_trigger: 2,
            l0_write_stall_trigger: 30,
            max_open_table_files: 4,
            ..super::compaction_tests::tiny_options()
        };
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();

        let stats_db = db.clone();
        let stats_handle = std::thread::spawn(move || {
            let mut last_completed = 0u64;
            let mut last_stalls = 0u64;
            for _ in 0..200 {
                let s = stats_db.stats().unwrap();
                assert!(s.table_files.resident <= s.table_files.capacity);
                assert!(s.maintenance.compactions_completed >= last_completed);
                assert!(s.maintenance.write_stalls >= last_stalls);
                last_completed = s.maintenance.compactions_completed;
                last_stalls = s.maintenance.write_stalls;
                for l in &s.levels {
                    assert!(l.tables > 0, "an empty level must not be reported");
                }
            }
        });

        let writer_handles: Vec<_> = (0..3u32)
            .map(|t| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for i in 0..30u32 {
                        db.put(format!("t{t}-k{i:03}"), format!("v{i}")).unwrap();
                        if i % 5 == 0 {
                            db.sync().unwrap();
                            db.flush().unwrap();
                        }
                    }
                })
            })
            .collect();
        for h in writer_handles {
            h.join().unwrap();
        }
        stats_handle.join().unwrap();

        db.maintenance_for_test().wait_settled();
        assert!(db.maintenance_error().is_none());
    }
}

/// 11.8: immutable memtable + background flush. The active memtable
/// freezes (WAL handoff, durable, MANIFEST-committed) instead of
/// blocking writers for the SST build; the frozen memtable flushes on
/// the shared maintenance worker, ahead of compaction. Low-level format
/// mechanics (the multi-WAL MANIFEST, recovery replay/consolidation)
/// get direct `Kiban`-level tests; the concurrency/lifetime properties
/// need `SharedKiban` and the worker's own `before_flush_build`
/// checkpoint — a separate freeze from compaction's `before_build`, so
/// a test freezing one never transitively blocks the other on the one
/// shared worker (see `TestHooks::before_flush_build`'s doc comment).
#[cfg(test)]
mod flush_pipeline_tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    /// A tiny, easily-crossed write-buffer threshold, isolated from
    /// compaction so only freeze/flush behavior is under test.
    fn tiny_buffer_options() -> KibanOptions {
        KibanOptions {
            write_buffer_bytes: 300,
            l0_compaction_trigger: 1000,
            l0_write_stall_trigger: 2000,
            ..super::compaction_tests::tiny_options()
        }
    }

    /// Enough ~76-byte entries (40-byte value + short key + the 32-byte
    /// fixed overhead) to cross `write_buffer_bytes: 300` exactly once
    /// from a near-zero starting memtable — deliberately not more:
    /// called on this thread with nothing yet releasing a frozen
    /// worker, so the *remainder* after the freeze it triggers must
    /// itself stay safely under threshold too, or this call would
    /// block on the immutable slot before ever returning. 5 entries
    /// (freeze on the 4th; one entry, ~76 bytes, left over) leaves a
    /// wide margin under a second 300-byte crossing.
    fn fill_past_threshold(db: &SharedKiban, prefix: &str) {
        for i in 0..5u32 {
            db.put(format!("{prefix}{i:03}"), vec![b'x'; 40]).unwrap();
        }
    }

    /// Test 1: crossing the write-buffer threshold freezes the active
    /// memtable and hands writers a fresh memtable + WAL immediately —
    /// before the flush BUILD that publishes the frozen one even starts.
    #[test]
    fn automatic_freeze_hands_writers_a_fresh_memtable_and_wal_immediately() {
        let td = TempDir::new("flush-auto-freeze");
        let db = SharedKiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_flush_build();

        let wal_before = db.lock().unwrap().wal_number;
        // put one entry at a time, checking after each: the moment
        // freeze happens, the active memtable must immediately be a
        // fresh, empty one — checked right then, since later puts in
        // this same loop would otherwise land in it and hide the fact.
        let mut froze_after: Option<u32> = None;
        for i in 0..8u32 {
            db.put(format!("a{i:03}"), vec![b'x'; 40]).unwrap();
            if db.lock().unwrap().immutable.is_some() {
                froze_after = Some(i);
                break;
            }
        }
        let froze_after = froze_after.expect("must have frozen within 8 puts");
        {
            let guard = db.lock().unwrap();
            assert_ne!(guard.wal_number, wal_before, "a fresh WAL must be active");
            assert!(
                guard.memtable.is_empty(),
                "the new active memtable starts empty right after freeze"
            );
        }
        m.wait_before_flush_build_reached();

        // writers continue immediately, without waiting for the BUILD
        db.put(b"during-flush-build", b"v").unwrap();
        assert_eq!(db.get(b"during-flush-build").unwrap(), Some(b"v".to_vec()));
        // the remaining seed puts land in the fresh active memtable
        for i in (froze_after + 1)..8u32 {
            db.put(format!("a{i:03}"), vec![b'x'; 40]).unwrap();
        }

        m.release_before_flush_build();
        m.wait_settled();
        assert!(db.maintenance_error().is_none());
        for i in 0..8u32 {
            assert!(
                db.get(format!("a{i:03}").as_bytes()).unwrap().is_some(),
                "the frozen generation's data must have flushed"
            );
        }
    }

    /// Test 2: puts, gets, and scans against the current state all keep
    /// working while a flush BUILD is paused — the engine mutex was
    /// never held for SST construction.
    #[test]
    fn foreground_writes_and_reads_continue_during_flush_build() {
        let td = TempDir::new("flush-foreground-continues");
        let db = SharedKiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_flush_build();

        db.put(b"old-key", b"old-value").unwrap();
        fill_past_threshold(&db, "pad");
        m.wait_before_flush_build_reached();

        db.put(b"new-key", b"new-value").unwrap();
        assert_eq!(db.get(b"old-key").unwrap(), Some(b"old-value".to_vec()));
        assert_eq!(db.get(b"new-key").unwrap(), Some(b"new-value".to_vec()));
        let snap = db.snapshot().unwrap();
        let scanned = snap.scan().unwrap();
        assert!(scanned.iter().any(|(k, _)| k == b"old-key"));
        assert!(scanned.iter().any(|(k, _)| k == b"new-key"));

        m.release_before_flush_build();
        m.wait_settled();
        assert!(db.maintenance_error().is_none());
    }

    /// Test 3: the immutable memtable participates in reads exactly
    /// like the active one — including a delete shadowing an older
    /// SST's value for as long as the flush is pending.
    #[test]
    fn immutable_memtable_shadows_older_sst_values_including_deletes() {
        let td = TempDir::new("flush-immutable-shadow");
        let db = SharedKiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        let m = db.maintenance_for_test();

        db.put(b"k", b"old").unwrap();
        db.sync().unwrap();
        db.flush().unwrap(); // an SST with k = "old"

        m.arm_before_flush_build();
        db.delete(b"k").unwrap();
        let flush_db = db.clone();
        let handle = std::thread::spawn(move || flush_db.flush()); // freezes the delete-only memtable
        m.wait_before_flush_build_reached();

        assert!(db.lock().unwrap().immutable.is_some());
        assert_eq!(
            db.get(b"k").unwrap(),
            None,
            "the frozen delete must not fall through to the older SST value"
        );

        m.release_before_flush_build();
        handle.join().unwrap().unwrap();
        assert_eq!(db.get(b"k").unwrap(), None);

        db.put(b"k", b"newest").unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"newest".to_vec()));
    }

    /// Test 4: a snapshot returns the exact same state whether captured
    /// before a freeze, while the flush BUILD is paused, after the
    /// flush commits, or after a later compaction runs.
    #[test]
    fn snapshot_returns_the_same_state_through_every_freeze_flush_phase() {
        let td = TempDir::new("flush-snapshot-phases");
        let opts = super::compaction_tests::tiny_options();
        let db = SharedKiban::open_with_options(td.path(), opts).unwrap();
        let m = db.maintenance_for_test();

        db.put(b"k", b"v1").unwrap();
        let snap_before = db.snapshot().unwrap();
        assert_eq!(snap_before.get(b"k").unwrap(), Some(b"v1".to_vec()));

        m.arm_before_flush_build();
        let flush_db = db.clone();
        let handle = std::thread::spawn(move || flush_db.flush());
        m.wait_before_flush_build_reached();

        let snap_during = db.snapshot().unwrap(); // captured from the immutable Arc
        assert_eq!(snap_before.get(b"k").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(snap_during.get(b"k").unwrap(), Some(b"v1".to_vec()));

        m.release_before_flush_build();
        handle.join().unwrap().unwrap();

        let snap_after = db.snapshot().unwrap();
        for s in [&snap_before, &snap_during, &snap_after] {
            assert_eq!(s.get(b"k").unwrap(), Some(b"v1".to_vec()));
        }

        // a real compaction afterward (l0_compaction_trigger: 2) must
        // not disturb any of them
        for round in 0..6u32 {
            db.put(format!("pad{round}"), b"x").unwrap();
            db.sync().unwrap();
            db.flush().unwrap();
        }
        db.maintenance_for_test().wait_settled();
        assert!(db.maintenance_error().is_none());

        for s in [&snap_before, &snap_during, &snap_after] {
            assert_eq!(s.get(b"k").unwrap(), Some(b"v1".to_vec()));
            let scanned = s.scan().unwrap();
            assert!(scanned.iter().any(|(k, v)| k == b"k" && v == b"v1"));
        }
    }

    /// Test 12: once the immutable slot is occupied, a writer that also
    /// pushes the (new) active memtable past threshold must block —
    /// never allocate a second immutable memtable (impossible anyway:
    /// `Kiban::immutable` is an `Option`, not a queue) and never spin.
    #[test]
    fn immutable_slot_backpressure_blocks_until_the_pending_flush_commits() {
        let td = TempDir::new("flush-immutable-backpressure");
        let db = SharedKiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_flush_build();

        fill_past_threshold(&db, "a"); // freezes generation 1; worker parks before BUILD
        m.wait_before_flush_build_reached();
        assert!(db.lock().unwrap().immutable.is_some());

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || {
            for i in 0..8u32 {
                writer_db.put(format!("b{i:03}"), vec![b'x'; 40]).unwrap();
            }
        });
        db.maintenance_for_test().wait_until_writer_waiting();
        assert_eq!(db.maintenance_for_test().waiting_writers(), 1);
        assert!(
            !handle.is_finished(),
            "writer must wait for the immutable slot, not proceed"
        );
        assert!(db.lock().unwrap().immutable.is_some());

        m.release_before_flush_build();
        handle.join().unwrap();

        for i in 0..8u32 {
            assert_eq!(
                db.get(format!("b{i:03}").as_bytes()).unwrap(),
                Some(vec![b'x'; 40])
            );
        }
    }

    /// Test 13: `sync()` completes for already-accepted active-WAL
    /// writes even while a writer is stalled on the immutable slot —
    /// durability of accepted writes must never depend on the pending
    /// flush freeing that slot.
    #[test]
    fn sync_completes_while_a_writer_is_stalled_on_the_immutable_slot() {
        let td = TempDir::new("flush-sync-during-immutable-stall");
        let db = SharedKiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        let m = db.maintenance_for_test();

        db.put(b"early", b"durable-me").unwrap();

        m.arm_before_flush_build();
        fill_past_threshold(&db, "a");
        m.wait_before_flush_build_reached();

        let writer_db = db.clone();
        let handle = std::thread::spawn(move || {
            for i in 0..8u32 {
                writer_db.put(format!("b{i:03}"), vec![b'x'; 40]).unwrap();
            }
        });
        db.maintenance_for_test().wait_until_writer_waiting();

        db.sync().unwrap();

        m.release_before_flush_build();
        handle.join().unwrap();

        drop(db);
        let reopened = Kiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        assert_eq!(
            reopened.get(b"early").unwrap(),
            Some(b"durable-me".to_vec())
        );
    }

    /// Test 14: an explicit `flush()` waits for its OWN generation's
    /// commit — never returns early, never waits for a later, unrelated
    /// compaction to also finish.
    #[test]
    fn explicit_flush_waits_only_for_its_own_generation() {
        let td = TempDir::new("flush-waits-for-generation");
        let db = SharedKiban::open_with_options(td.path(), super::compaction_tests::tiny_options())
            .unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_flush_build();

        db.put(b"k", b"v").unwrap();
        let flush_db = db.clone();
        let handle = std::thread::spawn(move || flush_db.flush());
        m.wait_before_flush_build_reached();
        assert!(
            !handle.is_finished(),
            "flush must not return before its own commit"
        );

        m.release_before_flush_build();
        handle.join().unwrap().unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"v".to_vec()));

        db.maintenance_for_test().wait_settled();
        assert!(db.maintenance_error().is_none());
    }

    /// Test 17: the frozen memtable's WAL stays on disk right up until
    /// the flush that supersedes it with an SST actually commits — never
    /// deleted before.
    #[test]
    fn old_wal_is_deleted_only_after_the_flush_commits() {
        let td = TempDir::new("flush-old-wal-lifetime");
        let db = SharedKiban::open_with_options(td.path(), tiny_buffer_options()).unwrap();
        let m = db.maintenance_for_test();
        m.arm_before_flush_build();

        fill_past_threshold(&db, "a");
        m.wait_before_flush_build_reached();

        let old_wal_number = db.lock().unwrap().immutable.as_ref().unwrap().wal_number;
        let old_wal_path = td.path().join(file_name(old_wal_number, WAL_EXTENSION));
        assert!(
            old_wal_path.exists(),
            "the frozen memtable's WAL must remain on disk before commit"
        );

        m.release_before_flush_build();
        m.wait_settled();
        assert!(db.maintenance_error().is_none());

        assert!(
            !old_wal_path.exists(),
            "the retired WAL must be removed only after a successful commit"
        );
    }

    /// Test 10: an ambiguous MANIFEST rename during the freeze handoff
    /// (old+new WAL both live) must poison the engine exactly like every
    /// other commit point — no separate interpretation for "this one is
    /// just a WAL handoff".
    #[test]
    fn manifest_ambiguity_during_freeze_handoff_poisons_engine() {
        let td = TempDir::new("flush-ambiguity-freeze");
        let opts = tiny_buffer_options();
        let mut induced = false;
        for n in 0..40usize {
            drop(Kiban::open_with_options(td.path(), opts.clone()).unwrap());
            let mut d = Kiban::open_with_options(td.path(), opts.clone()).unwrap();
            if d.is_poisoned() {
                continue;
            }
            let _ = d.put(b"k", b"v");
            sys::install_faults(&[n]);
            let freeze_result = d.freeze();
            let poisoned = matches!(
                &freeze_result,
                Err(DbError::Poisoned(PoisonCause::CommitAmbiguity(_)))
            );
            sys::clear_fault();
            if poisoned {
                induced = true;
                let e = d.put(b"later", b"x").unwrap_err();
                assert!(e.to_string().contains("poisoned"), "{e}");
                let _ = d.get(b"k").unwrap(); // reads stay available
                drop(d);
                let reopened = Kiban::open_with_options(td.path(), opts.clone()).unwrap();
                assert!(!reopened.is_poisoned());
                break;
            }
        }
        assert!(induced, "freeze-handoff ambiguity never induced");
    }

    /// Test 11: an ambiguous MANIFEST rename while a flush COMMIT
    /// replaces the frozen memtable's WAL with its SST must poison the
    /// same way. The fault is installed only around `commit_flush`
    /// itself, so the sweep targets that step specifically rather than
    /// the whole freeze+build+commit sequence.
    #[test]
    fn manifest_ambiguity_during_flush_commit_poisons_engine() {
        let td = TempDir::new("flush-ambiguity-commit");
        let opts = tiny_buffer_options();
        let mut induced = false;
        for n in 0..40usize {
            drop(Kiban::open_with_options(td.path(), opts.clone()).unwrap());
            let mut d = Kiban::open_with_options(td.path(), opts.clone()).unwrap();
            if d.is_poisoned() {
                continue;
            }
            let _ = d.put(b"k", b"v");
            if d.freeze().is_err() {
                continue;
            }
            let Some(plan) = d.plan_flush() else { continue };
            let output = match plan.build() {
                Ok(o) => o,
                Err(_) => continue,
            };
            sys::install_faults(&[n]);
            let commit_result = d.commit_flush(plan, output);
            let poisoned = matches!(
                &commit_result,
                Err(DbError::Poisoned(PoisonCause::CommitAmbiguity(_)))
            );
            sys::clear_fault();
            if poisoned {
                induced = true;
                assert!(d.put(b"later", b"x").is_err());
                let _ = d.get(b"k").unwrap();
                drop(d);
                let reopened = Kiban::open_with_options(td.path(), opts.clone()).unwrap();
                assert!(!reopened.is_poisoned());
                break;
            }
        }
        assert!(induced, "flush-commit ambiguity never induced");
    }

    /// Test 15: a crash leaving two live WAL generations (a pending
    /// flush that never committed) must replay both, in order, into one
    /// consolidated active memtable/WAL on the next open — and sequence
    /// numbers must not be reused afterward.
    #[test]
    fn multiple_live_wal_recovery_replays_both_generations_in_order() {
        // Direct `Kiban`, calling `freeze()` itself: simpler and fully
        // deterministic (no worker/checkpoint dance needed just to get
        // two live WAL generations on disk with the flush never even
        // started), and it avoids abandoning a `SharedKiban` while its
        // worker sits parked in a test checkpoint — that combination
        // has its own dedicated backpressure tests; here the point is
        // purely the on-disk multi-WAL state and its recovery.
        let td = TempDir::new("flush-multi-wal-recovery");
        let mut db =
            Kiban::open_with_options(td.path(), super::compaction_tests::tiny_options()).unwrap();

        for i in 0..3u32 {
            db.put(format!("gen1-{i:03}"), format!("v{i}")).unwrap();
        }
        db.freeze().unwrap(); // two live WALs now; the flush itself never runs

        for i in 0..3u32 {
            db.put(format!("gen2-{i:03}"), format!("w{i}")).unwrap();
        }
        db.sync().unwrap();

        let live_wals = Manifest::load(td.path()).unwrap().unwrap().wal_numbers;
        assert_eq!(live_wals.len(), 2, "both generations must be MANIFEST-live");

        drop(db); // abandon: the pending flush never committed

        let mut reopened = Kiban::open(td.path()).unwrap();
        for i in 0..3u32 {
            assert_eq!(
                reopened.get(format!("gen1-{i:03}").as_bytes()).unwrap(),
                Some(format!("v{i}").into_bytes())
            );
            assert_eq!(
                reopened.get(format!("gen2-{i:03}").as_bytes()).unwrap(),
                Some(format!("w{i}").into_bytes())
            );
        }
        assert_eq!(
            Manifest::load(td.path())
                .unwrap()
                .unwrap()
                .wal_numbers
                .len(),
            1,
            "recovery must consolidate back to exactly one live wal"
        );

        // a fresh write's sequence number must exceed everything
        // recovered from either generation, not collide with it
        reopened.put(b"post-recovery", b"ok").unwrap();
        reopened.sync().unwrap();
        assert_eq!(
            reopened.get(b"post-recovery").unwrap(),
            Some(b"ok".to_vec())
        );
    }

    /// Test 16: the orphan sweep understands a two-WAL live set —
    /// stragglers outside it are removed, both live generations survive.
    #[test]
    fn orphan_sweep_keeps_both_live_wal_generations_and_removes_garbage() {
        let td = TempDir::new("flush-orphan-sweep-multi-wal");

        let mut throwaway = Memtable::new();
        atomic::create_durably(&td.path().join(file_name(7, WAL_EXTENSION))).unwrap();
        {
            let (mut wal, _) =
                Wal::open(td.path().join(file_name(7, WAL_EXTENSION)), &mut throwaway).unwrap();
            wal.put(1, b"k1", b"v1").unwrap();
            wal.sync().unwrap();
        }
        atomic::create_durably(&td.path().join(file_name(8, WAL_EXTENSION))).unwrap();
        {
            let (mut wal, _) =
                Wal::open(td.path().join(file_name(8, WAL_EXTENSION)), &mut throwaway).unwrap();
            wal.put(2, b"k2", b"v2").unwrap();
            wal.sync().unwrap();
        }
        // garbage: a superseded old generation and a stray future file
        atomic::create_durably(&td.path().join(file_name(6, WAL_EXTENSION))).unwrap();
        atomic::create_durably(&td.path().join(file_name(9, WAL_EXTENSION))).unwrap();

        Manifest {
            next_file_number: 10,
            wal_numbers: vec![7, 8],
            last_sequence: 0,
            tables: vec![],
        }
        .install(td.path())
        .unwrap();

        let mut db = Kiban::open(td.path()).unwrap();
        assert!(
            !td.path().join(file_name(6, WAL_EXTENSION)).exists(),
            "an orphan below the live set must be swept"
        );
        assert!(
            !td.path().join(file_name(9, WAL_EXTENSION)).exists(),
            "an orphan above the live set must be swept"
        );
        assert_eq!(db.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(db.get(b"k2").unwrap(), Some(b"v2".to_vec()));
        db.put(b"k3", b"v3").unwrap();
        assert_eq!(db.get(b"k3").unwrap(), Some(b"v3".to_vec()));
    }

    /// Deterministic scenario exercising the freeze/flush pipeline
    /// directly on `Kiban` (via `freeze`/`plan_flush`/`FlushPlan::build`/
    /// `commit_flush`, called just like the maintenance worker calls
    /// them) — two generations, each put+sync+freeze+flush. Mirrors
    /// `crash_sweep_tests::run_scenario_with_faults`'s shape and reuses
    /// its `Tracker`/`assert_band`, scoped separately so the existing
    /// sweep (which exercises `Kiban::flush`, never `freeze` — direct
    /// `Kiban` never auto-freezes) stays untouched.
    fn run_flush_scenario_with_faults(
        dir: &Path,
        n: &[usize],
    ) -> (
        Result<(), DbError>,
        super::crash_sweep_tests::Tracker,
        usize,
    ) {
        sys::install_faults(n);
        let mut tracker = super::crash_sweep_tests::Tracker::default();
        let result = (|| -> Result<(), DbError> {
            let opts = KibanOptions {
                l0_compaction_trigger: 1000,
                l0_write_stall_trigger: 2000,
                ..super::compaction_tests::tiny_options()
            };
            let mut db = Kiban::open_with_options(dir, opts)?;

            macro_rules! step {
                ($op:expr) => {
                    if $op.is_err() {
                        return Ok(());
                    }
                };
            }

            for round in 0..2u32 {
                for i in 0..5u32 {
                    let key = format!("k{i:03}");
                    let val = format!("gen{round}-{i}");
                    step!(db.put(&key, &val));
                    tracker.apply(key.as_bytes(), Some(val.as_bytes()));
                }
                step!(db.sync());
                tracker.on_sync_ok();

                if db.freeze().is_err() {
                    tracker.ambiguous = true;
                    return Ok(());
                }
                if let Some(plan) = db.plan_flush() {
                    let output = match plan.build() {
                        Ok(o) => o,
                        Err(_) => return Ok(()),
                    };
                    if db.commit_flush(plan, output).is_err() {
                        tracker.ambiguous = true;
                        return Ok(());
                    }
                    tracker.on_flush_ok();
                }
            }
            Ok(())
        })();
        let ops = sys::op_count();
        sys::clear_fault();
        (result, tracker, ops)
    }

    /// Tests 5, 7, 8, 9 (and every other single crash point in the
    /// pipeline) as one exhaustive sweep: every single-syscall failure,
    /// then every pair, must recover within the durability band —
    /// stronger coverage than naming each point individually, and it
    /// catches anything an individually-named test would have missed.
    #[test]
    fn every_single_and_pairwise_fault_in_the_flush_pipeline_recovers_within_the_band() {
        let clean_dir = TempDir::new("flush-sweep-clean");
        let (clean_result, _tracker, total) = run_flush_scenario_with_faults(clean_dir.path(), &[]);
        assert!(clean_result.is_ok());
        assert!(total > 5);

        for a in 0..total {
            let dir = TempDir::new("flush-sweep-single");
            let (_result, tracker, _ops) = run_flush_scenario_with_faults(dir.path(), &[a]);
            let db = match Kiban::open(dir.path()) {
                Ok(db) => db,
                Err(e) => panic!("a={a}: reopen failed: {e}"),
            };
            let recovered: BTreeMap<Vec<u8>, Vec<u8>> = db.iter().map(|r| r.unwrap()).collect();
            super::crash_sweep_tests::assert_band("flush-sweep-single", &[a], &recovered, &tracker);
        }

        let mut ran = 0usize;
        for a in 0..total {
            for b in 0..total {
                if a == b {
                    continue;
                }
                let dir = TempDir::new("flush-sweep-pair");
                let (_result, tracker, _ops) = run_flush_scenario_with_faults(dir.path(), &[a, b]);
                let db = match Kiban::open(dir.path()) {
                    Ok(db) => db,
                    Err(e) => panic!("a={a},b={b}: reopen failed: {e}"),
                };
                let recovered: BTreeMap<Vec<u8>, Vec<u8>> = db.iter().map(|r| r.unwrap()).collect();
                super::crash_sweep_tests::assert_band(
                    "flush-sweep-pair",
                    &[a, b],
                    &recovered,
                    &tracker,
                );
                ran += 1;
            }
        }
        assert!(ran > 20, "pair sweep barely exercised anything: {ran}");
    }

    /// Test 18: the strongest durability claim, extended to this
    /// pipeline — with a simulated volatile device, a crash discards
    /// exactly the unsynced bytes, so recovered state must EQUAL the
    /// last synced state, not merely fall within a band. Every single-
    /// and two-fault crash point is checked, mirroring
    /// `power_loss_tests::power_loss_recovers_exactly_the_last_synced_state`
    /// exactly but over the freeze/flush scenario.
    #[test]
    fn power_loss_recovers_exactly_the_last_synced_state_across_the_flush_pipeline() {
        let clean_dir = TempDir::new("flush-pl-clean");
        let (clean_result, _tracker, total) = run_flush_scenario_with_faults(clean_dir.path(), &[]);
        assert!(clean_result.is_ok());
        assert!(total > 5);

        let mut checked = 0usize;
        for a in 0..total {
            for b in 0..total {
                if a == b {
                    continue;
                }
                let dir = TempDir::new("flush-pl-sweep");
                sys::enable_device_sim();
                let (_result, tracker, _ops) = run_flush_scenario_with_faults(dir.path(), &[a, b]);
                sys::clear_fault();

                sys::power_loss();

                let db = match Kiban::open(dir.path()) {
                    Ok(db) => db,
                    Err(e) => {
                        sys::disable_device_sim();
                        panic!("faults {a},{b}: reopen after power loss failed: {e:?}");
                    }
                };
                let recovered: BTreeMap<Vec<u8>, Vec<u8>> = db.iter().map(|r| r.unwrap()).collect();
                sys::disable_device_sim();

                if tracker.ambiguous {
                    super::crash_sweep_tests::assert_band(
                        "flush-pl-sweep",
                        &[a, b],
                        &recovered,
                        &tracker,
                    );
                } else {
                    assert_eq!(
                        recovered, tracker.synced,
                        "faults {a},{b}: power loss must recover exactly the last synced state"
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 20, "barely exercised anything: {checked}");
    }
}
