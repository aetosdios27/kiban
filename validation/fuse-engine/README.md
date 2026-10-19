# Reproduce the integrated filter experiment

Source baseline: main 76a6a190; no runtime dependencies added. The baseline
worktree is target/fuse-baseline at that commit. Its harness receives the same
raw membership method and missed-ID stages as this tree; the two references to
KibanOptions::binary_fuse_filter are removed because main has no such option.
The frozen before/after hashes, hardware, toolchain and build settings are in
provenance.json. Never replace a frozen binary while resuming results.

Build and tests run under `taskset -c 12-21 nice -n 10`, outside timing windows.
Readbench uses the existing release profile (opt-level 2, one codegen unit,
LTO); the maintenance test uses the root release profile on both sides.
On this host readbench's existing RocksDB dependency needs
`BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/usr/lib64/llvm15/lib/clang/15.0.7`.
`ROCKSDB_LIB_DIR` and `ROCKSDB_STATIC=1` may point at an already built
librocksdb-sys 0.19.0 / RocksDB 11.8.1 static archive (path in provenance.json).
RocksDB is compiled into the binary, but is not a timed backend in this run.

Full correctness gate:

```sh
python3 tools/agentbench/readpath/verify.py validation/fuse-engine/gates-final --cpus 12-21
```

Build `kiban-readbench` and the root release unit-test executable for each
worktree with the affinity above. Copy them to target/fuse-engine/bin as
before, after, before-test, after-test. The candidate maintenance measurement
uses KIBAN_FUSE_BENCH=1 to enable its writer option; baseline is unchanged.

Prepare and measure, serially, with no builds or tests during either timed
phase:

```sh
python3 tools/agentbench/readpath/fuse_engine.py prepare --before target/fuse-engine/bin/before --after target/fuse-engine/bin/after
taskset -c 8 python3 tools/agentbench/readpath/fuse_engine.py measure --before target/fuse-engine/bin/before --after target/fuse-engine/bin/after
taskset -c 8 python3 tools/agentbench/readpath/fuse_engine.py maintenance --before target/fuse-engine/bin/before --after target/fuse-engine/bin/after --before-test target/fuse-engine/bin/before-test --after-test target/fuse-engine/bin/after-test
python3 tools/agentbench/readpath/fuse_engine_report.py
```

Preparation launches children on 12-21/nice10. Measurement children run on CPU
8, with CPU 9 accounting per process. Receipts permit restart only after complete
validated commands; inspect incomplete files rather than silently deleting them.
Each layout/cache/index/action cell contains five alternating before/after
pairs, every side followed by LMDB. Read and miss suites contain 100k calls per
stage and full population checks before and after. The write suite has 10k
insertions, 10k updates, 10k reads, 10k latest-K, and 10k mixed calls (21k writes
in total), immediate write readbacks, final sync/flush, and full reopen checks.
Each write run starts from its own disposable copy of the 10k population.

The missing-key stages use high-half IDs, with balanced, round-robin selection across all 64 agents or under agent
7's prefix; the remaining ID bits are pseudorandom. Table-range routing can reject them before a filter query. The
report does not equate these engine timings with forced filter lookups.

`xxh3-vectors.c` regenerates hash vectors against the BSD upstream xxhash.h
v0.8.3; no C build or library is required by the engine or its Rust tests.

## Round two: filter before the resident point index

After the first round, the lookup-order change (`SstTable::get` consults
the fuse filter before a resident point index) is measured with a paired
driver that reuses `fuse_engine.execute` and the same frozen round-1
after-format fixtures; both sides read them byte-for-byte because the
change is lookup-order only:

```sh
BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/usr/lib64/llvm15/lib/clang/15.0.7 \
  taskset -c 12-21 nice -n 10 cargo build --release --manifest-path tools/agentbench/readpath/Cargo.toml
cp tools/agentbench/readpath/target/release/kiban-readbench target/fuse-engine/bin/after2
taskset -c 8 python3 tools/agentbench/readpath/fuse_miss_order.py \
    --before target/fuse-engine/bin/after --after target/fuse-engine/bin/after2
python3 tools/agentbench/readpath/filter_first_report.py \
    --results validation/fuse-engine/results2 --md validation/fuse-engine/FILTER-FIRST-TABLES.md
```

`--before` is the frozen round-1 binary (the "after" of `provenance.json`);
receipts land in `validation/fuse-engine/results2`. Round two runs no
prepare, writes or maintenance: the fixtures are immutable, every process
verification covers the full one-million-record population, and the
change cannot affect build paths. Binary hashes for this round are in
`provenance-filter-first.json`; gate receipts in `gates-fpi/`.
