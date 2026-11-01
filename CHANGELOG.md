# Changelog

## Unreleased

## 0.2.0 - 2026-10-31

- **Behavior change:** `KibanOptions::point_index` now defaults to `true`. New SSTs carry
  the perfect-hash point index (format 4) and cached point reads are faster than LMDB.
  Costs, measured in `validation/pointindex-cost`: flush builds about 1.36x and
  compaction builds about 1.26x slower, about 9% more SST bytes, and index memory shared
  with `block_cache_bytes`. Existing databases keep working (readers accept both formats;
  tables are rewritten as compaction touches them). **Format-4 SSTs cannot be read by
  binaries older than the point-index release**: set `point_index: false` and rewrite
  (compact or backup) before downgrading. Validated by the crash-simulation suites at
  20k seeds on this change and a 9M-seed-per-suite sweep on the preceding commit
  (180 of 200 chunks, 0 failures; 20 chunks were interrupted by a system suspend).
- `KibanOptions::agent_memory(prefix_len)`: one-call profile enabling the point index and
  prefix scan extension for agent-memory workloads.
- Group commit hardening: a memtable freeze that fails after a group has committed no
  longer reports an error for writes that are already durable and visible. The group is
  acknowledged and the failure becomes the sticky maintenance error, so the next write
  fails fast instead of a retry writing twice. Leadership hand-off and follower delivery
  are panic-safe: followers that were not yet delivered are failed instead of left
  waiting. New tests cover a failing WAL append, same-key arrival order within a group,
  failed-sync sequence handling, and a panic between hand-off and delivery.
- Atomic shared batch visibility and lifetime-held database ownership locks.
- Checksummed, versioned MANIFEST envelope with legacy decoding; strict recovery
  identity/range checks and bounded, checked batch/SST/frame decoding.
- Typed mutation errors and sequence exhaustion without wrapping or poisoning.
- Bounded hot-key memtables and write-volume WAL rotation, RAII snapshot identities,
  constant-space governor averages and retryable obsolete-file reclamation with pending-byte counters.
- Snapshot file pinning replaces conservative output-version retention; maximum
  sequences cannot collide with compaction sentinels, and direct clones share memory.
- Compaction refill descriptors independent of fan-in; hashed filter construction
  and streaming memtable/shared-snapshot iteration.
- Fallible checkpoint/close, logical backup/restore and offline integrity CLI.
- Audit regressions, parser mutation corpus, release progress stress, expanded CI,
  Rust 1.89 minimum-version gate and concurrent usage example.
- Read path: per-level key index for point lookups, one lazily advancing scan
  source per disjoint level, `range()` table pruning, allocation-free restart
  search and restart seeking (short scans +79–81%, full scans +27% in
  `validation/RANGE_CACHE.md`).
- Optional raw SST range tier beneath the block cache, ported from HelixDB's
  `range-cache` (`range_cache_bytes`, default 0 = disabled; `range_readahead_bytes`)
  with `KibanStats::range_cache` counters, and a `read_paths` benchmark.
- WAL recovery truncates a zero-filled tail as torn instead of refusing to open;
  WAL file numbers are reserved before creation and the allocator never moves backwards.

New MANIFESTs cannot be read by older Kiban builds. Existing unversioned MANIFESTs
remain readable. Mutation methods now consistently return `DbError`; conversion
to `std::io::Error` preserves the original error as its source. New size limits
reject old oversized records rather than silently accepting unbounded input.

## 0.1.0

First public release of Kiban, an embedded LSM-tree key-value engine for Linux.

- WAL-backed explicit durability and crash recovery.
- Mutable and immutable memtables, SSTables, and leveled compaction with an
  adaptive governor and optional static schedulers.
- Point reads, ordered scans/iterators, tombstones, multi-version snapshots,
  and atomic WAL write batches.
- Shared concurrent handles with background flush and compaction.
- Bloom filters, block caching, bounded SST descriptor caching, and optional mmap.
- Maintenance and read-amplification metrics, deterministic fault injection,
  volatile-device recovery testing, process-kill torture, and scale workloads.
- Correctness repairs for frozen-WAL sync, captured snapshot sources, point-read
  sequence resolution, direct-flush installation failures, and SST lifetime
  across metadata-only compaction moves.

This is an early release. API and storage-format compatibility are not promised
before 1.0. Compression, range deletes, reverse iterators, and transactions are
not implemented. See validation/CORRECTNESS.md for evidence and model limits.
