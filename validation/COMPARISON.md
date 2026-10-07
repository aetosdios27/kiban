# Comparative validation

Kiban v0.1.0 is competitive on small cached point reads in this campaign, but
RocksDB is substantially ahead on write and mixed workloads. Kiban's mixed
throughput stops improving after four threads while foreground write tails
continue growing. These results locate symptoms; they do not establish a causal
bottleneck or production readiness.

## Reproduction and scope

The unpublished [comparison tool](../tools/compare/README.md) pins Kiban
**76d9d857c61bed3127fa3f7a2136701cd9b49380** (the v0.1.0 tagged release commit)
and RocksDB **11.8.1**, through rocksdb **0.25.0**. Engine source, historical
benchmarks and the release tag are unchanged. Harness commits are `835759a`
and `c1ee3db`; each campaign retains exact source snapshots/hashes, native options,
commands and environment. The second commit adds checked binary reuse rather
than changing compiled Rust or workloads. The initial binary hash was recorded
after the smoke campaign, explicitly marked in its metadata; the binary predates
that campaign and was unchanged. Subsequent campaigns validate that hash and all
compiled inputs before execution.

Host: Intel Core Ultra 9 185H, 22 logical CPUs, approximately 30.8 GiB physical
RAM, Fedora 44/Linux 7.2.8, rustc 1.98.1, release opt-level 3 with debug information,
no LTO. Storage is encrypted Btrfs/NVMe with filesystem zstd compression. This
is a shared desktop, with other processes and swap activity; trials are serial,
paired, randomized in recorded order and repeated three times. No CPU isolation,
power changes or global cache clearing were performed. Read results are strongly
specific to dataset/cache settings; neither engine is universally faster.

Matched profiles use 4 MiB memtables, two memtables, 32 MiB block caches, 4 MiB
target SST/L1 budgets, multiplier ten, L0 trigger four/hard stop eight and an
open-file setting of 128, with engine compression disabled. These match named
knobs, not a hard total-RSS budget or maintenance implementation. Defaults are
reported separately. RocksDB primary runs have optional statistics disabled;
statistics-enabled trials are diagnostic. All keys are eight bytes and values
128 bytes, constant per seed. Seed and order seed are `20261008`. Full parameters
are explicit in the [JSON plans](../tools/compare/plans).

Each valid trial completed its exact workload, synced/flushed/closed/reopened,
and verified the full ordered expected key set and values, plus selected point
reads. Every foreground read validates bytes and rejects missing preloaded keys.
Three-repeat tables report medians of per-trial throughput and per-trial p99,
not pooled percentiles or confidence intervals. Histograms have bounded memory
and less than 3.125% bucket error. JSON/CSV also retain p50/p95/p99.9, ranges,
read/write rates, end-sync costs, maintenance, disk size, sampled RSS/FDs and I/O.

## Measurements

### One million operations, four threads

Smoke cases use 100k keys/working set, except sequential writes (1M unique
keys) and hot updates (10k working set within 100k keys). Reads/mixed/hot updates
preload keys; random and sequential writes start empty. Mixed is 50% reads,
hot updates 5% reads. Transition workloads are stage-based; the table aggregates
them, and raw `stages.json`/samples preserve transition behavior.

| Case | Kiban ops/s | RocksDB ops/s | Kiban / RocksDB GET p99 µs | Kiban / RocksDB PUT p99 µs |
| --- | ---: | ---: | ---: | ---: |
| burst-read-1m | 516,163 | 1,326,676 | 13.57 / 5.76 | 52.22 / 8.96 |
| burst-recovery-1m | 460,882 | 1,104,378 | 18.43 / 7.04 | 50.18 / 8.70 |
| mixed-1m | 579,758 | 1,227,890 | 22.53 / 7.55 | 34.82 / 7.42 |
| random-write-1m | 269,163 | 947,079 | — | 55.30 / 9.22 |
| read-1m | 3,501,696 | 3,495,030 | 2.02 / 2.37 | — |
| sequential-1m | 343,673 | 812,316 | — | 46.08 / 8.70 |
| updates-1m | 385,464 | 1,051,692 | 61.44 / 11.26 | 45.06 / 7.55 |

### Concurrency

Matched 1M-operation, 100k-key warm reads and 50/50 mixed traffic:

| Case | Kiban ops/s | RocksDB ops/s | Kiban / RocksDB GET p99 µs | Kiban / RocksDB PUT p99 µs |
| --- | ---: | ---: | ---: | ---: |
| mixed-1t | 416,561 | 658,595 | 13.82 / 4.48 | 6.27 / 2.24 |
| mixed-2t | 511,630 | 1,000,616 | 20.48 / 5.76 | 10.75 / 4.10 |
| mixed-4t | 594,670 | 1,457,657 | 22.53 / 6.53 | 35.84 / 7.04 |
| mixed-8t | 460,373 | 1,579,081 | 29.70 / 8.96 | 104.45 / 15.36 |
| mixed-16t | 441,256 | 1,579,359 | 30.72 / 11.26 | 286.72 / 83.97 |
| mixed-32t | 426,175 | 733,163 | 30.21 / 13.31 | 655.36 / 417.79 |
| mixed-64t | 410,143 | 657,314 | 31.74 / 17.92 | 1572.86 / 770.05 |

