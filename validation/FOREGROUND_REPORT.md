# Foreground performance and endurance report

This campaign follows the maintenance-progress pass. Its engine candidate is
`5efc19b1b8b4e046712a560e5edc06c18ddb53cd`; later commits add plans, documentation and evidence,
not engine behavior changes. The matched baseline is
`47454e53c95bbc8ac8bad3a1a64a3971e40bc9da`, containing the unchanged
`9b85720` engine and the same scale/verification harness as the candidate.
Both builds use release optimization, debug information, ordinary features,
identical configuration and deterministic seeds. Binary/source hashes, compiler,
host information, commands, trial order and full verification results accompany
each campaign in `foreground-results/`. Reproduction is in `FOREGROUND.md`.

The host is Fedora 44, Linux 7.2.8, Core Ultra 9 185H, 22 logical CPUs and
30.8 GiB RAM, using encrypted Btrfs with zstd compression. Rustc is 1.98.1
(LLVM 22.1.8), with release opt-level 3, debug information and no LTO.
Latency quantiles use the existing bounded histogram's upper estimates
(under 3.125% relative bucket error above 32 ns); maxima are retained exactly.
p99.9 is omitted below 10,000 samples. This is a shared desktop, not an
isolated laboratory host.

## Diagnosis and implemented changes

The earlier 16-thread mixed regression did **not** reproduce in the five-repeat
alternating diagnosis campaign. Its median changed from 415,567 to 424,487 ops/s
(+2.1%). Four-thread mixed improved 11%; eight-thread mixed improved 1.5%.
All 30 trials passed reopened-state verification. Different measurement
conditions are a hypothesis for the earlier loss, not a confirmed explanation.

The long write CPU profile identified SST block construction and verification
as substantial userspace CPU consumers (18.38% and 17.98% of samples).
Assembly annotations placed about 98% of each function's local samples in the
byte-at-a-time CRC dependency loop. These are userspace CPU percentages including
preparation; they are not elapsed I/O-time attribution. The fix processes eight
bytes at a time using safe Rust, the original IEEE polynomial and unchanged
streaming/checksum bytes. No checksum check was removed. Independent bitwise
tests cover lengths, offsets, tails and arbitrary starting states.

A 100M warm-read diagnostic profile found hot shared statistics increments in
the corrected point resolver and block cache. The candidate uses eight aligned
atomic counter slots, selected by the existing stable per-thread slot, with exact
aggregation. It preserves cache membership, capacity, eviction, single-flight,
visibility and synchronization. Slot collisions still use atomics and cannot
lose counts. A 32-thread test checks monotonic observations and the exact final
320,000 increments. Attribution to a contended cache line is supported by the
profile; paired measurements assess the effect of the change.

Two separate correctness bugs were found during SST review: unchecked restart
array subtraction/multiplication and unchecked footer range addition. Malformed
sizes could panic instead of returning corruption, including a restart array
with a valid checksum. Checked arithmetic now rejects them before reads or
allocation. Both new regressions failed with the old arithmetic and passed with
the fixed code. This is not evidence of normal-workload data loss.

The targeted RocksDB 11.8.1 study is `ROCKSDB_STUDY.md`, with source receipts.
It informed chunked checksum processing and aligned observational counters.
RocksDB uses CRC32C, so its hardware CRC implementation cannot replace Kiban's
IEEE checksum. Writer grouping, independent flush scheduling, cache membership
sharding and heap merging remain separate designs, not copied in this pass.

## Matched short workloads — measured

All 60 trials completed and passed full reopened-state verification: 168M
operations total, three trials per case/build. Every trial settled with no
pending flush, running maintenance or compaction debt. Values below are medians
of trial results; three repeats are not a statistical-significance claim.

| Workload | Baseline ops/s | Candidate ops/s | Change |
| --- | ---: | ---: | ---: |
| burst-recovery-1m | 544,761 | 598,146 | +9.8% |
| concurrency-jump-1m | 424,329 | 486,605 | +14.7% |
| mixed-16t | 442,394 | 466,368 | +5.4% |
| mixed-4t | 564,039 | 639,588 | +13.4% |
| mixed-64t | 417,256 | 451,383 | +8.2% |
| mixed-8t | 496,827 | 554,948 | +11.7% |
| read-1m | 3,669,273 | 4,135,844 | +12.7% |
| sequential-1m | 379,747 | 395,704 | +4.2% |
| updates-10m | 414,445 | 465,515 | +12.3% |
| write-10m | 113,325 | 201,820 | +78.1% |

