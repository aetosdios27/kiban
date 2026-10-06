# Kiban scale validation

This phase measures limits; it does not change engine policy. Build:

```sh
cargo build --release --examples
```

`scale --help` and `torture --help` list all flags. No new Rust dependencies. Run the commands from the repository root.
Use a dedicated directory on the filesystem under test. **Check `findmnt -T DIR`:
`/tmp` is tmpfs on this machine and is unsuitable for storage measurements.**
Database directories are never automatically deleted. Output files use exclusive
creation; reusing a result path fails. A nonempty database requires explicit
`--reuse true`. Existing arbitrary data can fail value validation; use only a
previous scale database with the same seed and value size.

## Standard campaigns

The script prints exact commands by default. `--execute` builds and runs them
sequentially, records `commands.json`, exit codes, and stdout/stderr logs. It
continues independent runs after a failure and exits nonzero if any failed.
Every run preserves resolved parameters, engine options, source revision/dirty
state, CPU information, memory availability, filesystem, and stage schedule.
No parameters change between repetitions except those explicitly requested.

```sh
# All workloads, including transitions: 1M smoke / 10M / 100M / 1B.
python3 validation/run.py workloads --operations 1000000 --data-root target/data/smoke --output-root target/results/smoke --execute
python3 validation/run.py workloads --operations 10000000 --data-root target/data/10m --output-root target/results/10m --execute
python3 validation/run.py workloads --operations 100000000 --data-root target/data/100m --output-root target/results/100m --execute
python3 validation/run.py workloads --operations 1000000000 --data-root target/data/1b --output-root target/results/1b

# Fixed mix, key/value/working-set/seed/durability; threads vary 1,2,4,8,16,32,64.
python3 validation/run.py concurrency --keys 100000 --value-size 256 --data-root target/data/concurrency --output-root target/results/concurrency --execute

# Duration runs stop issuing operations at the configured duration.
python3 validation/run.py long --duration 600 --data-root target/data/10min --output-root target/results/10min --execute
python3 validation/run.py long --duration 3600 --data-root target/data/hour --output-root target/results/hour
python3 validation/run.py long --duration 43200 --timeout 44400 --data-root target/data/overnight --output-root target/results/overnight

# Recovery + snapshots: nonzero exit is failure evidence, not an expected pass.
python3 validation/run.py torture --data-root target/data/torture --output-root target/results/torture --execute
python3 validation/analyze.py target/results/concurrency
```

`--operations` is u64. Memory used by operation telemetry is bounded by thread
count, not operation count. CSV records stream to files rather than accumulating
in RAM. Long runs naturally grow their output files; include their disk space
in planning. External timeout (default 12 hours) also catches deadlocks that
prevent both operations and sampling; a missing `complete` status is incomplete,
not success. Save the live process stack before killing a hung run if practical.

## Workload definitions and reproducibility

Keys are eight-byte big-endian integers. Values are deterministic pseudorandom
bytes generated from the recorded seed. Counter-based random operation choices
and keys depend on `(seed, stage, operation ID)`. Threads receive fixed strided
operation IDs; scheduling affects interleaving, not the requested operation set.
Repeated concurrent runs are reproducible workloads, not identical histories.
Per-stage budgets divide the total operation/duration limits evenly, preserving
remainders; each stage stops at its first limit. Blocking operations may finish
past a duration deadline. Drain/close/preload/warming time is separate.

- Sequential writes advance across `keys`, wrapping explicitly after that count.
  To grow unique data, choose `keys >= operations`.
- Random writes and sustained writes uniformly target `working-set` keys.
  Sustained is a named duration/stress profile, not special engine behavior.
- Random reads target that same range. `--preload-keys` controls population;
  misses are counted. The standard read/mixed profiles preload all keys.
- Mixed uses the recorded read percentage.
- Update-heavy uses that same mix; set a small, explicit `--working-set` to create
  hot updates. Its default working set is not silently reduced.
