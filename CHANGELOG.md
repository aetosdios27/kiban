# Changelog

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