The 10M-write throughput ranges were 80,726–132,088 ops/s for the baseline
and 196,212–252,898 ops/s for the candidate. Warm-read ranges were
3,536,639–3,669,762 and 4,131,413–4,308,090 ops/s respectively.
These ranges must be read from `final-primary/analysis.json` when reproducing;
the complete raw percentile, resource and phase counters remain in the CSVs.

Read p99 improved in all read-bearing cases. Warm-read median p99 went from
1.952 to 1.664 µs; 16-thread mixed reads went from 30.208 to 17.920 µs.
Ten-million-write compaction BUILD time fell from 76.13 to 40.71 seconds;
aggregate L0 writer waiting fell from 214.60 to 76.16 seconds. Aggregate
waiting sums overlapping worker waits and is not wall time. The CPU diagnosis
and matched results support checksum work as a substantial maintenance cost.

Adverse results are retained: four-thread mixed write p99 increased from
31.232 to 32.768 µs, concurrency-jump write p99.9 increased from 573.44
to 622.59 µs, and several maximum-latency medians increased. Sequential write
p99 stayed 44.032 µs. Four-thread mixed SST write amplification increased from
2.63x to 3.20x (cause not isolated); update amplification
stayed about 1.24x. This is not an across-the-board tail-latency or amplification
win. The worst candidate PUT in the 10M-write trials still took 1.09 seconds:
the L0-pressure cliff was reduced, not eliminated.

## Larger-than-memory reads — measured

Nine million deterministic keyed 4 KiB values produced a settled database of
37,505,360,680 logical bytes and **37,510,303,744 allocated bytes**. The host
has 33,032,933,376 bytes of physical RAM. Values are key-dependent and resist
the compression that made constant-value logical sizes misleading on this
Btrfs/zstd host. This separately named plan uses 32 MiB memtables/SST targets
and a 256 MiB base budget; it is not pooled with small-workload results.

| Preparation | Reads/s | p50 | p95 | p99 | p99.9 | Physical reads | Peak sampled RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Full prewalk | 29,812 | 131.1 µs | 217.1 µs | 294.9 µs | 557.1 µs | 7.77 GB | 1.41 GB |
| File-scoped cache reduction | 28,554 | 135.2 µs | 221.2 µs | 303.1 µs | 606.2 µs | 8.35 GB | 0.72 GB |

Each trial performed one million reads, returned no missing expected keys,
and passed a separate full reopened verification of all nine million keys.
Population plus prewalking took 466 seconds; verification plus advisory cache
reduction took 112 seconds before the second trial. Both used the same prepared
database. No system-wide cache clearing or root operation was performed.

The populated database had 1,387 SSTs: L0=2, L1=281, L2=72, L3=792, L4=240.
Total descriptors stayed at 140, including the benchmark outputs and the
configured 128-SST descriptor limit. Actual probes were two L0 tables and about
one lower-level table per get; metadata range filtering still walks the table
list. There is no observed explosion in actual SST probes here, but this does
not prove that metadata scanning is negligible.

The prewalked trial cannot be described as fully hot: the dataset exceeds RAM.
Both trials performed substantial physical reads. These observations exclude
a page-cache-only result, but do not isolate CPU, file-cache locking and storage
waits as causes of the remaining read ceiling. There is one trial per cache
condition, so the 4.2% throughput difference is an observation, not a robust
cache-effect estimate. Resources are sampled during foreground/drain, not a
proof of maximum preparation-time memory.

## 100M sustained writes — measured

The separate preloaded-one-million-key run performed **100,000,000 writes** in
616.538 seconds: **162,196 writes/s**. Full reopened verification passed for all
one million keys. Write p50/p95/p99/p99.9 were 2.240/27.648/55.296/286.720 µs.
The worst PUT was **1.533 seconds**, so rare long foreground stalls remain.
This plan preloads the complete key space; do not interpret its difference from
the unpreloaded 10M trials as an isolated operation-count regression.

Peak sampled L0 was eight files, compaction debt 137.95 MB, RSS 331.76 MB,
database directory size 501.60 MB and total descriptors 179. It completed 1,015
compactions, reading 105.14 GB and writing 89.80 GB of compaction output. Total
SST write amplification was 7.73x. Compaction BUILD accumulated 506.94 seconds;
aggregate L0 writer waiting accumulated 1,206.96 seconds across workers.

Final L0 and debt were zero, with no pending flush/running maintenance. The
bounded samples and verified final state are measured results. L0 waiting is a
confirmed limiting mechanism because explicit writer-pressure counters record
it; attributing every long PUT to a particular CPU or I/O phase would need
additional correlation. The checksum fix does not remove this ceiling.

