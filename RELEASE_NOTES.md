# Kiban v0.2.0

Kiban is an embedded LSM-tree key-value storage engine written in Rust for Linux. This release brings faster cached reads, group commit and the audit repairs made since v0.1.0. It is still an early release: API and storage-format compatibility are not promised before 1.0, and these results do not establish production readiness.

## Highlights

- **Perfect-hash point index, on by default.** Cached point reads reach a median of 1.09 µs on the normal 1M-record layout: 2.6× faster than tuned RocksDB and 1.3× faster than LMDB in the same campaign.
- **Group commit.** Durable writers share a sync. At 16 writers, kiban reaches 14,101 synced writes per second with 480 syncs, against 1,836 per second and 4,000 syncs without grouping. RocksDB reaches 12,830.
- **`KibanOptions::agent_memory(prefix_len)`.** One call enables the point index and the prefix scan extension for agent-memory workloads.
- **Read path rework.** A per-level key index, lazily advancing scan sources and restart seeking, plus an optional range tier ported from HelixDB.
- **Hardening.** Atomic shared batch visibility, ownership locks, a checksummed MANIFEST envelope, strict recovery checks, bounded decoding, logical backup and restore, and an offline integrity CLI.
- **Python bindings and an MCP server**, installed from this repository.

## Where kiban loses

- Peak memory is 450 MiB on the 1M-record campaign, against 356 for RocksDB and 349 for SQLite.
- Disk use is 352 MiB for 1M records, against 304 and 315.
- RocksDB finishes the long sustained-write runs about 3× sooner.
- At 32 MiB of cache, LMDB is 1.4× faster on latest-ten reads, and on point reads unless mmap is on.
- The point index halves 100M reads at a 33 MiB cache (1.7M ops/s, 3.8M with it off) and makes flush and compaction 1.36× and 1.26× slower.

The campaign ran on one shared laptop with warm caches. It says nothing about larger-than-memory or cold-storage performance. Raw results are in `validation/site-refresh`.

## Upgrading

- **Format-4 SSTs cannot be read by older binaries.** New tables carry the point index, and existing tables are rewritten as compaction touches them. To downgrade, set `point_index: false` and rewrite (compact or back up) first.
- New MANIFESTs cannot be read by older builds. Existing unversioned MANIFESTs stay readable and upgrade at their next install.
- Mutation methods now return `DbError`. Converting to `std::io::Error` keeps the original error as its source.
- New size limits reject old oversized records.

See `CHANGELOG.md` for the full list.

## Installation

```toml
[dependencies]
kiban = { git = "https://github.com/aetosdios27/kiban", tag = "v0.2.0" }
```

Registry installation with `cargo add kiban` becomes available only after crates.io publication succeeds.