- Burst-read: 0% reads followed by 95% reads.
- Burst-recovery: configured mix, 0% reads, 95% reads.
- Concurrency-jump: configured mix at one thread, then at requested concurrency.

Each transition keeps the same database/worker/governor. `stages.csv` records the
exact mix, concurrency, operation cap, and duration cap for every stage.
Foreground values are checked for content consistency; writes deliberately use
the same value per run, so this benchmark cannot detect stale versions of an
update. **The torture oracle uses versioned values and deletes for that purpose.**

`--sync-every 0` syncs only at the end. Successful puts in this mode are not
acknowledged durable operations. `1` syncs each write, with sync included in write
latency; `N` group-syncs every N writes per thread, with that call's sync included.
Intermediate writes in a group are not individually durable. Final sync and
flush are always attempted and reported separately. No engine durability is
weakened. Preload syncs and flushes before measurement.

## Metrics and limits

- `summary.csv`: per-stage/total throughput and p50/p95/p99/p99.9, separately for
  reads/writes and combined. Latencies are ns. Fixed histograms use 32 buckets
  per power of two, reporting bucket upper bounds (at most 3.125% relative error
  above 32ns). p99.9 is blank below 10,000 observations. All operations are timed;
  key selection is outside latency, while public engine calls and configured
  sync are inside. Throughput includes harness overhead and value checks.
- `samples.csv`: interval latency/throughput, cumulative workload bytes, engine
  state/counters, governor signals, RSS, FDs, Linux process I/O, and sample cost.
  Workers publish metrics every 256 operations and at exit, so an interval can
  lag by up to 255 ops per thread. Final totals are exact. Sampling defaults to
  1s; peaks between samples can be missed. Verify publication/sampling overhead
  with a profiler before drawing small-performance-difference conclusions.
- `levels.csv`: table count and bytes for every populated level at every sample.
- `decisions.csv`: actual existing governor decisions, per-open sequence number,
  observed sample time, and gap count. It retains 4096 decisions in-engine;
  sequence gaps reveal overwritten records. Observation time is **not** decision
  execution time. A transition's exact decision-to-phase assignment is uncertain
  within one sample interval. Preload decisions can precede initial measurement.
- `db_bytes`: apparent size of *all* files in the database directory, including
  WALs, obsolete files and temporary outputs, not only published SST sizes.
- Application bytes = key + value per successful put. `sst_bytes_written` and
  `sst_write_amp` use completed flush/compaction output metadata. They exclude
  WAL, MANIFEST, unsuccessful/orphan output, filesystem journaling and device
  write amplification. Initial preload is excluded; final flush/drain is included
  in the final sample. Read-only write amp is blank. Compaction input bytes are
  the complete-file-size estimate from existing counters, not a device read meter.
- Linux `process_read_bytes`/`process_write_bytes` are deltas of `/proc/self/io`:
  storage-attributed process I/O, including telemetry and background threads.
  They are not an exact engine-only/device-wide byte counter. Cache hits and
  tmpfs can produce zero device reads/writes. Missing OS metrics are blank.
- Compaction/flush time = successful BUILD + COMMIT elapsed ns, including commit
  gate waits, excluding PLAN and pacing. Trivial moves count as jobs with zero
  rewrite bytes. Failed-job duration is not counted.
- Stall count = calls that hit backpressure, not lock acquisitions. Stall ns sums
  actual progress waits across callers (overlap possible), excluding gate waits.
  Ordinary gate contention is not directly instrumented. `perf record -g -- ...`
  or `perf stat -- ...` can investigate it where permitted; no privileged profiling
  or machine changes are performed automatically.
- Debt = sum of positive `(level bytes - level budget)` over L1+, with maximum
  budget ratio and overbudget-level count. Pending flush is separate. Neither is
  a literal job queue: the scheduler synthesizes jobs on demand.

The existing governor's `batch_sizes_chosen` vector grows with deep-level jobs;
`stats()` recomputes its average by scanning that vector. This is a code-level
long-run risk, not a measured cause of degradation. Left unchanged in this phase.

