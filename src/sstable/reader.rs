//! Table reader: lazy table handles with cached block reads.
//!
//! Per `ARCHITECTURE.md` D1: opening reads only footer,
//! index, and bloom filter; data blocks load on demand through the
//! shared LRU. Corruption is detected and reported, never repaired.
//!
//! Per phase 11.6: an `SstTable` does not permanently own an open file
//! descriptor. Every read leases one from the shared `TableFileCache`
//! for just the duration of that read — see `read_block`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::block::{BlockIter, VerifiedBlock};
use super::builder::{
    FILTER_FORMAT_VERSION, FILTER_POINT_FORMAT_VERSION, FOOTER_LEN, FORMAT_VERSION, MAGIC,
    POINT_FOOTER_EXTRA, POINT_FORMAT_VERSION,
};
use super::filter::{BLOCK_TYPE_FILTER, SstFilter};
use super::point::{PointIndex, compact::CompactIndex};
use super::separator::Separator;
use super::{Kind, SstError};
use crate::bloom::BloomFilter;
use crate::cache::{BlockCache, CachedBlock};
use crate::crc32;
use crate::file_cache::TableFileCache;
use crate::range_cache::Window;

#[repr(align(64))]
struct IndexEntry {
    separator: Separator,
    offset: u64,
    len: u64,
}