| Case | Kiban ops/s | RocksDB ops/s | Kiban / RocksDB GET p99 µs | Kiban / RocksDB PUT p99 µs |
| --- | ---: | ---: | ---: | ---: |
| random-read-1t | 1,914,426 | 1,047,037 | 1.01 / 1.70 | — |
| random-read-2t | 2,738,466 | 1,940,953 | 1.38 / 1.89 | — |
| random-read-4t | 3,689,074 | 3,284,938 | 1.82 / 2.37 | — |
| random-read-8t | 5,486,355 | 4,895,260 | 2.56 / 2.75 | — |
| random-read-16t | 5,127,050 | 7,280,733 | 6.53 / 4.99 | — |
| random-read-32t | 5,324,322 | 7,822,517 | 7.81 / 8.96 | — |
| random-read-64t | 5,372,181 | 7,732,381 | 9.22 / 10.50 | — |

32/64 threads deliberately oversubscribe 22 logical CPUs. Kiban mixed throughput
peaks near four threads; its PUT p99 grows approximately 44-fold between four and
64 threads. RocksDB mixed throughput peaks near 8–16 threads. Cached-read scaling
has a different shape: Kiban wins at low counts in this cohort, RocksDB at higher
counts. Do not treat the small four-thread warm-read difference as significant;
trial ranges overlap in the smoke cohort.

### Durability, defaults and cache

Durability uses random writes starting empty. Buffered and sync-every-1000 cases
have 1M operations; per-write durability explicitly uses 10k operations. Sync
latency is included in required write acknowledgement. Native group commit is
allowed. Buffered throughput does not promise per-write crash durability.

| Case | Kiban ops/s | RocksDB ops/s | Kiban / RocksDB GET p99 µs | Kiban / RocksDB PUT p99 µs |
| --- | ---: | ---: | ---: | ---: |
| write-sync-0 | 293,418 | 991,524 | — | 55.30 / 8.19 |
| write-sync-1 | 1,748 | 4,957 | — | 16252.93 / 1605.63 |
| write-sync-1000 | 211,191 | 397,089 | — | 71.68 / 9.73 |

Defaults retain native engine settings and unequal memory/compression policies.
Constant repeated values favor compression; these numbers describe that workload.

| Case | Kiban ops/s | RocksDB ops/s | Kiban / RocksDB GET p99 µs | Kiban / RocksDB PUT p99 µs |
| --- | ---: | ---: | ---: | ---: |
| defaults-mixed | 548,892 | 977,136 | 23.55 / 4.99 | 35.84 / 12.03 |
| defaults-random-read | 3,634,778 | 4,277,753 | 2.02 / 1.92 | — |
| defaults-random-write | 308,968 | 533,566 | — | 51.20 / 12.54 |

Cache cases preload 1M keys (136 MB of application data) and read a 1M-key working
set. Both fit host RAM. Warm means a pre-measurement working-set walk; reduced
means file-scoped advisory DONTNEED followed by reopen. Reduced is not guaranteed
cold, and these are not larger-than-RAM measurements.

| Case | Kiban ops/s | RocksDB ops/s | Kiban / RocksDB GET p99 µs | Kiban / RocksDB PUT p99 µs |
| --- | ---: | ---: | ---: | ---: |
| reads-cache-reduced | 393,693 | 593,588 | 39.94 / 71.68 | — |
| reads-cache-warm | 425,958 | 769,243 | 16.90 / 33.79 | — |

### Write amplification and resources

In separate statistics-enabled random-write trials, median SST write
amplification is **3.09× Kiban / 2.48× RocksDB**, excluding WAL/MANIFEST. Common
Linux process-write accounting gives **4.56× / 3.70×**. These are diagnostic trials,
not counters silently enabled in primary performance runs. Primary RocksDB SST
amplification is unknown and recorded as null. Process I/O counters are not
physical device bytes; filesystem compression, metadata and cancellation matter.

For smoke random writes, median sampled RSS is approximately **79.8 MiB Kiban /
33.9 MiB RocksDB**. Sampling can miss short peaks and excludes verification memory.
End sync/flush/bounded maintenance observation takes median **1.09 s / 1.01 s**
in that cohort, including the observer's one-second polling granularity. Foreground
throughput excludes that deferred work; raw summaries expose the separate costs
and end-sync-inclusive throughput. Native debt definitions differ, so their
absolute values must not be ranked as equivalent metrics.

### Ten million writes/updates and one hundred million reads

Each case has three verified trials per engine, four foreground threads and
128-byte values. Updates use a 10k hot working set within 100k preloaded keys;
random writes start empty and use a 1M-key working set. Reads preload 1M keys
and warm a 100k-key working set. Thus the 10M random-write case also changes
population size relative to smoke; the difference cannot be attributed to
operation count alone. The 100M evidence is read-only, not a 100M-write claim.

