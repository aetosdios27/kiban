# Kiban / RocksDB comparison

This separate, unpublished tool compares **Kiban v0.1.0 commit
76d9d857c61bed3127fa3f7a2136701cd9b49380** with **RocksDB 11.8.1** through the
pinned rocksdb 0.25.0 Rust binding. Kiban is a Git-revision dependency, so local
engine changes do not silently change the baseline. Cargo.lock pins transitive
dependencies. No database implementation or historical benchmark was changed.

The tool imports the existing scale workload generator, counter-based PRNG,
configuration parser, cache-reduction helper and bounded histograms. Both engines
run the same operation IDs, keys, values, stage boundaries and thread partitions.
Concurrent execution order is naturally nondeterministic; values are constant per
seed, so reordering writes does not change the expected logical state. Every read
validates the returned bytes. Missing preloaded keys fail immediately. Each run
syncs, flushes, closes, reopens and verifies the complete expected ordered key set,
values and selected point reads. These checks are outside the measured foreground
window. A SIGKILL regression tests acknowledged overwrites and deletes across
flush/WAL state for both adapters. This is not a power-loss test.

## Build and checks

From the repository root, with a C++20 compiler and libclang available:

```sh
CARGO_BUILD_JOBS=2 cargo build --release --locked --manifest-path tools/compare/Cargo.toml
cargo fmt --manifest-path tools/compare/Cargo.toml --check
cargo clippy --release --manifest-path tools/compare/Cargo.toml --all-targets -- -D warnings
cargo test --release --manifest-path tools/compare/Cargo.toml
python3 -m unittest discover -s tools/compare/tests -p 'test_*.py'
```

The first RocksDB build compiles native C++; subsequent builds reuse it. For further
campaigns with identical compiled sources, `--prebuilt-from PATH_TO_VALIDATED_CAMPAIGN`
checks the binary and all compiled-source hashes before reusing it. It refuses a
changed binary/source instead of silently running stale code. This
Fedora host has libclang but lacks its resource headers. The supported workaround
for bindgen is to supply the installed GCC standard headers:

```sh
export BINDGEN_EXTRA_CLANG_ARGS="-isystem $(gcc -print-file-name=include)"
```

Use the same build environment throughout; the runner records it. The release
profile uses opt-level 3 defaults with debug information, no LTO. Measurements
must use the release binary, never a debug build. Native builds and correctness
checks must finish before timed comparisons. Source snapshots and hashes are
retained with each campaign.

## Run plans

```sh
# Print deterministic commands without creating databases.
python3 tools/compare/run.py --plan tools/compare/plans/smoke.json \
  --output-root validation/comparison-results/smoke \
  --data-root target/comparison-data/smoke

# Build if necessary, then execute serially in randomized, recorded paired order.
python3 tools/compare/run.py --plan tools/compare/plans/smoke.json \
  --output-root validation/comparison-results/smoke \
  --data-root target/comparison-data/smoke --execute

python3 tools/compare/analyze.py validation/comparison-results/smoke
```

Output roots must be new, databases must be fresh, and data/output trees must be
separate. No data is deleted automatically. The runner stops at the first failed,
incomplete, timed-out or suspend/clock-gap-affected run, retaining evidence. It
does not retry or reduce parameters. Choose a new directory for an explicitly
requested rerun. Build failures are also retained and stop the campaign.

Plans explicitly pin every shared workload parameter, each timeout and all
matched engine knobs. Repetitions use the same workload seed; scheduling/host
variability remains. A separate order seed randomizes paired engine order and
case/repetition order. Medians and ranges are reported; three trials are not a
statistical significance claim. Never compare an engine from one host/time/profile
with another engine from a different setup.

- `smoke.json`: seven 1M-operation workloads, four threads, three trials/engine.
- `scaling.json`: read/mixed cases at 1/2/4/8/16/32/64 threads. On this 22-logical-CPU
  host, 32/64 are deliberate oversubscription tests, not additional physical cores.
- `durability.json`: buffered, per-thread sync every 1,000 writes and durable
  per-write acknowledgement. The per-write case uses **10,000 operations**, not
  1M; that explicit smaller budget bounds fsync-heavy trial duration.
- `defaults.json`: native defaults, separately labeled, three trials.
- `scale.json`: 10M updates/writes and 100M warm reads, three trials.
- `cache.json`: warm versus advisory cache-reduced 1M-key reads.
- `statistics.json`: separate RocksDB statistics on/off sensitivity trials.
- `long.json`: 600-second sustained-write trial; change duration/timeout explicitly
  in a copied plan for one hour or overnight, keeping both engines identical.
- `large-memory.json`: nine million 4,096-byte values, approximately 36.9 GB of
  application keys/values, greater than this host's physical RAM. Preparation
  writes the entire population before reads. It needs ample time/disk; do not
  confuse prepared commands with measured evidence. For 1B+ operations, change
  only the explicit operations budget in a copied plan; no harness rewrite is needed.

## Fairness and durability