unsafe extern "C" {
    fn mmap(
        addr: *mut std::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut std::ffi::c_void;
    fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
}
const PROT_READ: i32 = 0x1;
const MAP_SHARED: i32 = 0x1;
const MAP_FAILED: i64 = -1;

/// 11.17-B/mmap-hybrid: a whole-file, read-only `mmap(2)` mapping,
/// opt-in per table via `KibanOptions::mmap_max_level` (disabled by
/// default — see there for the warm-path-win/cold-tail-risk tradeoff
/// this exists to capture only for tables actually worth the risk).
///
/// Sound specifically because of two Kiban-specific guarantees, not
/// general-purpose mmap safety: (1) an SST file is fully written once
/// via `atomic::commit_file` and never modified or truncated after —
/// so a mapping sized to the validated `file_len` at open time can
/// never see a SIGBUS from the file shrinking underneath it; (2) a
/// later compaction that retires this file only `unlink`s it — POSIX
/// guarantees a mapping's pages stay valid after unlink, identically
/// to how a held file descriptor would, so no lifetime coordination
/// with `reclaim_obsolete` is needed at all.
struct MappedFile {
    addr: *mut std::ffi::c_void,
    len: usize,
}

// SAFETY: PROT_READ-only, never mutated after construction (see the
// struct doc for why the underlying bytes themselves never change
// either), so sharing `&MappedFile`/sending it across threads is sound
// the same way sharing any other read-only, immutable byte buffer is.
unsafe impl Send for MappedFile {}
unsafe impl Sync for MappedFile {}

impl MappedFile {
    /// A short-lived `fd`, open only long enough to create the
    /// mapping: POSIX guarantees the mapping outlives the descriptor
    /// that created it, so this does not hold a `TableFileCache` slot
    /// (or any descriptor at all) for the table's lifetime.
    fn open(path: &Path, len: u64) -> std::io::Result<MappedFile> {
        let len = len as usize;
        if len == 0 {
            return Err(std::io::Error::other("cannot mmap a zero-length file"));
        }
        let file = std::fs::File::open(path)?;
        use std::os::unix::io::AsRawFd;
        let addr = unsafe {
            mmap(
                std::ptr::null_mut(),
                len,
                PROT_READ,
                MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        drop(file);
        if addr as i64 == MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(MappedFile { addr, len })
    }

    fn as_slice(&self) -> &[u8] {
        // SAFETY: `addr`/`len` came from a successful `mmap` of exactly
        // `len` bytes, PROT_READ, held for `self`'s entire lifetime;
        // see the struct doc for why those bytes never change.
        unsafe { std::slice::from_raw_parts(self.addr as *const u8, self.len) }
    }
}

impl Drop for MappedFile {
    fn drop(&mut self) {
        // SAFETY: `addr`/`len` are exactly what the successful `mmap`
        // in `open` returned/was called with; not unmapped anywhere
        // else (this is the only place that calls `munmap`).
        unsafe {
            munmap(self.addr, self.len);
        }
    }
}

pub struct SstTable {
    // Debug is implemented manually below: neither the file cache nor
    // the block cache derive it, and their contents aren't useful here
    // anyway.
    number: u64,
    path: PathBuf,
    file_len: u64,
    index: Vec<IndexEntry>,
    filter: SstFilter,
    point: Option<Box<PointIndex>>,
    locators: Option<Box<CompactIndex>>,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    block_cache: Arc<BlockCache>,
    file_cache: Arc<TableFileCache>,
    /// `Some` only when this table was opened with `use_mmap: true`
    /// AND the `mmap(2)` call itself succeeded — a failure there
    /// (e.g. address-space exhaustion) degrades silently to the
    /// existing `file_cache`-leased pread path, never fails the whole
    /// table open, since this is a pure performance opt-in.
    mmap: Option<MappedFile>,
}

impl std::fmt::Debug for SstTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SstTable")
            .field("number", &self.number)
            .field("file_len", &self.file_len)
            .field("tables", &self.index.len())
            .finish()
    }
}

pub struct Found {
    pub kind: Kind,
    pub seq: u64,
    pub value: Vec<u8>,
}

impl SstTable {
    /// Opens a handle: footer + index + filter only, each read through
    /// a leased file-cache handle rather than a descriptor this table
    /// keeps for itself. Data blocks load on demand through
    /// `block_cache`, each lazily leasing `file_cache` on a miss.
    ///
    /// Crate-internal: `file_cache`'s type is itself crate-internal
    /// (11.6), so opening a table is something only `Kiban`'s own
    /// machinery does, never a direct external construction path.
    pub(crate) fn open(
        number: u64,
        path: &Path,
        block_cache: Arc<BlockCache>,
        file_cache: Arc<TableFileCache>,
        use_mmap: bool,
    ) -> Result<SstTable, SstError> {
        let bad = |m: String| SstError::Corrupt(m);
        let open_err =
            |e: std::io::Error| SstError::Corrupt(format!("table {number} cannot be opened: {e}"));

        let (footer, file_len) = {
            let lease = file_cache.acquire(number, path).map_err(open_err)?;
            let file_len = lease.len().map_err(|e| bad(e.to_string()))?;
            if file_len < FOOTER_LEN as u64 {
                return Err(bad("file is smaller than the footer".to_string()));
            }
            let footer = lease
                .read_range_at(path, file_len - FOOTER_LEN as u64, FOOTER_LEN as u64)
                .map_err(|e| bad(e.to_string()))?;
            (footer, file_len)
        };
        let footer = &footer[..];
        if &footer[36..44] != MAGIC {
            return Err(bad("bad magic number; not a kiban sstable".to_string()));
        }
        let version = u32::from_le_bytes(footer[16..20].try_into().unwrap());
        if ![
            FORMAT_VERSION,
            POINT_FORMAT_VERSION,
            FILTER_FORMAT_VERSION,
            FILTER_POINT_FORMAT_VERSION,
        ]
        .contains(&version)
        {
            return Err(bad(format!("unsupported format version {version}")));
        }
        let index_offset = u64::from_le_bytes(footer[0..8].try_into().unwrap());
        let index_len = u64::from_le_bytes(footer[8..16].try_into().unwrap());
        let filter_offset = u64::from_le_bytes(footer[20..28].try_into().unwrap());
        let filter_len = u64::from_le_bytes(footer[28..36].try_into().unwrap());
        let point_range =
            if version == POINT_FORMAT_VERSION || version == FILTER_POINT_FORMAT_VERSION {
                if file_len < (FOOTER_LEN + POINT_FOOTER_EXTRA) as u64 {
                    return Err(bad("truncated point footer".into()));
                }
                let lease = file_cache.acquire(number, path).map_err(open_err)?;
                let extra = lease
                    .read_range_at(
                        path,
                        file_len - (FOOTER_LEN + POINT_FOOTER_EXTRA) as u64,
                        POINT_FOOTER_EXTRA as u64,
                    )
                    .map_err(|e| bad(e.to_string()))?;
                let state = crc32::update(crc32::INITIAL, &extra[..20]);
                let checksum = crc32::finalize(crc32::update(state, footer));
                if checksum != u32::from_le_bytes(extra[20..24].try_into().unwrap())
                    || u32::from_le_bytes(extra[16..20].try_into().unwrap()) != 2
                {
                    return Err(bad("point footer checksum or feature flags".into()));
                }
                let offset = u64::from_le_bytes(extra[..8].try_into().unwrap());
                let len = u64::from_le_bytes(extra[8..16].try_into().unwrap());
                if len < 37 || offset.checked_add(len) != Some(filter_offset) {
                    return Err(bad("point section out of bounds".into()));
                }
                Some((offset, len))
            } else {
                None
            };
        let data_end = file_len
            - FOOTER_LEN as u64
            - if point_range.is_some() {
                POINT_FOOTER_EXTRA as u64
            } else {
                0
            };
        if index_len == 0
            || index_len > crate::limits::MAX_RECORD_BYTES as u64
            || filter_len > crate::limits::MAX_RECORD_BYTES as u64
            || index_offset
                .checked_add(index_len)
                .is_none_or(|end| end != data_end)
        {
            return Err(bad("index block out of file bounds".to_string()));
        }
        if filter_offset.checked_add(filter_len) != Some(index_offset) || filter_len < 10 {
            return Err(bad(
                "filter block does not sit directly before the index".to_string()
            ));
        }

        let (filter_raw, index_raw) = {
            let lease = file_cache.acquire(number, path).map_err(open_err)?;
            let filter_raw = lease
                .read_range_at(path, filter_offset, filter_len)
                .map_err(|e| bad(e.to_string()))?;
            let index_raw = lease
                .read_range_at(path, index_offset, index_len)
                .map_err(|e| bad(e.to_string()))?;
            (filter_raw, index_raw)
        };
        let tagged = version == FILTER_FORMAT_VERSION || version == FILTER_POINT_FORMAT_VERSION;
        verify_typed_trailer(
            &filter_raw,
            "filter",
            if tagged { BLOCK_TYPE_FILTER } else { 0 },
        )?;
        let payload = &filter_raw[..filter_raw.len() - 5];
        let filter = if tagged {
            SstFilter::decode(payload)
        } else {
            BloomFilter::decode(payload).map(SstFilter::Legacy)
        }
        .ok_or_else(|| bad("filter block payload is malformed".to_string()))?;
        let index = parse_index(
            &index_raw,
            point_range.map_or(filter_offset, |(offset, _)| offset),
        )?;
        let mut locators = None;
        let point = if let Some((offset, len)) = point_range {
            let lease = file_cache.acquire(number, path).map_err(open_err)?;
            let resident = (!block_cache.prefer_compact_points())
                .then(|| {
                    offset
                        .checked_add(len)
                        .and_then(|n| usize::try_from(n).ok())
                        .and_then(|n| block_cache.reserve_point(n))
                })
                .flatten();
            let header = lease
                .read_range_at(path, offset, len.min(64))
                .map_err(|e| bad(e.to_string()))?;
            let locator_reservation = if resident.is_none() {
                CompactIndex::locator_bytes(&header, len, offset)
                    .and_then(|n| block_cache.reserve_point(n))
            } else {
                None
            };
            if let Some(reservation) = resident {
                let raw = lease
                    .read_shared_range_at(path, offset, len)
                    .map_err(|e| bad(e.to_string()))?;
                let data = lease
                    .read_shared_range_at(path, 0, offset)
                    .map_err(|e| bad(e.to_string()))?;
                Some(Box::new(PointIndex::decode(
                    raw,
                    data,
                    reservation,
                    index.iter().map(|entry| (entry.offset, entry.len)),
                )?))
            } else if let Some(reservation) = locator_reservation {
                let raw = lease
                    .read_shared_range_at(path, offset, len)
                    .map_err(|e| bad(e.to_string()))?;
                locators = CompactIndex::decode_locators(
                    raw,
                    usize::try_from(offset).map_err(|_| bad("backing offset overflow".into()))?,
                    reservation,
                    index.iter().map(|entry| (entry.offset, entry.len)),
                    |start, amount| {
                        lease
                            .read_range_at(path, start as u64, amount as u64)
                            .map(std::borrow::Cow::Owned)
                            .map_err(|e| bad(e.to_string()))
                    },
                )?
                .map(Box::new);
                None
            } else {
                let header = lease
                    .read_range_at(path, offset, 8)
                    .map_err(|e| bad(e.to_string()))?;
                if header != b"KIBANPH2" && header != b"KIBANPH3" {
                    return Err(bad("unsupported point section encoding".into()));
                }
                // Even an unused optional section must pass its checksum.
                // Stream it so a small cache never allocates a whole SST arena.
                let mut state = crc32::INITIAL;
                let mut pos = 0;
                while pos < len - 4 {
                    let amount = (len - 4 - pos).min(64 * 1024);
                    let raw = lease
                        .read_range_at(path, offset + pos, amount)
                        .map_err(|e| bad(e.to_string()))?;
                    state = crc32::update(state, &raw);
                    pos += amount;
                }
                let trailer = lease
                    .read_range_at(path, offset + len - 5, 5)
                    .map_err(|e| bad(e.to_string()))?;
                if trailer[0] != 0
                    || crc32::finalize(state)
                        != u32::from_le_bytes(trailer[1..].try_into().unwrap())
                {
                    return Err(bad("point index checksum mismatch".into()));
                }
                None
            }
        } else {
            None
        };

        // Best-effort: a failed mmap degrades silently to the pread
        // path (see `SstTable::mmap`'s doc), never fails the open.
        let mmap = use_mmap
            .then(|| MappedFile::open(path, file_len).ok())
            .flatten();

        let mut table = SstTable {
            number,
            path: path.to_path_buf(),
            file_len,
            index,
            filter,
            point,
            locators,
            first_key: Vec::new(),
            last_key: Vec::new(),
            block_cache,
            file_cache,
            mmap,
        };

        // Boundary keys come from the boundary blocks (two cached
        // reads, each its own brief file-cache lease on a miss).
        let first_block = table.read_block(&table.index[0])?;
        let (_, _, first, _) = first_block
            .first_entry()?
            .ok_or_else(|| bad("first block yielded no entry".to_string()))?;
        let last_entry_ref = table.index.last().expect("parse rejects empty index");
        let last_block = table.read_block(last_entry_ref)?;
        let (_, _, last, _) = last_block
            .last_entry()?
            .ok_or_else(|| bad("last block yielded no entry".to_string()))?;
        table.first_key = first;
        table.last_key = last;
        Ok(table)
    }

    pub(crate) fn scan_index(&self) -> Option<&super::point::compact::CompactIndex> {
        match self.point.as_deref()? {
            PointIndex::Compact(index) if index.scan_config().is_some() => Some(index),
            _ => None,
        }
    }
    pub(crate) fn scan_cache(&self) -> &Arc<BlockCache> {
        &self.block_cache
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn size_on_disk(&self) -> u64 {
        self.file_len
    }

    pub fn smallest_key(&self) -> &[u8] {
        &self.first_key
    }

    pub fn largest_key(&self) -> &[u8] {
        &self.last_key
    }

    /// A block-cache hit needs no file descriptor (or mmap access) at
    /// all — memory hit means memory hit. Only a miss reads through,
    /// via `self.mmap` when this table was opened with it (11.17-mmap-
    /// hybrid: no syscall at all for OS-page-cache-resident bytes, at
    /// the cost of the file's whole address-space mapping living for
    /// the table's lifetime) or `file_cache` otherwise (11.6: a lease
    /// held only for the duration of the positioned read itself).
    ///
    /// Concurrent misses on the SAME block (11.17-F: multiple readers
    /// racing the same not-yet-cached key) are coalesced through
    /// `BlockCache::get_or_load`: only the first caller actually reads
    /// (leases a descriptor, or slices the mapping); the rest wait for
    /// its result.
    fn read_block(&self, entry: &IndexEntry) -> Result<VerifiedBlock, SstError> {
        if let Some(point) = &self.point {
            return Ok(point.block_at(entry.offset));
        }
        let key = (self.number, entry.offset);
        let number = self.number;
        let path = &self.path;
        let file_cache = &self.file_cache;
        let mmap = self.mmap.as_ref();
        let cached = self
            .block_cache
            .get_or_load(key, || -> Result<CachedBlock, SstError> {
                let data = if let Some(m) = mmap {
                    let start = entry.offset as usize;
                    let end = start + entry.len as usize;
                    let slice = m.as_slice();
                    if end > slice.len() {
                        return Err(SstError::Corrupt(format!(
                            "mmap read past end of file at offset {}",
                            entry.offset
                        )));
                    }
                    slice[start..end].to_vec()
                } else {
                    let lease = file_cache.acquire(number, path).map_err(|e| {
                        SstError::Corrupt(format!("table {number} cannot be opened: {e}"))
                    })?;
                    lease
                        .read_range_at(path, entry.offset, entry.len)
                        .map_err(|e| {
                            SstError::Corrupt(format!(
                                "read failed at offset {}: {e}",
                                entry.offset
                            ))
                        })?
                };
                let meta = VerifiedBlock::verify(&data)?;
                Ok(CachedBlock {
                    data: Arc::from(data),
                    meta,
                })
            })
            .map_err(SstError::Corrupt)?;
        Ok(VerifiedBlock::from_cached(cached))
    }

    /// Sequential-iterator block read. Identical to `read_block` unless the
    /// block cache has an enabled range tier and this table is not mapped.
    /// Then a block-cache miss is served, in order, from the iterator's
    /// pinned window, from a resident range-tier window (which becomes the
    /// pin), or by one positional read of a forward window of whole blocks
    /// that is admitted to the range tier and pinned. Blocks inside a pinned
    /// window therefore cost no lock and no syscall.
    ///
    /// An iterator's first `READS_BEFORE_READAHEAD` source reads fetch only
    /// their block and are not admitted, so short scans read nothing extra;
    /// after that each source read doubles the window up to
    /// `BlockCache::readahead_bytes`. Range-tier
    /// entries are unions of whole blocks, so a block is either entirely
    /// inside a resident range or not in the tier at all.
    fn read_block_sequential(
        &self,
        idx: usize,
        state: &mut ReadAhead,
    ) -> Result<VerifiedBlock, SstError> {
        if let Some(point) = &self.point {
            return Ok(point.block(idx));
        }
        let ranges = self.block_cache.range_cache();
        let entry = &self.index[idx];
        if self.mmap.is_some() || !ranges.is_enabled() {
            return self.read_block(entry);
        }
        let number = self.number;
        let span = entry.offset..entry.offset + entry.len;
        let mut fetched = false;
        let cached = self
            .block_cache
            .get_or_load(
                (number, entry.offset),
                || -> Result<CachedBlock, SstError> {
                    let pinned = state.pinned.as_ref().filter(|(start, window)| {
                        *start <= span.start && span.end <= start + window.len() as u64
                    });
                    if pinned.is_none() {
                        state.pinned = ranges.get_window(number, span.clone());
                    }
                    if state.pinned.as_ref().is_none_or(|(start, window)| {
                        span.start < *start || start + (window.len() as u64) < span.end
                    }) {
                        state.pinned = Some(self.read_window(idx, state.window)?);
                        fetched = true;
                    }
                    let (start, window) = state.pinned.as_ref().expect("window pinned above");
                    let offset = (span.start - start) as usize;
                    let data = &window[offset..offset + entry.len as usize];
                    let meta = VerifiedBlock::verify(data)?;
                    Ok(CachedBlock {
                        data: Arc::from(data),
                        meta,
                    })
                },
            )
            .map_err(SstError::Corrupt)?;
        if fetched {
            state.source_reads += 1;
            let max = self.block_cache.readahead_bytes();
            state.window = if state.source_reads < READS_BEFORE_READAHEAD {
                0
            } else if state.window == 0 {
                INITIAL_READAHEAD_BYTES.min(max)
            } else {
                state.window.saturating_mul(2).min(max)
            };
        }
        Ok(VerifiedBlock::from_cached(cached))
    }

    /// One positional read starting at block `idx`, extended over whole
    /// following blocks up to `window` bytes but stopping before bytes the
    /// range tier already holds. Multi-block windows are admitted.
    fn read_window(&self, idx: usize, window: u64) -> Result<(u64, Window), SstError> {
        let ranges = self.block_cache.range_cache();
        let number = self.number;
        let entry = &self.index[idx];
        let start = entry.offset;
        let mut end = start + entry.len;
        if window > entry.len {
            for next in &self.index[idx + 1..] {
                let candidate = next.offset + next.len;
                if candidate - start > window {
                    break;
                }
                end = candidate;
            }
            if let Some(gap) = ranges.missing_ranges(number, start..end).first()
                && gap.start == start
            {
                end = gap.end;
            }
        }
        let lease = self
            .file_cache
            .acquire(number, &self.path)
            .map_err(|e| SstError::Corrupt(format!("table {number} cannot be opened: {e}")))?;
        let data = lease
            .read_range_at(&self.path, start, end - start)
            .map_err(|e| SstError::Corrupt(format!("read failed at offset {start}: {e}")))?;
        drop(lease);
        ranges.record_source_read(data.len());
        let data: Window = Arc::new(data);
        if end > entry.offset + entry.len {
            ranges.insert_window(number, start, &data);
        }
        Ok((start, data))
    }

    /// Point lookup. Returns the newest version of `key` whose sequence
    /// number does not exceed `limit`, or `None` when provably absent.
    pub fn get(&self, key: &[u8], limit: Option<u64>) -> Result<Option<Found>, SstError> {
        // A table carrying BOTH a resident point index and a binary fuse
        // filter asks the (much smaller, cache-independent) filter first: a
        // fuse "absent" is a proof of absence, because the construction has
        // zero false negatives and the builder verifies every published
        // filter against all of its sorted source keys before the table can
        // ever become visible. Missing the filter skips the resident
        // point-index probe entirely; a positive answer falls through to the
        // unchanged point-index path. Bloom tables, positive-fallback
        // tables (`SstFilter::Positive`, guard rejects them) and tables
        // without the fuse filter keep the original ordering below.
        if let Some(point) = &self.point {
            if matches!(self.filter, SstFilter::Fuse(_)) && !self.filter.may_contain(key) {
                return Ok(None);
            }
            return Ok(point.get(key, limit));
        }
        if let Some(locators) = &self.locators {
            let Some(v) = locators.locate(key, limit) else {
                return Ok(None);
            };
            let value = if let Some(m) = &self.mmap {
                m.as_slice()[v.offset..v.offset + v.len].to_vec()
            } else {
                let lease = self
                    .file_cache
                    .acquire(self.number, &self.path)
                    .map_err(|e| SstError::Corrupt(format!("point file cannot be opened: {e}")))?;
                lease
                    .read_range_at(&self.path, v.offset as u64, v.len as u64)
                    .map_err(|e| SstError::Corrupt(format!("point value read failed: {e}")))?
            };
            // Verify the owned copy, not the mutable file page that supplied it.
            if crc32::crc32(&value) != v.checksum {
                return Err(SstError::Corrupt("point value checksum mismatch".into()));
            }
            return Ok(Some(Found {
                kind: v.kind,
                seq: v.seq,
                value,
            }));
        }
        if !self.filter.may_contain(key) {
            return Ok(None);
        }
        let idx = self.index.partition_point(|e| e.separator.as_slice() < key);
        if idx == self.index.len() {
            return Ok(None);
        }
        let block = self.read_block(&self.index[idx])?;
        Ok(block.get(key, limit)?.map(|m| Found {
            kind: m.kind,
            seq: m.seq,
            value: m.value.to_vec(),
        }))
    }

    /// Iterates from the first key >= `target`. Positions at the right
    /// block via separators; nothing is scanned from the file start.
    pub fn iter_from(&self, target: &[u8]) -> Iter<'_> {
        let mut it = self.iter();
        it.next_block = self
            .index
            .partition_point(|e| e.separator.as_slice() < target);
        it.lower_bound = Some(target.to_vec());
        it
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter {
            table: self,
            next_block: 0,
            current: None,
            failed: false,
            lower_bound: None,
            readahead: ReadAhead::default(),
        }
    }

    /// A dedicated bulk/sequential reader for compaction's k-way merge
    /// (11.17-C), never for foreground point lookups or scans: compaction
    /// always reads a table start-to-finish exactly once and its output
    /// is unlikely to be re-read soon, so routing it through
    /// `read_block` would (a) evict hotter foreground blocks from
    /// `block_cache` for no benefit and (b) lease `file_cache` — a
    /// shared, bounded descriptor pool foreground GETs also need — once
    /// per ~block-sized positional read instead of once for the whole
    /// scan. `compaction_iter` opens an independent handle for each refill
    /// (never touches `file_cache`) and reads ahead in large sequential
    /// chunks (see `CompactionIter::read_block_bulk`), touching neither
    /// shared cache at all.
    pub(crate) fn compaction_iter(&self) -> Result<CompactionIter<'_>, SstError> {
        self.compaction_iter_with_window(COMPACTION_READ_AHEAD_BYTES)
    }

    /// Same as `compaction_iter`, with the read-ahead window size
    /// exposed — production always uses `COMPACTION_READ_AHEAD_BYTES`
    /// (see `compaction_iter`); tests use a tiny window here to force
    /// many refills over a small fixture table, exercising the same
    /// refill logic a multi-megabyte production table would.
    fn compaction_iter_with_window(
        &self,
        window_bytes: u64,
    ) -> Result<CompactionIter<'_>, SstError> {
        Ok(CompactionIter {
            table: self,
            next_block: 0,
            buffer: Vec::new(),
            buffer_start: 0,
            current: None,
            failed: false,
            window_bytes,
        })
    }
}