| Case | Kiban ops/s (min–max) | RocksDB ops/s (min–max) | Kiban / RocksDB p99 µs |
| --- | ---: | ---: | ---: |
| 100M warm reads | 2,895,634 (2,617,808–3,193,501) | 340,456 (267,666–340,871) | GET 2.30 / 106.50 |
| 10M hot updates | 307,899 (293,808–312,206) | 858,160 (841,159–869,748) | PUT 46.08 / 8.96 |
| 10M random writes | 87,577 (87,286–91,826) | 317,130 (309,648–335,048) | PUT 59.39 / 10.24 |

The larger read case reverses the relative throughput seen in the full-working-set
1M-key cache cases. This result held across all three repetitions, but its cause
has not been isolated. Warm means a preparatory walk, not a measured 100% block-
cache hit rate. Native cache hit/miss statistics were not enabled in primary runs.
The result is specific to these traces, file layouts and settings.

For 10M random writes, median sampled peak RSS is **254.4 MiB / 39.2 MiB**;
process-write amplification is **10.99× / 5.95×**. Kiban SST amplification is
**9.27×**; RocksDB SST amplification remains unknown in these primary trials.
Per-trial maximum PUT latency is **1.37–1.81 s Kiban / 0.570–0.593 s RocksDB**.
Rare stalls can be much larger than p99/p99.9.

In Kiban's `write-10m-r2` trial, the native counters record **765 blocked
mutations and 278.8 cumulative seconds of writer waiting across four threads**,
while foreground wall time is 108.9 seconds. This confirms maintenance-pressure
backpressure blocks foreground writes; cumulative wait time is not wall time.
The same run records 247 compactions, approximately 96.2 seconds of native
compaction time, and 401 flushes. Those counters do not isolate whether flush,
compaction I/O, scheduling or commit work is the limiting maintenance component.
Sampled L0 maximum four does not rule out brief higher peaks between samples.
All three runs eventually report zero native debt and settled maintenance.

## Observations, hypotheses and remaining work

**Measured observations:** write/mixed throughput and tails lag RocksDB on the
specified matched cases; Kiban mixed concurrency scaling saturates early. The
warm/reduced 1M-key cases show lower Kiban throughput but lower GET p99. No
wrong value, missing preloaded key, reopen mismatch or ordered-scan mismatch
occurred in the completed primary/diagnostic cohorts. All 204 trials
ended with native maintenance reporting settled within the bounded drain.
This is not a claim about sustained bounds over hours.

**Hypotheses, not confirmed bottlenecks:** foreground serialization and/or
maintenance interactions could explain Kiban's mixed plateau and growing write
tails. Cache/layout effects could explain changes in relative read performance.
No lock profile or read-cache hit/miss diagnostic was collected in primary runs,
so this campaign cannot choose among those explanations. Native metrics are
retained for subsequent correlation. The confirmed mechanism in the larger write run is maintenance-related
foreground backpressure (native blocked-mutation and wait-time counters). These
throughput tables cannot isolate flush versus compaction pressure, quantify all
lock contention, or explain the complete cross-engine gap. No optimization was made.

**Prepared but unmeasured:** the 600-second sustained-write plan, one-hour/
overnight extensions, nine-million-key × 4,096-byte larger-than-RAM plan,
1B+ operation budgets, and comparisons with LMDB/redb. The large-memory plan
contains approximately 36.9 GB of application data before reads and needs ample
disk/time. All executed data fits host RAM; these results do not establish
out-of-memory storage performance. The comparison SIGKILL check is a sanity
regression, not a replacement for Kiban's existing recovery campaigns or a
machine-power-loss model.

## Evidence and checks

The completed campaign has **204 valid trials, 102 complete engine pairs and
900,060,000 combined foreground operations**. Counts exclude preload, warming,
verification and the separate SIGKILL check. Every case has three repetitions per
engine. No engine trial failed, timed out or had a detected clock/suspend gap.
The [combined JSON/CSV summary](comparison-results/summary/comparison.json)
retains per-trial ranges, resource use, maxima and the analysis-source hash.

The per-campaign [raw results](comparison-results) include machine-readable
JSON/CSV, periodic state, exact configurations, verification markers, source
snapshots and commands. Database directories are retained locally under
`target/comparison-data`, outside the published crate. Generated results and the
comparison tool are excluded from crate packaging, but remain in the repository.
The initial `scaling` directory records a **build setup failure before any trial**:
bindgen could not locate standard headers because the build environment was
omitted. That failure was inspected, preserved and excluded; the unchanged plan
ran under `scaling-validated-build` using verified existing binary/source hashes.
No engine trial was retried, discarded or parameter-reduced to improve results.

Fresh [check logs](comparison-results/checks) cover formatting, normal/Loom Clippy,
260 library and three integration tests, three example tests, three Loom models,
and two release-mode ignored stress tests. The standalone harness passed five
unit tests, its SIGKILL/reopen integration test for both adapters, release Clippy
and formatting; the Python supervisor/plans passed six tests including timeout,
wall-clock-gap invalidation and child reaping. Engine source and the v0.1.0 tag
are unchanged.
