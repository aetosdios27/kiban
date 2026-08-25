# Kiban

Kiban is a single-node, embedded, durable, ordered byte-key → byte-value
storage engine, written in Rust from first principles.

## Goal

Kiban is being built as a serious systems project, not a tutorial exercise.
The aim is a storage engine whose durability guarantees, on-disk formats, and
failure behavior are understood and can be defended in detail — not one that
merely appears to work.

## Direction

Kiban's architecture is intended to eventually follow the standard LSM-tree
write path:

```text
write
  ↓
WAL
  ↓
mutable memtable
  ↓
immutable memtable
  ↓
SSTables
  ↓
compaction
```

with supporting machinery (block indexes, Bloom filters, a block cache,
snapshots, iterators, checksums, and crash recovery) added once each is
understood well enough to implement deliberately.

This is a direction, not a current feature list. See `docs/design/` for what
has actually been decided, and `docs/research/` for the investigations behind
those decisions.

## Priorities

- **Correctness before performance.** A fast, wrong database is worthless.
- **Explicit durability.** Every claim about persistence distinguishes
  application buffers, kernel page cache, device persistence, and filesystem
  metadata durability.
- **Understood bytes.** Persistent formats are specified, not hidden behind
  serialization frameworks.
- **Recoverable by design.** Crash recovery is normal execution, not an edge
  case.

See `docs/design/principles.md` for the full set of engineering principles
governing this project.

## Status

Kiban is in its bootstrap phase. The repository currently contains only
project scaffolding and documentation structure — no storage engine
functionality has been implemented.

## Building

```bash
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Kiban currently has zero dependencies.