## Larger than memory and cache attribution

Choose a *populated* data set well above RAM available to the process/cgroup,
not just above Kiban's block cache. On this host, a 50M-key x 1024-byte population
is approximately 51.6GB of application payload, above 31GiB physical RAM.
Allow additional space for LSM overlap, WALs and preserved outputs; verify actual
`db_bytes`, filesystem free space, RAM and cgroup limits before and during the run.

```sh
# Preparation is intentionally explicit and potentially expensive.
python3 validation/run.py large-memory --operations 50000000 --keys 50000000 --working-set 50000000 --value-size 1024 --threads 4 --data-root target/data/large --output-root target/results/large
```

Warm mode reopens (empty Kiban cache), then reads the whole configured working set
before timing. With a set larger than RAM, that pass cannot make the whole set
resident; call it a warm-up pass, not a fully warm working set. Reduced mode closes
the engine, syncs each database file, issues file-scoped `posix_fadvise(DONTNEED)`,
and reopens. It is advisory; it cannot guarantee cold storage. No system-wide
cache clearing occurs. Compare process storage-read deltas, cache misses, RSS,
working-set footprint and repeated windows rather than inferring coldness from
reopen alone. Compaction can warm pages in mixed workloads.

If an administrator chooses a global cold-cache experiment on an otherwise idle,
dedicated machine, the manual Linux command is:

```sh
# MACHINE-GLOBAL: flushes writes and evicts shared page/inode/dentry caches.
# Documented only; the harness NEVER executes this.
sudo sh -c 'sync; echo 3 > /proc/sys/vm/drop_caches'
```

## Crash semantics

`simulated` reuses `sys::install_fault`, `enable_device_sim`, and `power_loss` with
direct `Kiban`. The framework is thread-local; it cannot represent the background
worker. It keeps the same simulated namespace across cycles. Seeded crash limits
and fault indices are recorded. Successful sync/flush advances a durable oracle;
normal power loss requires exact equality. A failed/ambiguous flush permits only
known per-key durable/attempt values. Every recovery checks get/scan agreement,
all key-domain tombstones, ordered/unique scan results, and MANIFEST reference
existence/uniqueness. SST checksums are exercised by full scans; WAL/open validates
replay. Snapshot checks are on by default and fail the run if inconsistent.

`process` runs concurrent `SharedKiban` rounds in a child. Per-thread key partitions
make overwrite order deterministic. A round receipt is printed only after sync
success; the parent updates its oracle before permitting any unsynced tail. A
seeded delay then SIGKILLs the child, waits for it, reopens and checks the same
invariants against durable state plus explicitly known tail values. This models
process death with a surviving OS cache, **not power loss**. Kill scheduling is
inherently nondeterministic; seed, delay, acknowledged count and tail budget are
recorded. Receipts are parent memory, adequate for killing only the child; this
is not an oracle for machine power loss.

`--snapshots false` is an explicit independent recovery investigation, recorded
in configuration. It never makes a snapshot-checking run pass. Known failures
remain nonzero exit codes in preserved results. `--mode probes` runs two small,
real-file correctness reproductions without any injected fault or process kill.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --lib --tests
cargo test --examples
cargo test --features loom -- --test-threads=1 loom_
cargo clippy --all-targets --features loom -- -D warnings
cargo test --lib every_single_syscall_failure_recovers_correctly_under_governor
# Known failing regression, deliberately run separately; must not be reported as a pass.
cargo test --lib sync_after_freeze_must_durably_cover_the_older_wal -- --ignored --nocapture
```

The frozen-WAL durability reproducer is retained with its assertion intact and
is ignored by default; the report records its explicit failure. The full library
suite includes existing single/pairwise faults, power-loss,
background flush failure and snapshot tests. New integration tests validate the
oracle and failure detection; their success is **not** evidence that the known
storage defects pass scale validation. See REPORT.md for measured results and
remaining unexecuted campaigns.