/// How far ahead `CompactionIter` reads in one sequential syscall —
/// covers many blocks per read instead of one syscall per block. Not
/// tuned against real hardware this phase; chosen to comfortably
/// exceed `KibanOptions::target_file_size`'s default (4 MiB) so a
/// typical table needs only one or two refills total.
const COMPACTION_READ_AHEAD_BYTES: u64 = 4 * 1024 * 1024;

/// First foreground read-ahead window after an iterator's initial
/// single-block source read; see `SstTable::read_block_sequential`.
const INITIAL_READAHEAD_BYTES: u64 = 16 * 1024;

/// Single-block source reads an iterator makes before reading ahead, as in
/// RocksDB's automatic iterator read-ahead: one block boundary crossed by a
/// short scan must not fetch blocks it never uses.
const READS_BEFORE_READAHEAD: u32 = 2;

pub(crate) struct CompactionIter<'a> {
    table: &'a SstTable,
    next_block: usize,
    /// Bytes covering `[buffer_start, buffer_start + buffer.len())` of
    /// the file, refilled forward as blocks are consumed. Never
    /// touches `block_cache`: every block handed out is its own owned
    /// copy sliced from here, so the buffer's own lifetime (replaced
    /// wholesale on each refill) can't outlive what callers hold.
    buffer: Vec<u8>,
    buffer_start: u64,
    current: Option<BlockIter>,
    failed: bool,
    /// How far ahead one refill reads — `COMPACTION_READ_AHEAD_BYTES`
    /// in production, deliberately tiny in some tests to force many
    /// refills over a small fixture table.
    window_bytes: u64,
}

