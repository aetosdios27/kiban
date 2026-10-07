# Kiban

Kiban is an embedded LSM-tree key-value storage engine written in Rust. It stores
ordered byte keys and byte values and targets Linux. **v0.1.0 is an early public
release**, with no pre-1.0 API or storage-format compatibility promise.

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
compactions do not change the captured view. Finish with
`db.release_snapshot(&snapshot)` and drop the snapshot. Shared snapshots release
bookkeeping on drop. `SharedKiban` offers clonable concurrent handles to one
engine with background flush/compaction; its snapshot exposes `get` and `scan`.
Use one engine owner per directory; independent owners are not validated.

## Supported operations

- WAL-backed puts, deletes and atomic WAL write batches with explicit sync.
- Mutable/frozen memtables, SSTables and leveled compaction with an adaptive
  governor by default; optional fixed-priority and scored schedulers.
- Point reads, scans/iterators, tombstones and sequence-bounded snapshots.
- Recovery from WAL/MANIFEST state; poisoning after unsafe WAL/commit failures.
- Bloom filters, block cache, bounded SST descriptor cache and optional mmap.
- Maintenance/read-amplification metrics and reproducible validation tooling.

Compression, range deletes, reverse iterators and transactions are not implemented.

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

WAL records and SST blocks have checksums. MANIFEST has structural validation
but **no checksum**. Independent processes opening the same directory are not
validated. SIGKILL testing preserves the OS cache and is not machine power-loss
testing. The volatile-device model and Loom models are bounded; arbitrary
OS/device behavior and every thread schedule have not been proved correct.
These results do not establish production readiness or formal correctness.

## Validation and performance

[Correctness repairs and evidence](validation/CORRECTNESS.md) includes the recorded
1,000-cycle recovery campaigns and their model limits. [Validation guide](validation/README.md)
documents workloads, cache treatment, fault injection and reproduction commands.
The [first scale report](validation/REPORT.md) is historical evidence from before
the correctness repairs; its numbers are not v0.1.0 performance results.

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