## Hour-long transitions and concurrency — measured

The one-hour run completed 1,033,815,726 operations, including 428,446,039
writes and 605,369,687 reads. Each of the three phases ran for 20 minutes:
90%-write steady state, all-write burst, then 95%-read recovery. Full reopened
verification of all one million keys passed. Within-phase first/last window
throughput changed by -1.3%, -0.5% and -0.2% respectively. Compaction debt first
reached zero about three seconds into recovery. Recovery RSS window medians rose
from 256.1 to 313.8 MB; samples cycle, and these observations neither demonstrate
nor rule out a leak. The overnight plan is prepared but has not been run.

The ten-minute transition from one to 16 threads also passed full reopened
verification, but **failed the performance-stability objective**. During its
five-minute high-concurrency phase, first/last window throughput fell from
289,911 to 187,553 ops/s (-35.3%). Median interval read p99 increased from
167.936 to 311.296 microseconds (+85.4%); write p99 increased from 120.832 to
167.936 microseconds (+39%). These are interval statistics, not pooled phase
percentiles. This cliff remains unresolved. Compaction pressure and synchronization
are hypotheses requiring correlated profiling, not confirmed root causes.

The separately named concurrency matrix completed all 42 trials (three per
build/thread count), with full reopened verification and settled maintenance:

| Threads | Baseline median ops/s | Candidate median ops/s |
| --- | ---: | ---: |
| 1 | 211,866 | 265,162 |
| 2 | 321,061 | 352,145 |
| 4 | 270,297 | 278,963 |
| 8 | 227,092 | 242,460 |
| 16 | 221,619 | 236,098 |
| 32 | 214,773 | 224,438 |
| 64 | 210,715 | 212,872 |

Two threads were the measured throughput peak under these conditions. Absolute
rates for both builds were substantially below the earlier matched short campaign.
A later host inspection found battery discharge and the powersave governor;
that inspection does not establish when power conditions changed or explain the
within-run decline. Shared-host/power effects are possible confounders. No global
power setting was changed. The full matrix and transition results must not be
pooled with the earlier primary campaign or presented as isolated engine effects.
Raw windows and trends are retained under `foreground-results/endurance`.

## Correctness and compatibility

Formatting and ordinary/Loom/diagnostic Clippy passed. Ordinary and diagnostic
test runs each passed 266 library tests and four integration tests. Eight example
tests, three Loom models and both existing ignored release stress tests passed.
Packaging and package listing passed. Comparison-wrapper Clippy/tests and Python
supervisor tests passed against the existing pinned RocksDB native archives.
The first wrapper attempt was interrupted to avoid a redundant C++ rebuild;
that attempt and the successful replacement are both recorded.

A second post-documentation gate passed formatting, all three Clippy variants,
ordinary/diagnostic/example tests, Loom and ignored release stress. Strict Rustdoc
initially rejected public links to private implementation items. Commit `8a3de58`
replaced those links with code spans; the same strict Rustdoc command then passed.
The initial failure and successful correction are retained.

The fresh deterministic torture campaign passed all durability transition probes,
1,000 simulated cycles (414,831 completed mutations, 341 injected faults), and
1,000 SIGKILL/reopen cycles (532,576 acknowledged mutations). Snapshots were
enabled. This is testing evidence, not formal proof or a model of every power
failure and device behavior. Logs and commands are in `foreground-results/checks`.

## Remaining limits

Normal performance trials buffer writes and sync at the end; they are not
per-operation durable throughput. Torture separately tests acknowledged durable
state. Timing-dependent operation interleavings are not made deterministic by
a fixed seed; keys and intended operation streams are deterministic. The host
is a shared desktop with 22 logical CPUs; 32/64 workers oversubscribe it.

No new worker pool, WAL pipeline, read-visibility shortcut, format change or
snapshot-retention change was introduced. Eight counter slots can still collide
at high concurrency. The cache's membership lock remains shared. The
`max_open_table_files` setting bounds the foreground file cache, not every
engine descriptor: compaction deliberately opens separate input handles to
avoid file-cache starvation/deadlock. Its existing independent-handle regression
documents this behavior; total descriptor usage also depends on compaction
fan-in. The option documentation now states that scope accurately. A true engine-wide
descriptor budget is architectural debt, not implemented by this pass. Existing
limitations concerning MANIFEST checksums, conservative snapshot retention,
independent engine owners opening one directory (including snapshots carried
across independent reopenings), and OS/device fault coverage
remain. No fresh result establishes superiority over RocksDB.
