# Read-path experiments

This driver reuses `api.rs`, `embedded.rs`, `model.rs`, `runner.rs`, the
histogram, the random sequence, and the raw LMDB implementation from the
in-process head-to-head harness. Helix is excluded. MiMalloc, its C library,
the LMDB C library, JSON dependencies, and release settings match
`engine/Cargo.lock` and `engine/Cargo.toml`.
The additional `crc32fast 1.5.0` dependency is a checksum microbenchmark
reference. It is not an engine dependency.

All subprocesses run on P-cores. Latency trials alternate Kiban and LMDB on
CPU 3. The external E-core crash sweep keeps running. Absolute timings can
therefore differ from the earlier head-to-head report.

```sh
taskset -c 2-5 cargo build --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
python3 tools/agentbench/readpath/run.py prepare --out validation/read-path-results/prepare
python3 tools/agentbench/readpath/verify.py validation/read-path-results/checks
python3 tools/agentbench/readpath/run.py measure --out validation/read-path-results/baseline
python3 tools/agentbench/readpath/profile.py --out validation/read-path-results/profile
python3 tools/agentbench/readpath/writes.py --out validation/read-path-results/writes
python3 tools/agentbench/readpath/summarize.py validation/read-path-results/baseline
```

Use a fresh output folder for each run. Preparation requires fresh database
paths in `target/read-path-data`. The normal shape uses individual writes,
flushes at the default write-buffer budget, and runs compaction through
`Kiban::flush`. Direct `Kiban` delegates flush policy to its caller. No engine
option is changed to manufacture a leveled shape. At 10k records both loading
methods fit in one SST. Reads reopen and fully verify the database before
measurement, check every timed result outside its timer, then fully verify
again. Any mismatch, failed call, or layout change exits unsuccessfully.

Profile loops exclude the oracle's payload generation, histogram, bulk load,
and warmup. `perf` starts disabled. FIFO commands enable counters immediately
before the work loop and disable them immediately after. Profile reads check
IDs and scan counts in the loop and verify all payloads before and after it.
The counter interval includes loop and materialization costs, including
destruction of returned owned objects. Latency timers match the original
harness and exclude destruction, except temporary byte buffers destroyed
inside a backend call.

`profile.py` expects a local `perf` binary at
`target/read-path-tools/root/usr/bin/perf` and inferno at
`target/read-path-tools/inferno/bin`. It uses explicit P-core user events and
`--user-callchains` so kernel stack restrictions do not block sampling.
It retains counters, configuration, folded stacks, SVG flame graphs, text
reports, and compressed decoded stack text. Empty DWARF chains are restored
from the sampled IP, with missing callers labeled. Folded sample periods
must match the complete raw sample count. Raw perf data is removed after
the checked evidence is produced.

The microbenchmarks use the actual 1M bulk SST's filter, separators, verified
blocks, and payloads. They isolate memtable lookup, filter probes, separator
search, cache hashing, an uncontended read lock, Arc cloning, the complete
cache hit, restart-based block seek, full linear iteration, copying, JSON
decode, range construction, first entry, and ten entries. Costs overlap and
must not be added. The linear block iterator also materializes owned entries;
it is a comparison with the existing restart-based seek, not an equivalent
alternative search implementation.

`writes.py` starts each trial from a fresh copy of a prepared database. It
runs the original W1–W5 plan with 1,000 calls per workload, in fast and durable
modes, preserving p50 and p99 for insertions and updates. Output includes all
other stages and final verification. Original read-only datasets remain
available for later comparisons.
The default write check uses the 10k bulk database. Use `--populations` and
`--layouts` to select additional datasets.

`micro_compare.py` alternates two component cases or two frozen binaries,
with counters enabled only during their loops. For example:

```sh
python3 tools/agentbench/readpath/micro_compare.py --before block-seek --after block-compare --out validation/read-path-results/block-compare-prototype
python3 tools/agentbench/readpath/micro_compare.py --before iter-construct --after iter-construct --before-binary target/read-path-tools/kiban-readbench-inline --after-binary target/read-path-tools/kiban-readbench-range-alloc-final --layout normal --out validation/read-path-results/range-construct-normal
```

`block-indexed` and `block-compare` verify sampled results against the
engine decoder before enabling counters. Resident-index and borrowed-cache
prototypes that failed complete-read comparisons are preserved as patches
under `validation/read-path-results/indexed-prototype-source`, based on
`1a685d1`. Apply each independently in a disposable worktree; the packed
variant uses `inline-search.patch` with its block-index module replaced by
`packed-block-index.rs.txt`. Neither cache prototype is part of the engine.

`profile_pair.py --before <frozen-binary> --after <frozen-binary> --out <folder>
alternates full W2 profiles with LMDB controls; use `--stage 2` for W3 and
`--stacks` for complete sampled-IP flame graphs. Offline perf decoding and
SVG generation inherit the profiler coordinator's P-core affinity (2–5);
the timed children remain pinned to CPU 3.

## SQLite comparison

The SQLite campaign adds `backend: "sqlite"` with bundled rusqlite. Config options
are `sqlite_layout` (`rowid` or `clustered`), `sqlite_sync` (`OFF`, `NORMAL`, or
`FULL`), and `sqlite_mmap_bytes`. SQLite durability follows `sqlite_sync`;
`durable` controls Kiban and LMDB. The driver records effective pragmas, SQLite
version/source ID and indexed query plans. SQLite uses cached statements,
one connection per worker, and one commit per acknowledged mutation.

[`validation/sqlite/REPORT.md`](../../../validation/sqlite/REPORT.md) describes
schema selection, crash guarantees, cache budgets, timing boundaries, CPU
allocation and the reproduction commands. `sqlite_bench.py` prepares fixtures,
screens layouts and runs five rotated trials. Its select phase also invokes
`sqlite_mmap_screen.py` before choosing the mmap layout. `sqlite_report.py`
audits receipts and generates tables, JSON and CSV. `sqlite_fdatasync.py`
compares Linux SQLite FULL sync implementations in a separate five-round group.
CPU counters and sync-call probes are separate diagnostics.

All benchmark children use CPU 10. Keep CPU 11 free of benchmark work. Build
and test on CPUs 12–21 at nice 10, with no timed campaign active. Start with
fresh private data and output directories. `--resume` validates completed
receipts and frozen binary/source hashes. The write suite now fully verifies
again after closing and reopening each database.