Matched cases use equal **named knobs**, not an equal total-RSS guarantee: 4 MiB
memtable thresholds, at most two memtables, 32 MiB block caches, 4 MiB target SST
and L1 base budgets, multiplier ten, L0 compaction trigger four and hard stop
eight, 128 SST/open-file setting, and no engine compression. Both use 10-bit/key
Bloom filters. Internal metadata, file accounting, sizing formulas, L0 slowdown
policy and maintenance worker policies differ. RocksDB dynamic-level sizing is
disabled to make the configured base/multiplier meaningful. No engine is manually
fully compacted to give it a cleaner starting state. Preload sync/flush/close and
reopening follow the same lifecycle in both adapters.

Defaults cases retain native storage settings: Kiban 4 MiB memtable/32 MiB cache
and no compression; RocksDB's defaults include larger memtables and Snappy.
Native RocksDB OPTIONS files are copied into run output. For checked-in evidence,
`python3 tools/compare/archive_options.py CAMPAIGN_ROOT ...` replaces them with
byte-exact gzip archives and an original-byte SHA-256 manifest. It verifies each
archive before removing the plain copy; `gzip -dc OPTIONS-....gz` restores it. Match/defaults must be
reported separately; default results do not imply equal resource budgets.

`sync-every=0` acknowledges buffered acceptance and ends with explicit durable
sync. Matched RocksDB uses manual WAL buffering; native-default RocksDB uses its
native WAL flush policy. Neither mode promises per-write crash durability.
`sync-every=1`: Kiban put+sync; RocksDB WriteOptions.sync=true. Native group commit
is permitted; don't disable it to make implementations artificially alike.
`sync-every=N`: each thread syncs after its own N writes; intervening puts are
not durability acknowledgements. Remaining tails receive end sync. Latency
includes any required write-associated sync. End sync, flush, bounded maintenance
observation and close are timed separately. Durable-completion throughput includes
end sync. Foreground throughput must not conceal deferred flush/compaction costs.

Kiban uses the public `SharedKiban` thread-safe API even at one thread; RocksDB
uses its native thread-safe `DB` API. Single-thread direct-`Kiban` throughput is
not substituted into this concurrency comparison.

The adapter reuses immutable RocksDB read/write options instead of allocating
configuration objects per operation. Both get APIs return owned byte vectors;
zero-copy APIs are not substituted for only one engine. Shared timing/value checks
and histogram accounting are identical. Histograms have <3.125% relative bucket
error above 32 ns; p99.9 requires at least 10,000 samples. Per-thread counters are
published every 4,096 operations, so interval windows are approximate; stage/total
counts are exact. Histogram memory is bounded even at 1B operations.

## Output and metric limits

`metadata.json` records commit, dirty state, engine revisions, compiler, build
environment, OS, CPU, initial RAM/swap, filesystem, source hashes, locked packages,
plan and exact commands. `runs.json` marks valid/incomplete/failed trials. Each run
has config.json, preparation.json, stages.json, samples.jsonl, summary.json and
status.json. Summary includes foreground and durable-completion throughput,
read/write/all p50/p95/p99/p99.9 and maxima, misses, application bytes, sampled peak RSS/FDs,
L0 maximum, size, close/drain costs, native state and full verification outcome.

Engine-native cumulative statistics are sampled once per second. Linux process
read_bytes/write_bytes provide common I/O accounting, **not physical device
bytes**. Application bytes count eight key bytes plus each value for every put.
SST amplification excludes WAL/MANIFEST, keys/values have no per-engine denominator
change, and sampled engine counters are baseline-subtracted. Kiban's native
compaction/flush/stall counters are always enabled. RocksDB optional statistics
are **off in primary performance runs**; unknown counters are null, never zero.
Separate statistics-on trials expose SST rewrite bytes without silently changing
primary instrumentation. Their throughput must be labeled diagnostic. No custom
engine instrumentation was added. RocksDB compaction count/duration/stall count
remain unreported rather than guessed.

Kiban debt sums bytes over per-level budgets excluding L0. RocksDB debt is its
native estimated pending-compaction bytes; these definitions are not directly
comparable. Report within-engine evolution, along with L0 and actual throughput.
A bounded drain ending unsettled is visible; it is not silently called convergence.
RSS covers the benchmark process; verification allocations occur after sampling
and are not included in sampled foreground peaks. Sample costs are recorded.

The runner detects suspend or wall-clock gaps by comparing real and monotonic
elapsed time. A detected gap invalidates the trial even if all operations succeed.
Timeouts include preparation, verification and close; they expose practical cliffs
rather than turn an incomplete run into a throughput number.

## Cache and host limits

Warm mode reopens and walks the selected working set before measurement. Reduced
mode closes, fsyncs and applies file-scoped POSIX_FADV_DONTNEED before reopening.
This is advisory and is **not guaranteed cold**. No automatic root/global cache
clearing, power configuration changes or sleep inhibitors are used. Root-only
`sync; echo 3 > /proc/sys/vm/drop_caches` is a machine-global optional manual action;
do not run it on a shared machine, and record it if used.

A working set greater than engine block cache but fitting OS RAM is not an
out-of-memory storage test. The large-memory plan's logical data exceeds physical
RAM. This host uses encrypted Btrfs with filesystem zstd; record its effect rather
than confuse filesystem compression with engine compression. Values are constant
per seed, so default-compression results are specific to this repeated-value
pattern. Runs on this shared host are not isolated; per-sample available RAM and
swap are recorded. Runtime limits and practical unexecuted cases must be explicit.
