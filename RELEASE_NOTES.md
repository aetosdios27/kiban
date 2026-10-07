# Kiban v0.1.0

First public release of Kiban, an embedded LSM-tree key-value storage engine written in Rust. This is an early release; API and storage-format compatibility are not promised before 1.0.

## Highlights

- WAL-backed writes, recovery, mutable/immutable memtables and SSTables.
- Point reads, ordered scans, deletes, sequence-bounded snapshots and shared read views.
- Leveled compaction with an adaptive governor, Bloom filters and block caching.
- MIT license, a persistence example, reproducible benchmarks and recovery tooling.

## Correctness

The release includes repairs for frozen-WAL durability, snapshot visibility across flushes, point-read/scan version resolution, flush installation retries and snapshot file lifetime during compaction. Successful buffered writes require `sync()` for an explicit durability acknowledgement. Dropping the database is not a substitute for sync.

See `validation/CORRECTNESS.md` for deterministic regressions, campaign configurations, raw evidence and model limitations. Testing is evidence, not formal proof or a production-readiness claim.

## Performance

The full canonical benchmarks are rerun on the release candidate. See `validation/RELEASE.md` and its recorded baseline for configurations, compiler/host metadata, commands and measured results. Earlier benchmark results predate the correctness fixes and are not v0.1.0 performance claims. These measurements include Linux page-cache effects and do not establish cold-storage or larger-than-RAM performance.

## Installation

After the tag is pushed:

```toml
[dependencies]
kiban = { git = "https://github.com/aetosdios27/kiban", tag = "v0.1.0" }
```

Registry installation with `cargo add kiban` becomes available only after crates.io publication succeeds. See `examples/basic.rs` for open, put, sync, read and reopen.

## Known limitations

MANIFEST is structurally validated but has no checksum. Snapshots conservatively retain captured state. Independent owners opening one directory are not validated. SIGKILL is not machine power loss; arbitrary OS/device failures are not exhaustively modeled. Compression, range deletes, reverse iteration and transactions are not supported.
