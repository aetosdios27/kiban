//! Table reader: lazy table handles with cached block reads.
//!
//! Per `docs/design/block-cache.md` D1: opening reads only footer,
//! index, and bloom filter; data blocks load on demand through the
//! shared LRU. Corruption is detected and reported, never repaired.
//!
//! Per phase 11.6: an `SstTable` does not permanently own an open file
//! descriptor. Every read leases one from the shared `TableFileCache`
//! for just the duration of that read — see `read_block`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::block::{BlockIter, VerifiedBlock};
use super::builder::{FOOTER_LEN, FORMAT_VERSION, MAGIC};
use super::{Kind, SstError};
use crate::bloom::BloomFilter;
use crate::cache::{BlockCache, CachedBlock};
use crate::crc32;
use crate::file_cache::TableFileCache;

struct IndexEntry {
    separator: Vec<u8>,
    offset: u64,
    len: u64,
}

pub struct SstTable {
    // Debug is implemented manually below: neither the file cache nor
    // the block cache derive it, and their contents aren't useful here
    // anyway.
    number: u64,
    path: PathBuf,
    file_len: u64,
    index: Vec<IndexEntry>,
    filter: BloomFilter,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    block_cache: Arc<BlockCache>,
    file_cache: Arc<TableFileCache>,
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
        if version != FORMAT_VERSION {
            return Err(bad(format!("unsupported format version {version}")));
        }
        let index_offset = u64::from_le_bytes(footer[0..8].try_into().unwrap());
        let index_len = u64::from_le_bytes(footer[8..16].try_into().unwrap());
        let filter_offset = u64::from_le_bytes(footer[20..28].try_into().unwrap());
        let filter_len = u64::from_le_bytes(footer[28..36].try_into().unwrap());
        let data_end = file_len - FOOTER_LEN as u64;
        if index_offset > data_end || index_len == 0 || index_offset + index_len > data_end {
            return Err(bad("index block out of file bounds".to_string()));
        }
        if filter_offset + filter_len != index_offset || filter_len < 10 {
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
        verify_trailer(&filter_raw, "filter")?;
        let filter = BloomFilter::decode(&filter_raw[..filter_raw.len() - 5])
            .ok_or_else(|| bad("filter block payload is malformed".to_string()))?;
        let index = parse_index(&index_raw, data_end)?;

        let mut table = SstTable {
            number,
            path: path.to_path_buf(),
            file_len,
            index,
            filter,
            first_key: Vec::new(),
            last_key: Vec::new(),
            block_cache,
            file_cache,
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

    /// A block-cache hit needs no file descriptor at all — memory hit
    /// means memory hit. Only a miss leases `file_cache`, and only for
    /// the duration of the positioned read itself (11.6).
    ///
    /// Concurrent misses on the SAME block (11.17-F: multiple readers
    /// racing the same not-yet-cached key) are coalesced through
    /// `BlockCache::get_or_load`: only the first caller actually leases
    /// a file descriptor and reads; the rest wait for its result.
    fn read_block(&self, entry: &IndexEntry) -> Result<VerifiedBlock, SstError> {
        let key = (self.number, entry.offset);
        let number = self.number;
        let path = &self.path;
        let file_cache = &self.file_cache;
        let cached = self
            .block_cache
            .get_or_load(key, || -> Result<CachedBlock, SstError> {
                let data = {
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

    /// Point lookup. Returns the newest version of `key` whose sequence
    /// number does not exceed `limit`, or `None` when provably absent.
    pub fn get(&self, key: &[u8], limit: Option<u64>) -> Result<Option<Found>, SstError> {
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
    /// scan. `compaction_iter` opens its own OWNED file handle up front
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
        let file = std::fs::File::open(&self.path).map_err(|e| {
            SstError::Corrupt(format!(
                "table {} cannot be opened for compaction: {e}",
                self.number
            ))
        })?;
        Ok(CompactionIter {
            table: self,
            file,
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

pub(crate) struct CompactionIter<'a> {
    table: &'a SstTable,
    file: std::fs::File,
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
            use std::os::unix::fs::FileExt;
            self.file.read_exact_at(&mut buf, start).map_err(|e| {
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

fn verify_trailer(raw: &[u8], what: &str) -> Result<(), SstError> {
    if raw.len() < 6 || raw[raw.len() - 5] != super::block::BLOCK_TYPE_NONE {
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
    let mut index = Vec::with_capacity(count);
    let mut prev_separator: Option<Vec<u8>> = None;
    for _ in 0..count {
        let sep_len = read_u32(&mut pos)? as usize;
        if pos + sep_len + 16 > payload.len() {
            return Err(bad("index entry runs past index block".to_string()));
        }
        let separator = payload[pos..pos + sep_len].to_vec();
        pos += sep_len;
        let offset = u64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let len = u64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        if offset + len > data_end || len < 14 {
            return Err(bad("index entry points outside data area".to_string()));
        }
        if let Some(prev) = &prev_separator
            && separator <= *prev
        {
            return Err(bad("separators not strictly increasing".to_string()));
        }
        prev_separator = Some(separator.clone());
        index.push(IndexEntry {
            separator,
            offset,
            len,
        });
    }
    if pos != payload.len() {
        return Err(bad("index block has trailing garbage".to_string()));
    }
    Ok(index)
}

pub struct Iter<'a> {
    table: &'a SstTable,
    next_block: usize,
    current: Option<BlockIter>,
    failed: bool,
    lower_bound: Option<Vec<u8>>,
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
            match self.table.read_block(&self.table.index[self.next_block]) {
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

#[cfg(test)]
mod compaction_iter_tests {
    use super::*;
    use crate::sstable::TableBuilder;
    use crate::testutil::TempDir;

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
        SstTable::open(number, &path, block_cache, file_cache).unwrap()
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