impl CompactionIter<'_> {
    fn read_block_bulk(&mut self, idx: usize) -> Result<VerifiedBlock, SstError> {
        let entry = &self.table.index[idx];
        let (start, len) = (entry.offset, entry.len);
        let buffer_end = self.buffer_start + self.buffer.len() as u64;
        if self.buffer.is_empty() || start < self.buffer_start || start + len > buffer_end {
            // Forward read-ahead window starting at this block: extend
            // past it to include as many subsequent (whole) blocks as
            // fit within window_bytes. Always covers at least this one
            // block, even if it alone exceeds the nominal window.
            let mut window_end = start + len;
            for e in &self.table.index[idx..] {
                let candidate_end = e.offset + e.len;
                if candidate_end > start + self.window_bytes {
                    break;
                }
                window_end = candidate_end;
            }
            let mut buf = vec![0u8; (window_end - start) as usize];
            // Keep only the active refill descriptor open. A merge can have
            // many source iterators without pinning one descriptor per input.
            let file = crate::sys::File::open_read(&self.table.path)
                .map_err(|e| SstError::Corrupt(format!("compaction open: {e}")))?;
            file.read_exact_at(&self.table.path, &mut buf, start)
                .map_err(|e| {
                    SstError::Corrupt(format!("compaction read failed at offset {start}: {e}"))
                })?;
            self.buffer = buf;
            self.buffer_start = start;
        }
        let rel = (start - self.buffer_start) as usize;
        VerifiedBlock::from_raw(self.buffer[rel..rel + len as usize].to_vec())
    }
}

