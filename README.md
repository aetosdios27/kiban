# Kiban

Kiban is an embedded LSM-tree key-value storage engine written in Rust. It stores
ordered byte keys and byte values and targets Linux. **v0.1.0 is an early public
release**, with no pre-1.0 API or storage-format compatibility promise.

See [architecture](ARCHITECTURE.md) for the current invariants and [unreleased
changes](CHANGELOG.md) for audit repairs on `main` that are newer than v0.1.0.

## Installation

These are the release installation commands. Use the Git dependency after the
`v0.1.0` tag is pushed:

```toml
[dependencies]
kiban = { git = "https://github.com/aetosdios27/kiban", tag = "v0.1.0" }
```

Crates.io installation becomes available only after publication succeeds. Check
the [crate listing](https://crates.io/crates/kiban) for publication status before
running:

```sh
cargo add kiban
```

Until the tag/publication is available, build from a local checkout. Runtime
builds have no non-optional dependencies; Loom is optional testing support.

## Usage

```rust
use kiban::db::Kiban;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut db = Kiban::open("example-db")?;
    db.put(b"hello", b"world")?;
    db.sync()?;
    assert_eq!(db.get(b"hello")?, Some(b"world".to_vec()));
    drop(db);

    let reopened = Kiban::open("example-db")?;
    assert_eq!(reopened.get(b"hello")?, Some(b"world".to_vec()));
    Ok(())
}
```

Run the same workflow with `cargo run --example basic -- /path/to/example-db`.
`open` creates or recovers the directory. `delete(key)` writes a tombstone;
call `sync()` to make it durable. `iter()` yields ordered live key/value pairs.
A missing or deleted key returns `None`.

For a consistent logical view, use `let snapshot = db.snapshot()`,
`db.get_at(&snapshot, key)` and `db.scan_at(&snapshot)`. Later writes, flushes and
compactions do not change the captured view. Dropping the last snapshot clone releases bookkeeping automatically.
`db.release_snapshot(&snapshot)` remains an optional, idempotent early release.
Snapshots keep directory ownership while their captured files are needed. `SharedKiban` offers clonable concurrent handles to one
engine with background flush/compaction; its snapshot exposes `get` and `scan`.
One engine owner per directory is enforced with an OS lock; a second independent
open returns `DbError::Locked`. Shared clones belong to one owner.

For agent-memory style workloads (many small records, keys that begin with a fixed-length
agent prefix, reads by id or "the latest N for one agent"), start from
`KibanOptions::agent_memory(prefix_len)`. It turns on the per-SST perfect-hash point index and
the prefix scan extension, which is the configuration measured in the head-to-head reports
(`validation/{sqlite,rocksdb,remix-engine}`). It costs about 9% more SST bytes and slower
flush/compaction builds. `mmap_max_level` and `binary_fuse_filter` stay separate choices:
the first helps caches far smaller than the data but worsens cold-read tails, the second
speeds misses but slows hits while a point index is resident.

## Supported operations

- WAL-backed puts, deletes and atomic WAL write batches with explicit sync.
- Mutable/frozen memtables, SSTables and leveled compaction with an adaptive
  governor by default; optional fixed-priority and scored schedulers.
- Point reads, scans/iterators, tombstones and sequence-bounded snapshots.
- Recovery from WAL/MANIFEST state; poisoning after unsafe WAL/commit failures.
- Bloom filters, block cache, optional range read-ahead tier, bounded SST descriptor cache and optional mmap.
- Maintenance/read-amplification metrics and reproducible validation tooling.

Streaming scans are available through `SharedSnapshot::iter()` and
`SharedSnapshot::range(start, end)`; `scan()` collects the results. Run
`cargo run --example concurrent -- /path/to/new-db` for concurrent writes and a
stable captured view.

Mutation methods consistently return `DbError`, including typed invalid-input,
locked-directory and sequence-exhaustion errors. Keys are limited to 1 MiB,
values to 16 MiB, batches to 100,000 operations and WAL records to 64 MiB.
Oversized input does not poison the engine. The write-buffer option is a flush
trigger, not a hard RSS budget; direct `Kiban` requires explicit flushing.

Compression, range deletes, reverse iterators and transactions are not implemented.

`KibanOptions::block_cache_bytes` defaults to min(physical RAM / 64, 512 MiB),
with a 32 MiB floor. On Linux, a lower cgroup v2 `memory.max` (including
ancestor limits) replaces physical RAM in that calculation. If neither memory source
can be read, the fallback is 32 MiB. This is a per-database budget; opening
several databases multiplies it. Set `block_cache_bytes` explicitly to override
it, including zero to disable block admission.

`KibanOptions::point_index` is opt-in (`false` by default). It writes compact
`KIBANPH3` sections with per-version value checksums in format-4 SSTs. Readers
also accept PH2, flag-zero PH3, and format-3 tables. When the dataset exceeds the
cache budget, resident locators can fetch and verify exact values through pread
or the existing `mmap_max_level` option. Locator and full-data residency share
the block-cache budget. Denied admission uses verified sorted blocks. Older
engines may reject the new section feature; rewrite with the option off before
downgrading. See the [encoding](validation/pointindex-cost/DESIGN.md) and
[small-cache validation](validation/smallcache/REPORT.md).

## Checkpoints, backups and inspection

`checkpoint()` syncs accepted writes and flushes them to SSTs; shared checkpoints
also wait for requested maintenance. `close()` performs that checkpoint and
reports errors before releasing the handle. Other clones/snapshots may retain
ownership. Coordinate concurrent writers for a quiescent checkpoint.

`db.backup(new_directory)` exports a logical view into a newly created database;
shared backups capture a snapshot while writers continue. A completed backup is
synced; an error can leave a partial destination. Existing destinations are never
overwritten. Backup preserves live keys/values, not tuning or version history.

```sh
cargo run --bin kiban -- check /path/to/existing-db
cargo run --bin kiban -- inspect /path/to/existing-db
cargo run --bin kiban -- backup /path/to/existing-db /path/to/new-backup
cargo run --bin kiban -- restore /path/to/new-backup /path/to/new-restored-db
```

The CLI requires exclusive ownership. Inspection/checking performs ordinary open
recovery and may truncate a torn WAL tail or sweep orphan files; use a copy for
forensic examination. Integrity checks read every SST entry and compare point
lookups with scans.

## Durability and limitations

A successful put/delete applies a mutation in memory and appends to a buffered
WAL; it does **not** acknowledge crash durability. A successful `sync()` covers
previously accepted writes in every live WAL generation. Durable flush commits
an SST and its MANIFEST state before retiring the WAL. Dropping a handle is not
a substitute for sync. Sync depends on the OS/filesystem/device honoring its
persistence contract; failures are reported, and unsafe engines reject mutations
until reopened.

Snapshots retain captured state conservatively. Point reads and scans select the
newest visible sequence; a visible tombstone hides older values, while a future
invisible tombstone cannot hide a snapshot's value.

WAL records and SST blocks have checksums. New MANIFESTs have a versioned CRC-32 envelope and strict structural validation.
Legacy unversioned MANIFESTs remain readable and upgrade at their next install;
older Kiban builds cannot read the new envelope. Records above the current size
limits are rejected. Independent engine owners are rejected by the directory lock. SIGKILL testing preserves the OS cache and is not machine power-loss
testing. The volatile-device model and Loom models are bounded; arbitrary
OS/device behavior and every thread schedule have not been proved correct.
These results do not establish production readiness or formal correctness.

## Validation and performance

[Correctness repairs and evidence](validation/CORRECTNESS.md) includes the recorded
1,000-cycle recovery campaigns and their model limits. [Validation guide](validation/README.md)
documents workloads, cache treatment, fault injection and reproduction commands.
The [first scale report](validation/REPORT.md) is historical evidence from before
the correctness repairs; its numbers are not v0.1.0 performance results.

The [foreground report](validation/FOREGROUND_REPORT.md) records the latest
published optimization/endurance campaign, including an unresolved concurrency
throughput decline. [Audit repair evidence](validation/AUDIT_REPAIRS.md) tracks
subsequent fixes and their validation; the
[read-path and range-cache report](validation/RANGE_CACHE.md) covers the read-path
rework, the optional range tier and their benchmarks. Historical measurements describe their
recorded source revision; they are not automatic performance claims for `main`.

The [release baseline and gate](validation/RELEASE.md) records post-correctness
benchmarks, exact source/environment metadata, limitations and release commands.
Throughput and tail latency depend on workload, durability mode and cache state.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --examples
cargo test --features loom --lib -- --test-threads=1 loom_
```

Licensed under [MIT](LICENSE).