impl Iterator for CompactionIter<'_> {
    type Item = Result<(Kind, u64, Vec<u8>, Vec<u8>), SstError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(state) = &mut self.current {
                match state.next() {
                    Some(Ok(item)) => return Some(Ok(item)),
                    Some(Err(e)) => {
                        self.failed = true;
                        return Some(Err(e));
                    }
                    None => self.current = None,
                }
            }
            if self.next_block >= self.table.index.len() {
                return None;
            }
            match self.read_block_bulk(self.next_block) {
                Ok(block) => {
                    self.current = Some(BlockIter::from_verified(block));
                    self.next_block += 1;
                }
                Err(e) => {
                    self.failed = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

pub(super) fn verify_trailer(raw: &[u8], what: &str) -> Result<(), SstError> {
    verify_typed_trailer(raw, what, super::block::BLOCK_TYPE_NONE)
}

fn verify_typed_trailer(raw: &[u8], what: &str, kind: u8) -> Result<(), SstError> {
    if raw.len() < 6 || raw[raw.len() - 5] != kind {
        return Err(SstError::Corrupt(format!("unknown {what} block type")));
    }
    let stored_crc = u32::from_le_bytes(raw[raw.len() - 4..].try_into().unwrap());
    if crc32::crc32(&raw[..raw.len() - 4]) != stored_crc {
        return Err(SstError::Corrupt(format!("{what} block checksum mismatch")));
    }
    Ok(())
}

fn parse_index(index_raw: &[u8], data_end: u64) -> Result<Vec<IndexEntry>, SstError> {
    let bad = |m: String| SstError::Corrupt(m);
    verify_trailer(index_raw, "index")?;
    let payload = &index_raw[..index_raw.len() - 5];
    let mut pos = 0usize;
    let read_u32 = |pos: &mut usize| -> Result<u32, SstError> {
        if *pos + 4 > payload.len() {
            return Err(bad("index truncated in header".to_string()));
        }
        let v = u32::from_le_bytes(payload[*pos..*pos + 4].try_into().unwrap());
        *pos += 4;
        Ok(v)
    };
    let count = read_u32(&mut pos)? as usize;
    if count == 0 || count > payload.len().saturating_sub(pos) / 20 {
        return Err(bad("index count exceeds encoded entries".to_string()));
    }
    let mut index = Vec::new();
    index
        .try_reserve(count)
        .map_err(|e| bad(format!("index allocation: {e}")))?;
    let mut prev_separator: Option<&[u8]> = None;
    let mut previous_end = 0;
    for _ in 0..count {
        let sep_len = read_u32(&mut pos)? as usize;
        if pos
            .checked_add(sep_len)
            .and_then(|n| n.checked_add(16))
            .is_none_or(|end| end > payload.len())
        {
            return Err(bad("index entry runs past index block".to_string()));
        }
        let separator = &payload[pos..pos + sep_len];
        pos += sep_len;
        let offset = u64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let len = u64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        if offset != previous_end
            || offset.checked_add(len).is_none_or(|end| end > data_end)
            || len < 14
            || len > crate::limits::MAX_RECORD_BYTES as u64
        {
            return Err(bad("index entry points outside data area".to_string()));
        }
        previous_end = offset + len;
        if let Some(prev) = prev_separator
            && separator <= prev
        {
            return Err(bad("separators not strictly increasing".to_string()));
        }
        prev_separator = Some(separator);
        index.push(IndexEntry {
            separator: Separator::new(separator),
            offset,
            len,
        });
    }
    if pos != payload.len() {
        return Err(bad("index block has trailing garbage".to_string()));
    }
    if previous_end != data_end {
        return Err(bad("data blocks do not exactly cover data area".to_string()));
    }
    Ok(index)
}

pub struct Iter<'a> {
    table: &'a SstTable,
    next_block: usize,
    current: Option<BlockIter>,
    failed: bool,
    lower_bound: Option<Vec<u8>>,
    readahead: ReadAhead,
}

/// Per-iterator read-ahead state; see `SstTable::read_block_sequential`.
/// The pinned window keeps at most one window alive outside the range
/// tier's budget per iterator, like compaction's read-ahead buffers.
#[derive(Default)]
struct ReadAhead {
    /// Size of the next source read (0 = only the requested block).
    window: u64,
    source_reads: u32,
    pinned: Option<(u64, Window)>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<(Kind, u64, Vec<u8>, Vec<u8>), SstError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(state) = &mut self.current {
                match state.next() {
                    Some(Ok((kind, seq, key, value))) => {
                        if let Some(bound) = &self.lower_bound {
                            if key.as_slice() < bound.as_slice() {
                                continue;
                            }
                            self.lower_bound = None;
                        }
                        return Some(Ok((kind, seq, key, value)));
                    }
                    Some(Err(e)) => {
                        self.failed = true;
                        return Some(Err(e));
                    }
                    None => self.current = None,
                }
            }
            if self.next_block >= self.table.index.len() {
                return None;
            }
            match self
                .table
                .read_block_sequential(self.next_block, &mut self.readahead)
            {
                Ok(block) => {
                    let mut iter = BlockIter::from_verified(block);
                    if let Some(bound) = &self.lower_bound
                        && let Err(e) = iter.seek_restart(bound)
                    {
                        self.failed = true;
                        return Some(Err(e));
                    }
                    self.current = Some(iter);
                    self.next_block += 1;
                }
                Err(e) => {
                    self.failed = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

#[cfg(test)]
mod compaction_iter_tests {
    #[test]
    fn malformed_index_counts_and_ranges_are_rejected_before_allocation_or_read() {
        use super::*;
        let mut builder = crate::sstable::TableBuilder::new();
        builder.add(Kind::Put, b"k", b"v", 1).unwrap();
        let table = builder.finish().unwrap();
        let footer = table.len() - FOOTER_LEN;
        let offset = u64::from_le_bytes(table[footer..footer + 8].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(table[footer + 8..footer + 16].try_into().unwrap()) as usize;
        let data_end = u64::from_le_bytes(table[footer + 20..footer + 28].try_into().unwrap());
        let raw = table[offset..offset + len].to_vec();
        for huge_count in [false, true] {
            let mut bad = raw.clone();
            if huge_count {
                bad[..4].copy_from_slice(&u32::MAX.to_le_bytes());
            } else {
                let key_len = u32::from_le_bytes(bad[4..8].try_into().unwrap()) as usize;
                bad[8 + key_len..16 + key_len].copy_from_slice(&u64::MAX.to_le_bytes());
            }
            let n = bad.len();
            let crc = crate::crc32::crc32(&bad[..n - 4]);
            bad[n - 4..].copy_from_slice(&crc.to_le_bytes());
            assert!(parse_index(&bad, data_end).is_err());
        }
    }
    use super::*;
    use crate::sstable::TableBuilder;
    use crate::testutil::TempDir;

    #[test]
    fn overflowed_footer_ranges_return_corruption_before_reading_blocks() {
        let td = TempDir::new("overflowed-footer");
        let mut builder = TableBuilder::new();
        builder.add(Kind::Put, b"k", b"v", 1).unwrap();
        let raw = builder.finish().unwrap();
        for field in [8, 20, 28] {
            let mut bad = raw.clone();
            let at = bad.len() - FOOTER_LEN + field;
            bad[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            let path = td.path().join(format!("{field}.sst"));
            std::fs::write(&path, bad).unwrap();
            assert!(matches!(
                SstTable::open(
                    field as u64,
                    &path,
                    Arc::new(BlockCache::new(0)),
                    Arc::new(TableFileCache::new(1)),
                    false
                ),
                Err(SstError::Corrupt(_))
            ));
        }
    }

    /// Builds a real, on-disk SstTable with `entries` (key, value)
    /// pairs, each key `k{i:06}`. Enough entries at a big-enough value
    /// size spans multiple `TARGET_BLOCK_SIZE` (4096-byte) blocks, so
    /// tests exercise real block boundaries, not a single-block table.
    fn build_table(
        dir: &std::path::Path,
        number: u64,
        entries: usize,
        value_len: usize,
        block_cache: Arc<BlockCache>,
        file_cache: Arc<TableFileCache>,
    ) -> SstTable {
        build_table_with_mmap(
            dir,
            number,
            entries,
            value_len,
            block_cache,
            file_cache,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_table_with_mmap(
        dir: &std::path::Path,
        number: u64,
        entries: usize,
        value_len: usize,
        block_cache: Arc<BlockCache>,
        file_cache: Arc<TableFileCache>,
        use_mmap: bool,
    ) -> SstTable {
        let mut builder = TableBuilder::new();
        for i in 0..entries {
            let key = format!("k{i:06}").into_bytes();
            let value = vec![(i % 256) as u8; value_len];
            builder
                .add(Kind::Put, &key, &value, (i + 1) as u64)
                .unwrap();
        }
        let bytes = builder.finish().unwrap();
        let path = dir.join(format!("{number}.sst"));
        crate::atomic::commit_file(&path, &bytes).unwrap();
        SstTable::open(number, &path, block_cache, file_cache, use_mmap).unwrap()
    }

    /// The core correctness property: compaction_iter must yield
    /// EXACTLY the same sequence of entries as the foreground
    /// iter_from(b"") path, for a table spanning many blocks.
    #[test]
    fn compaction_iter_matches_foreground_iteration() {
        let td = TempDir::new("compaction-iter-match");
        let block_cache = Arc::new(BlockCache::new(1024 * 1024));
        let file_cache = Arc::new(TableFileCache::new(8));
        let table = build_table(td.path(), 1, 300, 64, block_cache, file_cache);

        let foreground: Vec<_> = table.iter_from(b"").map(|r| r.unwrap()).collect();
        assert!(foreground.len() >= 300);

        let compaction: Vec<_> = table
            .compaction_iter()
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(
            foreground, compaction,
            "compaction_iter must yield the same entries, in the same order, as the foreground path"
        );
    }

    #[test]
    fn compaction_iter_uses_the_simulated_device_and_bypasses_caches() {
        struct DeviceGuard;
        impl Drop for DeviceGuard {
            fn drop(&mut self) {
                crate::sys::disable_device_sim();
            }
        }
        let td = TempDir::new("simulated-compaction-reader");
        crate::sys::enable_device_sim();
        let _device = DeviceGuard;
        let cache = Arc::new(BlockCache::new(1024 * 1024));
        let files = Arc::new(TableFileCache::new(8));
        let table = build_table(td.path(), 1, 300, 64, cache.clone(), files.clone());
        assert!(
            !td.path().join("1.sst").exists(),
            "fixture must exist only in the simulated namespace"
        );
        let expected: Vec<_> = table.iter().map(Result::unwrap).collect();
        let before_cache = cache.stats();
        let before_files = files.stats();
        let got: Vec<_> = table
            .compaction_iter_with_window(1)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(got, expected);
        assert_eq!(cache.stats().hits, before_cache.hits);
        assert_eq!(cache.stats().misses, before_cache.misses);
        assert_eq!(files.stats().hits, before_files.hits);
        assert_eq!(files.stats().misses, before_files.misses);
    }

    /// Same equivalence property, but with a read-ahead window small
    /// enough to force a refill on almost every single block — proves
    /// the multi-refill path (not just the "whole table in one
    /// syscall" common case) parses correctly at block boundaries.
    #[test]
    fn compaction_iter_matches_with_tiny_read_ahead_window() {
        let td = TempDir::new("compaction-iter-tiny-window");
        let block_cache = Arc::new(BlockCache::new(1024 * 1024));
        let file_cache = Arc::new(TableFileCache::new(8));
        let table = build_table(td.path(), 1, 300, 64, block_cache, file_cache);

        let foreground: Vec<_> = table.iter_from(b"").map(|r| r.unwrap()).collect();

        // 1-byte window: every block is smaller than the window, so
        // every single block forces its own refill (the "at least
        // this one block" guarantee is what keeps this correct rather
        // than looping forever).
        let compaction: Vec<_> = table
            .compaction_iter_with_window(1)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(foreground, compaction);
    }

    /// The whole point of 11.17-C: a compaction scan must not touch
    /// the foreground block cache at all, in either direction — not a
    /// hit, not a miss, not an insertion.
    #[test]
    fn compaction_iter_never_touches_block_cache() {
        let td = TempDir::new("compaction-iter-no-cache");
        let block_cache = Arc::new(BlockCache::new(1024 * 1024));
        let file_cache = Arc::new(TableFileCache::new(8));
        let table = build_table(
            td.path(),
            1,
            300,
            64,
            block_cache.clone(),
            file_cache.clone(),
        );

        // `SstTable::open` itself already cached the first/last
        // boundary blocks (block-cache.md D1) — that's unrelated to
        // compaction_iter, so the property under test is "unchanged by
        // the scan", not "zero", and it's asserted only after opening.
        let before = block_cache.stats();
        assert!(
            before.resident_entries > 0,
            "sanity: table open should have cached its boundary blocks"
        );
        let count = table.compaction_iter().unwrap().count();
        assert!(count >= 300);
        let after = block_cache.stats();

        assert_eq!(before.hits, after.hits);
        assert_eq!(before.misses, after.misses);
        assert_eq!(
            before.resident_entries, after.resident_entries,
            "compaction_iter must not insert anything into the block cache"
        );
    }

    /// A compaction scan must not lease from the shared, bounded
    /// TableFileCache either — it opens its own handle. Proven here by
    /// shrinking the file-cache to zero effective capacity for foreign
    /// leases (capacity 1, already held by a concurrent foreground
    /// lease) and showing compaction still completes.
    #[test]
    fn compaction_iter_does_not_contend_the_shared_file_cache() {
        let td = TempDir::new("compaction-iter-no-fd-contention");
        let block_cache = Arc::new(BlockCache::new(1024 * 1024));
        let file_cache = Arc::new(TableFileCache::new(1));
        let table = build_table(
            td.path(),
            1,
            50,
            64,
            block_cache.clone(),
            file_cache.clone(),
        );

        // Hold the file cache's one slot with a foreground-style lease
        // for the table's own path for the whole compaction scan.
        let lease = file_cache.acquire(table.number(), &table.path).unwrap();
        let count = table.compaction_iter().unwrap().count();
        assert!(
            count >= 50,
            "compaction must complete without the shared file-cache lease"
        );
        drop(lease);
    }
}

#[cfg(test)]
mod mmap_tests {
    use super::compaction_iter_tests::build_table_with_mmap;
    use super::*;
    use crate::testutil::TempDir;

    /// The core correctness property: an mmap-backed table must yield
    /// byte-for-byte the same entries, in the same order, as the same
    /// table opened the normal (file-cache-leased pread) way.
    #[test]
    fn mmap_reads_match_pread_reads() {
        let td = TempDir::new("mmap-match");
        let pread_table = build_table_with_mmap(
            td.path(),
            1,
            400,
            96,
            Arc::new(BlockCache::new(1024 * 1024)),
            Arc::new(TableFileCache::new(8)),
            false,
        );
        let mmap_table = build_table_with_mmap(
            td.path(),
            1,
            400,
            96,
            Arc::new(BlockCache::new(1024 * 1024)),
            Arc::new(TableFileCache::new(8)),
            true,
        );

        let pread_entries: Vec<_> = pread_table.iter_from(b"").map(|r| r.unwrap()).collect();
        let mmap_entries: Vec<_> = mmap_table.iter_from(b"").map(|r| r.unwrap()).collect();
        assert!(!pread_entries.is_empty());
        assert_eq!(pread_entries, mmap_entries);

        // Point lookups too, not just full iteration.
        for i in [0usize, 137, 399] {
            let key = format!("k{i:06}");
            assert_eq!(
                pread_table
                    .get(key.as_bytes(), None)
                    .unwrap()
                    .map(|f| f.value),
                mmap_table
                    .get(key.as_bytes(), None)
                    .unwrap()
                    .map(|f| f.value),
            );
        }
    }

    /// POSIX guarantee this whole design leans on: unlinking the file
    /// backing a live mapping must not invalidate it. Simulates what a
    /// later compaction's `reclaim_obsolete` does to a retired table
    /// while this table handle (and its mapping) is still alive and in
    /// use.
    #[test]
    fn mmap_survives_the_backing_file_being_unlinked() {
        let td = TempDir::new("mmap-unlink");
        let table = build_table_with_mmap(
            td.path(),
            1,
            200,
            64,
            Arc::new(BlockCache::new(1024 * 1024)),
            Arc::new(TableFileCache::new(8)),
            true,
        );
        let before: Vec<_> = table.iter_from(b"").map(|r| r.unwrap()).collect();
        assert!(!before.is_empty());

        std::fs::remove_file(td.path().join("1.sst")).unwrap();

        // The path itself is gone; the mapping must not care.
        assert!(!td.path().join("1.sst").exists());
        let after: Vec<_> = table.iter_from(b"").map(|r| r.unwrap()).collect();
        assert_eq!(before, after);
        for i in [0usize, 55, 199] {
            let key = format!("k{i:06}");
            assert!(table.get(key.as_bytes(), None).unwrap().is_some());
        }
    }

    /// Many threads reading through the SAME mapping concurrently:
    /// proves `MappedFile`'s `Send + Sync` is actually sound in
    /// practice, not just type-checked.
    #[test]
    fn mmap_reads_are_safe_under_concurrent_threads() {
        let td = TempDir::new("mmap-concurrent");
        let table = Arc::new(build_table_with_mmap(
            td.path(),
            1,
            500,
            64,
            Arc::new(BlockCache::new(1024 * 1024)),
            Arc::new(TableFileCache::new(8)),
            true,
        ));
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let table = table.clone();
                std::thread::spawn(move || {
                    for round in 0..200 {
                        let i = (t * 137 + round * 31) % 500;
                        let key = format!("k{i:06}");
                        let found = table.get(key.as_bytes(), None).unwrap();
                        assert!(found.is_some(), "key {key} must be found");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
}

#[cfg(test)]
mod compact_point_tests {
    use super::*;
    use crate::sstable::TableBuilder;
    use crate::testutil::TempDir;
    use std::os::unix::fs::FileExt;

    fn fixture(path: &Path, mapped: bool) -> SstTable {
        fixture_with(path, mapped, TableBuilder::with_point_index(true))
    }

    fn fixture_with(path: &Path, mapped: bool, mut builder: TableBuilder) -> SstTable {
        builder.add(Kind::Put, b"", b"empty", 1).unwrap();
        for id in 0..97 {
            let key = [b"\0prefix/".as_slice(), &(id as u64).to_be_bytes(), b"\xff"].concat();
            for seq in (1..=40).rev() {
                let (kind, value) = if seq == 40 {
                    (Kind::Tombstone, Vec::new())
                } else {
                    (Kind::Put, vec![(id + seq) as u8; 200])
                };
                builder.add(kind, &key, &value, seq as u64).unwrap();
            }
        }
        let file = path.join("1.sst");
        crate::atomic::commit_file(&file, &builder.finish().unwrap()).unwrap();
        let cache = Arc::new(BlockCache::new(256 * 1024));
        cache.set_point_working_set(u64::MAX);
        let table =
            SstTable::open(1, &file, cache, Arc::new(TableFileCache::new(2)), mapped).unwrap();
        assert!(table.locators.is_some());
        assert!(table.point.is_none());
        table
    }

    #[test]
    fn compact_locators_preserve_versions_full_keys_and_cache_budget() {
        for mapped in [false, true] {
            let dir = TempDir::new("compact-versions");
            let table = fixture(dir.path(), mapped);
            assert_eq!(table.get(b"", None).unwrap().unwrap().value, b"empty");
            for id in 0..97u64 {
                let key = [b"\0prefix/".as_slice(), &id.to_be_bytes(), b"\xff"].concat();
                assert_eq!(
                    table.get(&key, None).unwrap().unwrap().kind,
                    Kind::Tombstone
                );
                for seq in [1, 17, 39] {
                    let found = table.get(&key, Some(seq)).unwrap().unwrap();
                    assert_eq!(found.seq, seq);
                    assert_eq!(found.value, vec![(id + seq) as u8; 200]);
                }
                assert!(table.get(&key, Some(0)).unwrap().is_none());
            }
            for id in 0..100_000u64 {
                assert!(table.get(&id.to_be_bytes(), None).unwrap().is_none());
            }
            assert!(table.block_cache.stats().point_index_bytes > 0);
            assert!(table.block_cache.resident_bytes() <= table.block_cache.capacity());
            let before = table.block_cache.stats();
            table.get(b"", None).unwrap();
            assert_eq!(before.misses, table.block_cache.stats().misses);
            let cache = table.block_cache.clone();
            drop(table);
            assert_eq!(cache.stats().point_index_bytes, 0);
        }
    }

    #[test]
    fn compact_values_recheck_corruption_after_open_for_pread_and_mapping() {
        for mapped in [false, true] {
            let dir = TempDir::new("compact-value-corruption");
            let table = fixture(dir.path(), mapped);
            let locator = table.locators.as_ref().unwrap().locate(b"", None).unwrap();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(dir.path().join("1.sst"))
                .unwrap();
            file.write_all_at(b"X", locator.offset as u64).unwrap();
            file.sync_all().unwrap();
            for _ in 0..2 {
                assert!(matches!(table.get(b"", None), Err(SstError::Corrupt(_))));
            }
        }
    }

    #[test]
    fn compact_pread_reports_short_reads_as_corruption() {
        let dir = TempDir::new("compact-short-read");
        let table = fixture(dir.path(), false);
        let locator = table.locators.as_ref().unwrap().locate(b"", None).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join("1.sst"))
            .unwrap()
            .set_len(locator.offset as u64 + 1)
            .unwrap();
        assert!(matches!(table.get(b"", None), Err(SstError::Corrupt(_))));
    }

    #[test]
    fn compact_open_rejects_corrupt_sorted_bytes_before_publishing_locators() {
        let dir = TempDir::new("compact-open-corruption");
        let table = fixture(dir.path(), false);
        let offset = table
            .locators
            .as_ref()
            .unwrap()
            .locate(b"", None)
            .unwrap()
            .offset;
        drop(table);
        let path = dir.path().join("1.sst");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .write_all_at(b"X", offset as u64)
            .unwrap();
        for mapped in [false, true] {
            let cache = Arc::new(BlockCache::new(256 * 1024));
            cache.set_point_working_set(u64::MAX);
            assert!(matches!(
                SstTable::open(1, &path, cache, Arc::new(TableFileCache::new(2)), mapped),
                Err(SstError::Corrupt(_))
            ));
        }
    }

    #[test]
    fn compact_mapping_survives_unlink_with_concurrent_reads() {
        let dir = TempDir::new("compact-unlink-concurrent");
        let table = fixture(dir.path(), true);
        assert!(table.mmap.is_some());
        std::fs::remove_file(dir.path().join("1.sst")).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let table = &table;
                scope.spawn(move || {
                    for _ in 0..1000 {
                        assert_eq!(table.get(b"", None).unwrap().unwrap().value, b"empty");
                    }
                });
            }
        });
    }

    #[test]
    fn locators_admit_tables_with_scan_extension_and_still_check_values() {
        for mapped in [false, true] {
            let dir = TempDir::new("compact-scan-flags");
            let table = fixture_with(dir.path(), mapped, TableBuilder::with_scan_index(true, 4));
            // Locator mode drops the scan directory; point reads stay verified.
            assert!(table.scan_index().is_none());
            let plain = TempDir::new("compact-plain-flags");
            let other = fixture(plain.path(), mapped);
            let charge = |t: &SstTable| t.block_cache.stats().point_index_bytes;
            assert_eq!(charge(&table), charge(&other));
            for id in 0..97u64 {
                let key = [b"\0prefix/".as_slice(), &id.to_be_bytes(), b"\xff"].concat();
                assert_eq!(
                    table.get(&key, Some(17)).unwrap().unwrap().value,
                    vec![(id + 17) as u8; 200]
                );
            }
            let locator = table.locators.as_ref().unwrap().locate(b"", None).unwrap();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(dir.path().join("1.sst"))
                .unwrap();
            file.write_all_at(b"X", locator.offset as u64).unwrap();
            file.sync_all().unwrap();
            assert!(matches!(table.get(b"", None), Err(SstError::Corrupt(_))));
        }
    }
}

/// Every {Bloom, fuse} x {no point index, PH2, PH3 flag zero, bit 0, bits 0+1}
/// table must round-trip through each read path and refuse damaged filters.
#[cfg(test)]
mod filter_matrix_tests {
    use super::*;
    use crate::sstable::TableBuilder;
    use crate::testutil::TempDir;
    use std::collections::BTreeMap;

    type Variant = (&'static str, fn() -> TableBuilder, bool);
    type Model = BTreeMap<Vec<u8>, Vec<(u64, Kind, Vec<u8>)>>;

    const VARIANTS: [Variant; 5] = [
        ("no-point", TableBuilder::new, false),
        ("ph2", TableBuilder::with_legacy_point_index, true),
        (
            "ph3-flag0",
            TableBuilder::with_point_index_without_value_crc,
            true,
        ),
        ("ph3-bit0", || TableBuilder::with_point_index(true), true),
        (
            "ph3-bit0+bit1",
            || TableBuilder::with_scan_index(true, 2),
            true,
        ),
    ];

    fn key(id: u64) -> Vec<u8> {
        if id == 0 {
            Vec::new()
        } else {
            [b"\0prefix/".as_slice(), &id.to_be_bytes(), b"\xff"].concat()
        }
    }

    /// Present ids are even; odd ids and ids past the end are provably absent.
    fn model(versions: u64, width: usize) -> Model {
        (0..97u64)
            .map(|n| {
                let id = n * 2;
                let rows = (1..=versions)
                    .rev()
                    .map(|seq| {
                        if seq == versions && id % 3 == 0 {
                            (seq, Kind::Tombstone, Vec::new())
                        } else {
                            (seq, Kind::Put, vec![(id + seq) as u8; width])
                        }
                    })
                    .collect();
                (key(id), rows)
            })
            .collect()
    }

    fn build(make: fn() -> TableBuilder, fuse: bool, model: &Model) -> Vec<u8> {
        let mut builder = make().with_filter(fuse);
        for (key, rows) in model {
            for (seq, kind, value) in rows {
                builder.add(*kind, key, value, *seq).unwrap();
            }
        }
        builder.finish().unwrap()
    }

    fn version_of(bytes: &[u8]) -> u32 {
        let footer = bytes.len() - FOOTER_LEN;
        u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap())
    }

    fn open(path: &Path, capacity: usize, compact: bool, mapped: bool) -> SstTable {
        let cache = Arc::new(BlockCache::new(capacity));
        if compact {
            cache.set_point_working_set(u64::MAX);
        }
        SstTable::open(1, path, cache, Arc::new(TableFileCache::new(2)), mapped).unwrap()
    }

    #[test]
    fn every_filter_and_point_combination_round_trips_on_every_read_path() {
        let data = model(12, 100);
        let dir = TempDir::new("filter-matrix");
        let path = dir.path().join("1.sst");
        for (name, make, point) in VARIANTS {
            for fuse in [false, true] {
                let bytes = build(make, fuse, &data);
                let expected_version = match (fuse, point) {
                    (false, false) => FORMAT_VERSION,
                    (false, true) => POINT_FORMAT_VERSION,
                    (true, false) => FILTER_FORMAT_VERSION,
                    (true, true) => FILTER_POINT_FORMAT_VERSION,
                };
                assert_eq!(version_of(&bytes), expected_version, "{name} fuse={fuse}");
                crate::atomic::commit_file(&path, &bytes).unwrap();
                // (capacity, compact locators): no admission, locators, resident.
                for (capacity, compact) in [(0, false), (64 * 1024, true), (64 << 20, false)] {
                    for mapped in [false, true] {
                        let table = open(&path, capacity, compact, mapped);
                        let context = format!("{name} fuse={fuse} cap={capacity} mapped={mapped}");
                        // Locators need the per-value CRC flag; older point
                        // sections stay unadmitted under a compact policy.
                        let locators = point && compact && name.starts_with("ph3-bit0");
                        let resident = point && capacity > 0 && !compact;
                        assert_eq!(table.locators.is_some(), locators, "{context}");
                        assert_eq!(table.point.is_some(), resident, "{context}");
                        assert_eq!(
                            matches!(table.filter, SstFilter::Fuse(_)),
                            fuse,
                            "{context}"
                        );
                        for (key, rows) in &data {
                            for limit in [None, Some(0), Some(3), Some(11), Some(12)] {
                                let want = rows
                                    .iter()
                                    .find(|(seq, ..)| limit.is_none_or(|l| *seq <= l));
                                let got = table.get(key, limit).unwrap();
                                assert_eq!(
                                    got.as_ref().map(|f| (f.seq, f.kind, f.value.clone())),
                                    want.cloned(),
                                    "{context} {key:?} {limit:?}"
                                );
                            }
                        }
                        for id in (0..300u64).filter(|id| id % 2 == 1 || *id > 200) {
                            assert!(
                                table.get(&key(id), None).unwrap().is_none(),
                                "{context} absent {id}"
                            );
                        }
                        for stray in [&b"\0"[..], b"zzz", b"\0prefix", b"\xff\xff"] {
                            assert!(table.get(stray, None).unwrap().is_none(), "{context}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn damaged_filter_blocks_and_rewritten_versions_are_rejected_for_every_combination() {
        let data = model(3, 8);
        let dir = TempDir::new("filter-matrix-corrupt");
        let path = dir.path().join("1.sst");
        let attempt = |bytes: &[u8], capacity: usize| {
            std::fs::write(&path, bytes).unwrap();
            SstTable::open(
                1,
                &path,
                Arc::new(BlockCache::new(capacity)),
                Arc::new(TableFileCache::new(2)),
                false,
            )
            .map(|_| ())
        };
        for (name, make, _) in VARIANTS {
            for fuse in [false, true] {
                let good = build(make, fuse, &data);
                assert!(attempt(&good, 0).is_ok() && attempt(&good, 1 << 20).is_ok());
                let footer = good.len() - FOOTER_LEN;
                let at = |lo: usize| {
                    u64::from_le_bytes(good[footer + lo..footer + lo + 8].try_into().unwrap())
                        as usize
                };
                let (offset, len) = (at(20), at(28));
                for capacity in [0, 1 << 20] {
                    for byte in offset..offset + len {
                        for bit in 0..8 {
                            let mut bad = good.clone();
                            bad[byte] ^= 1 << bit;
                            assert!(
                                matches!(attempt(&bad, capacity), Err(SstError::Corrupt(_))),
                                "{name} fuse={fuse} cap={capacity} byte={byte} bit={bit}"
                            );
                        }
                    }
                }
                // A foreign footer version must not parse the filter as another kind.
                for version in [
                    FORMAT_VERSION,
                    POINT_FORMAT_VERSION,
                    FILTER_FORMAT_VERSION,
                    FILTER_POINT_FORMAT_VERSION,
                    7,
                ] {
                    if version == version_of(&good) {
                        continue;
                    }
                    let mut bad = good.clone();
                    bad[footer + 16..footer + 20].copy_from_slice(&version.to_le_bytes());
                    assert!(
                        attempt(&bad, 1 << 20).is_err(),
                        "{name} fuse={fuse} accepted version {version}"
                    );
                }
            }
        }
    }
}
