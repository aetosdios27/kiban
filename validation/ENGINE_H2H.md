# Engine head-to-head: Kiban, Helix and raw LMDB

The N=10,000 study is complete: five alternating trials per engine and mode,
1,500,000 timed calls, 630,000 timed writes with immediate full-value read-back,
and 600,000 final records checked again after reopen. All checks passed.
The base population was verified twice before timing as well.

**Durable: measured on a shared desktop, pinned cores. Fast: preliminary.**
All durable trial medians were below 3. The first fast triple sampled one-minute
load 3.08, above the director's median threshold; its trials remain in the table.
The other fast trials passed. No trial was dropped or selectively rerun.
The complete per-trial load table follows the measurements.

**Large-N Helix is memory-blocked in this pinned version/workload.** A bounded
calibration and independent HTTP reproduction establish growth outside the
in-process harness; see the evidence below. Its large-N comparison cells remain
unmeasured. The three-trial Kiban/LMDB large-N subset completed with zero value errors; both modes are preliminary because later median loads exceeded 3.
No Helix patch or substitute query supplies published numbers.

## Interpretation of the small dataset

Kiban's durable remember/update p50 medians were 524/557 µs, versus Helix's
2,425/1,245 µs and raw LMDB's 1,278/1,278 µs. Kiban also had lower median p99
write latency in this campaign. The configured durability guarantees match;
the persistence operations differ. Kiban appends a WAL record and syncs that
WAL. LMDB commits copy-on-write pages and synchronized metadata. The separate
excluded sync diagnostic confirms these paths; this is more specific than
attributing the difference to an LSM versus a B+tree in general.

Kiban loses to raw LMDB on recent-for-agent p50: 2.75 versus 2.11 µs in durable
mode, with non-overlapping trial ranges. Point-read p50 is effectively tied
at 0.80 µs, with overlapping ranges. Helix's recent p50 is 156 µs; its faithful
query traverses and materializes that agent's memories, sorts by creation
sequence, then takes K. Both KV engines seek directly into a newest-first key
range and decode K values. This structural work difference is visible in the
query source, rather than inferred from HTTP latency.

In the preliminary fast results, Kiban's median insert/update p50 is lower
than both competitors. Raw LMDB retains the recent-scan advantage. Point-read
ranges overlap, and some Kiban maximum-latency observations are worse than
LMDB's; the tail columns are retained without selecting favorable trials.
Fast verified throughput includes generator/oracle work and is substantially
lower than reciprocal API-call throughput. It is not an engine capacity test.

RSS counts mapped resident pages and the complete process, including bulk
preload and oracle allocations; it is not a heap-only comparison. Kernel file
cache outside a process is excluded. The Helix RSS result needs a version-specific
allocation explanation before making memory-efficiency claims. The on-disk
numbers include all database files after close. Allocated bytes are filesystem
`st_blocks * 512` accounting, not measured device writes.

## Method and layers

Kiban engine source is unchanged from `361305a`; implementation commit
`be1df72` adds this standalone harness. Helix is the requested **v2.3.5**, source
`17e7ecf764aecd553e1f54ca25320d654153a9aa` (internal crate version 1.3.3),
with `heed3 0.22.0` and `lmdb-master3-sys 0.2.5`. This is not Helix v3.
All three run from one release binary using Helix's MiMalloc allocator and a
common upstream Helix profile: opt-level 2, LTO and one codegen unit.

Binary SHA-256: `d3901e63d7dd7b24529c5d2768762ae3d8ff40a6d7439a3d264d40d45e1c84ce`.

The timed Helix entry uses public graph storage and the exact generated
transaction/traversal bodies. Its generated handlers require Request/JSON, so
only those wrappers are replaced by typed inputs and owned benchmark fields.
There is no HTTP server, router, socket, JSON transport, task queue or async
runtime. Graph node/property encoding, UUIDs, required unique indexes, edge
maintenance, per-query Bump arena, traversal/sort and transaction commit remain.
The [layer statement and generated-code equivalence](../tools/agentbench/engine/README.md#included-and-excluded-layers)
include the original query excerpt and a machine-checkable source extraction.

Unused automatic BM25 indexing is explicitly disabled; W1–W5 do not query
full-text search. Schema, required indexes and query bodies are preserved.
The old HTTP study kept BM25 enabled and used different compiler/allocator
settings, so its measurements are not pooled or subtracted here. Kiban and raw
LMDB share the same ordered primary key and JSON persistent-value codec.
That JSON is storage encoding, not transport.

| Mode | Kiban | Helix and raw LMDB |
| --- | --- | --- |
| durable | Default options; put + sync per timed write (WAL sync_data/fdatasync) | One synchronous transaction commit per call; flags 0x0 |
| fast | Default options; put without per-write sync; maintenance can still sync | NO_SYNC + NO_META_SYNC; actual flags 0x50000 |

Helix exposes its environment through the public storage API; enabling fast
flags does not patch Helix or claim a corresponding server configuration switch.
Both LMDB environments use a 10 GiB map, 200 database slots and 200 reader slots;
WRITEMAP and MAP_ASYNC are absent. Synchronous LMDB commit flushes dirty data and
writes metadata through an O_DSYNC descriptor. This is per-call persistence,
not one identical fsync syscall per engine. Explicit final sync/flush is outside
foreground metrics. Fast reopen checks prove orderly persistence after that
final sync; no power-loss durability claim is made.

Each process warms a separate discarded 100-record database. It then bulk-loads
10,000 base memories untimed (Kiban WriteBatch chunks; Helix/LMDB one transaction,
relaxed flags, explicit final sync), verifies every base value, closes/reopens,
and verifies the base again. Timed W1 inserts fresh IDs 10,000–19,999. W2/W3/W5
use 10,000 calls; W4 overwrites 10,000 records. Final population is **20,000**.
All values, ordered lists and final/reopened state are checked against the same
seed 20261016, 64 agents, K=10 and 200-byte text plus metadata. Warmup, bulk load,
base verification, final full verification and reopen are excluded from the
foreground latency/throughput metrics. The raw resource files retain their times.

The CPU Ultra 9 185H exposes P-core CPUs 0–11. Every engine process is pinned to
CPUs **1–4**, two physical P-cores including their SMT siblings; the driver uses
E/LP-E CPUs **12–21**. Actual process affinity is checked. Host: 22 logical CPUs,
30.8 GiB RAM, Fedora 44 / Linux 7.2.10-200.fc44.x86_64, WD SN740 NVMe with
LUKS/Btrfs/zstd:1, Rust/Cargo 1.98.1. Other desktop processes share the host;
cores are not exclusively reserved and frequency/governor are not fixed.

The director's gate is each trial's median one-minute load **below 3**; maxima
are reported. Driver samples every 200 ms, with actual engine-side boundary
samples for short fast trials. Timing boundaries exclude preload/warmup; the
load interval includes final in-process verification. Gaps above 2.5 seconds
fail coverage. Readback/command/count/flag/affinity errors invalidate a run.
Small N uses five alternating repetitions of Kiban, Helix, LMDB for each mode;
large N uses three alternating Kiban/LMDB trials, with Helix excluded as documented below. Durable W1/W4 are capped at 10,000 calls for large N;
the 100,000-call mixed workload contains 10,000 writes. No million-record
per-write-fsync preload is performed.

[Reproduction and exact layer/method details](../tools/agentbench/engine/README.md).
[Compact raw evidence](../tools/agentbench/results/engine-small-10000/)
contains every stage/config/resource/reopen JSON, commands, source/binary hashes
and byte-exact gzip load log. Full databases, source snapshots and logs remain
under ignored `target/enginebench-results/small-10000`. `engine_audit.py` independently
rechecks counts, flags, affinities, persistence markers, source/binary hashes and
load classification; it passed with zero errors. The earlier HTTP evidence is
retained separately.

Validation passed: five engine Rust tests (including all engines/modes, bulk
preload/reopen and deliberate corruption rejection), four original harness
Rust tests, thirteen Python harness checks, main cargo test (320 unit + 11
integration; five stress tests ignored), format/Clippy, core cargo deny and
site-facts validation. No core crate dependency or source changed.

## Excluded sync diagnostic

Same binary; N=10,000 untimed base, 1,000 calls per measured workload (2,100
timed writes), plus warmup, bulk initialization, verification and close. The C
interposer observes libc fsync/fdatasync and pwrite/pwrite64; it is not a full
syscall or CPU stack tracer. These instrumented runs are not comparison trials.

| Mode | Engine | fsync/fdatasync calls | O_DSYNC/O_SYNC pwrites |
| --- | --- | ---: | ---: |
| durable | Kiban | 2348 | 0 |
| durable | Helix | 2381 | 2378 |
| durable | LMDB | 2315 | 2312 |
| fast | Kiban | 38 | 0 |
| fast | Helix | 7 | 4 |
| fast | LMDB | 5 | 2 |

All observed sync calls succeeded. Remaining fast-mode calls include startup,
final sync and Kiban maintenance; the counts are whole-process observations.

---

## N=10,000, durable

Measured on a shared desktop, pinned cores. Untimed verified/reopened base: 10,000 records. W1 inserts 10,000 fresh records; W2/W3/W5: 10,000 calls each; W4: 10,000 overwrites. Final population: 20,000. 5 alternating trials per engine.

| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| kiban | remember | 524.29 [524.29, 540.67] | 1,114.11 [1,015.81, 3,801.09] | 5,636.10 [2,883.58, 11,272.19] | 25,481.07 [15,995.68, 34,490.95] | 1,791.36 [1,543.26, 1,808.98] | 1,804.07 [1,552.51, 1,820.36] |
| helix | remember | 2,424.83 [1,605.63, 2,555.90] | 13,893.63 [13,369.34, 14,942.21] | 36,700.16 [29,360.13, 60,817.41] | 208,191.92 [116,659.44, 360,405.83] | 340.14 [307.02, 385.64] | 341.32 [308.12, 387.21] |
| lmdb | remember | 1,277.95 [1,245.18, 1,277.95] | 7,077.89 [6,815.74, 8,912.90] | 66,060.29 [44,040.19, 69,206.02] | 178,689.43 [154,417.47, 224,176.07] | 620.96 [588.89, 655.42] | 623.62 [591.28, 658.11] |
| kiban | recall-by-id | 0.80 [0.72, 0.80] | 1.82 [1.66, 1.89] | 4.48 [4.03, 4.86] | 10.29 [7.96, 19.25] | 597,623.36 [575,543.21, 633,207.60] | 1,167,591.81 [1,156,345.55, 1,273,851.27] |
| helix | recall-by-id | 1.28 [1.18, 1.63] | 2.11 [1.89, 2.82] | 5.76 [4.74, 8.70] | 18.89 [9.12, 42.75] | 469,250.37 [365,518.83, 496,164.45] | 768,849.18 [603,305.51, 825,541.66] |
| lmdb | recall-by-id | 0.80 [0.77, 0.82] | 1.41 [1.28, 1.82] | 2.94 [2.62, 6.27] | 10.59 [8.09, 26.79] | 633,337.81 [547,637.78, 647,868.08] | 1,233,970.57 [1,124,769.44, 1,318,899.30] |
| kiban | recent-for-agent | 2.75 [2.62, 2.75] | 5.38 [4.22, 5.63] | 7.68 [5.89, 11.26] | 44.69 [25.72, 51.11] | 147,877.20 [133,580.57, 150,499.74] | 359,499.56 [332,343.64, 371,660.46] |
| helix | recent-for-agent | 155.65 [143.36, 163.84] | 901.12 [319.49, 2,621.44] | 4,456.45 [393.22, 6,946.82] | 11,550.91 [943.53, 18,026.51] | 5,101.06 [4,377.63, 6,428.62] | 5,219.42 [4,462.94, 6,617.04] |
| lmdb | recent-for-agent | 2.11 [2.05, 2.18] | 3.90 [3.26, 4.22] | 7.55 [6.02, 12.80] | 36.64 [25.52, 53.21] | 163,555.45 [158,160.11, 165,733.36] | 468,570.91 [450,141.50, 473,324.50] |
| kiban | update | 557.06 [524.29, 573.44] | 1,114.11 [1,048.58, 1,998.85] | 5,898.24 [2,883.58, 7,340.03] | 22,300.89 [7,081.34, 38,931.86] | 1,730.86 [1,569.71, 1,833.56] | 1,743.82 [1,580.89, 1,847.88] |
| helix | update | 1,245.18 [1,245.18, 1,277.95] | 6,946.82 [6,684.67, 9,699.33] | 27,787.26 [18,874.37, 38,797.31] | 210,684.59 [77,799.90, 263,046.95] | 652.90 [623.89, 657.51] | 657.22 [627.50, 661.56] |
| lmdb | update | 1,277.95 [1,277.95, 1,376.26] | 8,388.61 [7,602.18, 9,175.04] | 41,943.04 [37,748.74, 90,177.54] | 122,197.23 [101,933.70, 313,257.52] | 608.78 [529.67, 626.74] | 611.35 [531.88, 629.52] |
| kiban | mixed-90-10 | 1.70 [1.50, 1.79] | 622.59 [573.44, 704.51] | 1,212.42 [868.35, 3,473.41] | 6,101.70 [5,102.17, 66,094.35] | 16,309.62 [12,632.86, 17,881.68] | 16,760.95 [12,905.23, 18,419.14] |
| helix | mixed-90-10 | 2.56 [2.43, 2.94] | 1,867.78 [1,474.56, 2,359.30] | 7,864.32 [6,160.38, 8,912.90] | 96,063.63 [30,977.79, 187,943.64] | 6,007.92 [5,113.14, 6,431.83] | 6,087.65 [5,172.18, 6,518.49] |
| lmdb | mixed-90-10 | 1.73 [1.57, 1.82] | 1,540.10 [1,507.33, 1,835.01] | 6,815.74 [6,422.53, 8,650.75] | 37,194.00 [33,210.31, 40,329.66] | 6,417.72 [5,919.59, 6,749.19] | 6,496.70 [5,986.64, 6,834.34] |

| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | ---: | ---: | ---: |
| kiban | 23.28 [23.16, 23.36] | 41.19 [40.52, 41.42] | 7.59 [7.59, 7.59] | 7.60 [7.60, 7.60] |
| helix | 833.94 [829.22, 834.02] | 833.94 [829.22, 834.02] | 14.18 [14.18, 14.18] | 14.17 [14.17, 14.17] |
| lmdb | 21.01 [19.73, 21.13] | 21.06 [21.01, 21.21] | 15.77 [15.77, 15.77] | 15.76 [15.76, 15.76] |

## N=10,000, fast

Preliminary: load/coverage gate failed. Untimed verified/reopened base: 10,000 records. W1 inserts 10,000 fresh records; W2/W3/W5: 10,000 calls each; W4: 10,000 overwrites. Final population: 20,000. 5 alternating trials per engine.

| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| kiban | remember | 0.72 [0.70, 0.74] | 5.12 [4.74, 5.38] | 8.45 [7.04, 9.47] | 42.17 [19.74, 174.50] | 482,581.46 [464,720.51, 496,570.07] | 1,051,615.05 [1,000,540.09, 1,086,447.54] |
| helix | remember | 17.92 [17.92, 18.43] | 25.60 [24.58, 27.65] | 34.82 [33.79, 39.94] | 77.83 [69.84, 98.59] | 49,310.08 [48,708.99, 49,464.65] | 54,363.19 [53,804.30, 54,558.15] |
| lmdb | remember | 6.02 [5.76, 6.14] | 10.24 [9.73, 11.78] | 15.10 [14.34, 18.43] | 39.33 [30.61, 373.91] | 127,649.75 [125,468.68, 131,478.72] | 158,456.50 [154,704.55, 162,687.47] |
| kiban | recall-by-id | 0.66 [0.62, 0.69] | 1.57 [1.47, 1.70] | 3.90 [2.75, 5.76] | 7.62 [6.55, 205.75] | 662,870.01 [610,081.19, 696,243.17] | 1,381,464.09 [1,237,492.20, 1,504,688.91] |
| helix | recall-by-id | 1.31 [1.12, 1.34] | 2.11 [1.98, 2.43] | 4.99 [4.35, 7.04] | 8.97 [8.35, 9.86] | 462,913.31 [450,692.14, 501,031.05] | 762,499.66 [739,523.49, 857,998.21] |
| lmdb | recall-by-id | 0.75 [0.62, 0.78] | 1.12 [1.01, 1.25] | 3.26 [1.57, 4.86] | 7.04 [4.34, 9.00] | 641,261.18 [635,911.81, 704,771.90] | 1,339,243.50 [1,286,333.45, 1,548,821.25] |
| kiban | recent-for-agent | 2.69 [2.69, 2.75] | 4.22 [4.22, 4.48] | 7.04 [6.14, 25.60] | 77.05 [14.46, 166.72] | 147,882.50 [142,290.50, 150,509.20] | 362,601.07 [351,088.13, 371,091.47] |
| helix | recent-for-agent | 151.55 [135.17, 155.65] | 483.33 [303.10, 1,507.33] | 1,114.11 [393.22, 3,997.70] | 18,998.27 [519.94, 22,244.53] | 5,812.74 [5,012.95, 6,828.69] | 5,966.45 [5,125.23, 7,035.35] |
| lmdb | recent-for-agent | 2.11 [2.05, 2.11] | 3.46 [3.07, 4.48] | 6.02 [5.89, 10.24] | 27.48 [24.13, 32.12] | 162,354.23 [160,987.83, 165,516.00] | 470,258.72 [462,443.98, 476,611.39] |
| kiban | update | 0.98 [0.94, 1.02] | 5.76 [5.50, 6.27] | 10.24 [8.96, 12.54] | 170.62 [155.52, 489.43] | 384,306.67 [377,664.48, 406,607.52] | 768,523.79 [740,245.60, 824,278.56] |
| helix | update | 7.55 [7.42, 7.94] | 11.52 [10.75, 14.08] | 16.90 [14.59, 28.67] | 173.69 [147.31, 637.64] | 100,227.17 [96,368.61, 103,417.71] | 128,353.60 [123,455.11, 132,807.96] |
| lmdb | update | 6.27 [6.14, 6.27] | 9.98 [9.73, 11.26] | 14.59 [13.57, 18.43] | 26.37 [24.83, 414.26] | 125,824.00 [125,531.94, 129,717.20] | 157,462.09 [156,925.88, 161,868.34] |
| kiban | mixed-90-10 | 0.67 [0.58, 0.74] | 2.02 [1.79, 2.24] | 7.04 [6.14, 10.24] | 14.03 [8.60, 325.12] | 605,246.02 [554,433.75, 647,035.73] | 1,207,243.80 [1,062,663.23, 1,362,378.15] |
| helix | mixed-90-10 | 1.31 [1.18, 1.44] | 8.70 [8.06, 9.47] | 12.80 [11.52, 18.94] | 146.98 [23.00, 176.29] | 304,219.89 [298,696.67, 348,243.87] | 506,544.32 [461,938.06, 532,963.36] |
| lmdb | mixed-90-10 | 0.82 [0.70, 0.83] | 6.78 [6.66, 7.04] | 10.24 [9.22, 11.52] | 19.00 [13.17, 41.27] | 450,931.93 [443,928.82, 472,455.69] | 739,324.91 [727,337.05, 795,751.13] |

| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | ---: | ---: | ---: |
| kiban | 41.59 [40.89, 41.79] | 41.59 [40.89, 41.79] | 7.59 [7.59, 7.59] | 7.60 [7.60, 7.60] |
| helix | 829.01 [828.92, 834.14] | 829.01 [828.92, 834.27] | 14.18 [14.18, 14.18] | 14.17 [14.17, 14.17] |
| lmdb | 20.90 [20.72, 21.42] | 21.19 [20.90, 21.42] | 15.77 [15.77, 15.77] | 15.76 [15.76, 15.76] |

## Load during every trial

Sampled every 200 ms plus actual engine-side boundary samples; median [minimum, maximum]. Warmup is excluded using engine-recorded timestamps; final sync and full in-process verification are included. Engine open is excluded. The separate reopen process is excluded. All three load windows are retained.

| N | Mode | Trial | Engine | 1-minute load | 5-minute load | 15-minute load | Samples / max gap | Gate |
| ---: | --- | ---: | --- | ---: | ---: | ---: | --- | --- |
| 10,000 | durable | 1 | kiban | 2.15 [2.14, 2.37] | 1.76 [1.76, 1.82] | 2.00 [2.00, 2.02] | 61 / 0.20s | pass |
| 10,000 | durable | 1 | helix | 2.61 [2.37, 2.94] | 1.94 [1.82, 1.95] | 2.05 [2.02, 2.06] | 226 / 0.20s | pass |
| 10,000 | durable | 1 | lmdb | 2.53 [2.34, 3.29] | 1.96 [1.95, 2.18] | 2.06 [2.05, 2.12] | 189 / 0.20s | pass |
| 10,000 | fast | 1 | kiban | 3.08 [3.08, 3.08] | 2.16 [2.16, 2.16] | 2.12 [2.12, 2.12] | 3 / 0.12s | preliminary |
| 10,000 | fast | 1 | helix | 3.08 [3.08, 3.08] | 2.16 [2.16, 2.16] | 2.12 [2.12, 2.12] | 14 / 0.20s | preliminary |
| 10,000 | fast | 1 | lmdb | 3.08 [3.08, 3.08] | 2.16 [2.16, 2.16] | 2.12 [2.12, 2.12] | 3 / 0.16s | preliminary |
| 10,000 | durable | 2 | kiban | 2.85 [2.84, 2.94] | 2.14 [2.12, 2.17] | 2.11 [2.11, 2.12] | 61 / 0.20s | pass |
| 10,000 | durable | 2 | helix | 2.62 [2.37, 2.94] | 2.13 [2.10, 2.20] | 2.11 [2.10, 2.13] | 247 / 0.20s | pass |
| 10,000 | durable | 2 | lmdb | 2.51 [2.31, 2.60] | 2.18 [2.16, 2.20] | 2.12 [2.12, 2.13] | 174 / 0.20s | pass |
| 10,000 | fast | 2 | kiban | 2.31 [2.31, 2.31] | 2.16 [2.16, 2.16] | 2.12 [2.12, 2.12] | 3 / 0.12s | pass |
| 10,000 | fast | 2 | helix | 2.31 [2.21, 2.31] | 2.16 [2.14, 2.16] | 2.12 [2.11, 2.12] | 12 / 0.20s | pass |
| 10,000 | fast | 2 | lmdb | 2.21 [2.21, 2.21] | 2.14 [2.14, 2.14] | 2.11 [2.11, 2.11] | 3 / 0.16s | pass |
| 10,000 | durable | 3 | kiban | 2.19 [2.09, 2.21] | 2.13 [2.12, 2.14] | 2.11 [2.10, 2.11] | 67 / 0.20s | pass |
| 10,000 | durable | 3 | helix | 2.09 [1.99, 2.55] | 2.11 [2.09, 2.20] | 2.10 [2.09, 2.13] | 241 / 0.20s | pass |
| 10,000 | durable | 3 | lmdb | 2.51 [2.22, 2.60] | 2.23 [2.15, 2.24] | 2.14 [2.12, 2.15] | 170 / 0.20s | pass |
| 10,000 | fast | 3 | kiban | 2.51 [2.51, 2.51] | 2.24 [2.24, 2.24] | 2.14 [2.14, 2.14] | 3 / 0.13s | pass |
| 10,000 | fast | 3 | helix | 2.51 [2.39, 2.51] | 2.24 [2.21, 2.24] | 2.14 [2.14, 2.14] | 11 / 0.20s | pass |
| 10,000 | fast | 3 | lmdb | 2.39 [2.39, 2.39] | 2.21 [2.21, 2.21] | 2.14 [2.14, 2.14] | 3 / 0.16s | pass |
| 10,000 | durable | 4 | kiban | 2.68 [2.39, 2.71] | 2.28 [2.21, 2.29] | 2.16 [2.14, 2.16] | 66 / 0.20s | pass |
| 10,000 | durable | 4 | helix | 2.77 [2.64, 2.99] | 2.34 [2.31, 2.39] | 2.18 [2.17, 2.20] | 238 / 0.20s | pass |
| 10,000 | durable | 4 | lmdb | 2.70 [2.47, 2.98] | 2.39 [2.35, 2.43] | 2.21 [2.20, 2.22] | 170 / 0.20s | pass |
| 10,000 | fast | 4 | kiban | 2.47 [2.47, 2.47] | 2.35 [2.35, 2.35] | 2.20 [2.20, 2.20] | 3 / 0.13s | pass |
| 10,000 | fast | 4 | helix | 2.47 [2.44, 2.47] | 2.35 [2.35, 2.35] | 2.20 [2.20, 2.20] | 13 / 0.20s | pass |
| 10,000 | fast | 4 | lmdb | 2.44 [2.44, 2.44] | 2.35 [2.35, 2.35] | 2.20 [2.20, 2.20] | 3 / 0.16s | pass |
| 10,000 | durable | 5 | kiban | 2.64 [2.44, 2.91] | 2.39 [2.35, 2.45] | 2.21 [2.20, 2.23] | 68 / 0.20s | pass |
| 10,000 | durable | 5 | helix | 2.93 [2.81, 3.32] | 2.52 [2.51, 2.56] | 2.27 [2.26, 2.28] | 259 / 0.20s | pass |
| 10,000 | durable | 5 | lmdb | 2.76 [2.66, 2.94] | 2.52 [2.51, 2.58] | 2.28 [2.27, 2.30] | 170 / 0.20s | pass |
| 10,000 | fast | 5 | kiban | 2.94 [2.94, 2.94] | 2.58 [2.58, 2.58] | 2.30 [2.30, 2.30] | 3 / 0.12s | pass |
| 10,000 | fast | 5 | helix | 2.94 [2.94, 2.94] | 2.58 [2.58, 2.58] | 2.30 [2.30, 2.30] | 13 / 0.20s | pass |
| 10,000 | fast | 5 | lmdb | 2.94 [2.94, 2.94] | 2.58 [2.58, 2.58] | 2.30 [2.30, 2.30] | 3 / 0.17s | pass |


## RSS and disk after every small-N run

| Mode | Trial | Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| durable | 1 | kiban | 23.35 | 41.42 | 7.59 | 7.60 |
| durable | 1 | helix | 833.98 | 833.98 | 14.18 | 14.17 |
| durable | 1 | lmdb | 19.73 | 21.06 | 15.77 | 15.76 |
| fast | 1 | kiban | 41.59 | 41.59 | 7.59 | 7.60 |
| fast | 1 | helix | 829.01 | 829.01 | 14.18 | 14.17 |
| fast | 1 | lmdb | 20.72 | 21.19 | 15.77 | 15.76 |
| durable | 2 | kiban | 23.28 | 40.52 | 7.59 | 7.60 |
| durable | 2 | helix | 833.94 | 833.94 | 14.18 | 14.17 |
| durable | 2 | lmdb | 21.07 | 21.08 | 15.77 | 15.76 |
| fast | 2 | kiban | 41.02 | 41.02 | 7.59 | 7.60 |
| fast | 2 | helix | 834.14 | 834.14 | 14.18 | 14.17 |
| fast | 2 | lmdb | 20.83 | 21.12 | 15.77 | 15.76 |
| durable | 3 | kiban | 23.16 | 40.60 | 7.59 | 7.60 |
| durable | 3 | helix | 834.02 | 834.02 | 14.18 | 14.17 |
| durable | 3 | lmdb | 21.01 | 21.01 | 15.77 | 15.76 |
| fast | 3 | kiban | 41.61 | 41.61 | 7.59 | 7.60 |
| fast | 3 | helix | 834.07 | 834.27 | 14.18 | 14.17 |
| fast | 3 | lmdb | 20.90 | 21.19 | 15.77 | 15.76 |
| durable | 4 | kiban | 23.36 | 41.21 | 7.59 | 7.60 |
| durable | 4 | helix | 832.10 | 832.10 | 14.18 | 14.17 |
| durable | 4 | lmdb | 21.13 | 21.21 | 15.77 | 15.76 |
| fast | 4 | kiban | 40.89 | 40.89 | 7.59 | 7.60 |
| fast | 4 | helix | 828.97 | 828.97 | 14.18 | 14.17 |
| fast | 4 | lmdb | 21.42 | 21.42 | 15.77 | 15.76 |
| durable | 5 | kiban | 23.26 | 41.19 | 7.59 | 7.60 |
| durable | 5 | helix | 829.22 | 829.22 | 14.18 | 14.17 |
| durable | 5 | lmdb | 21.01 | 21.04 | 15.77 | 15.76 |
| fast | 5 | kiban | 41.79 | 41.79 | 7.59 | 7.60 |
| fast | 5 | helix | 828.92 | 828.92 | 14.18 | 14.17 |
| fast | 5 | lmdb | 20.90 | 20.90 | 15.77 | 15.76 |

## Helix large-N exclusion and memory evidence

This exclusion is specific to Helix **v2.3.5 / 17e7ecf764aecd553e1f54ca25320d654153a9aa**,
this schema and this W3 query. The small-N measurements above are retained unchanged.
The large calibration loaded 1,000,000 base records, inserted 1,000 fresh records,
and ran 1,000 calls per workload with fast flags. All five foreground stage markers
passed value checks, but the 6 GiB guard stopped the process during final
verification, so there is no complete/reopen marker and **no valid comparison
trial**. Its maximum observed RSS was 6,443,012,096 bytes. Calibration W3 took
16.30 seconds for 1,000 requests, projecting roughly 163 minutes for W3 alone
across six 100,000-call trials. That projection is not a measured benchmark cell.

The owner's requested harness check took under ten minutes. The direct adapter's
`recent` function drops its local traversal Vec and Bump arena when returning K
owned records. The common runner consumes/drops the returned `many` Vec inside
each operation; only fixed-size oracles/histograms persist, and logs contain
one row per stage. There is no accumulating response Vec or record log.

A separate Python standard-library client then sent **1,000 POST /Recent**
requests to an independent pinned Helix HTTP process opening a copy of the
1,001,000-record database. Requests used `{"agent": q % 64, "k": 10}`;
all ten IDs, agent/creation metadata, order, revision type and text size were
checked before each response was discarded. Server and client were pinned to
CPUs 1–4 and 12–21 respectively. The client streams one RSS row per request.

| Completed W3 queries | HTTP server RSS MiB | Client RSS MiB |
| ---: | ---: | ---: |
| 0 | 74.45 | 20.25 |
| 64 | 1,200.95 | 20.86 |
| 100 | 1,258.11 | 20.86 |
| 250 | 1,847.06 | 20.86 |
| 500 | 2,810.98 | 20.86 |
| 750 | 3,790.99 | 20.86 |
| 1,000 | 4,765.00 | 20.86 |

The database file was about 673 MiB, so continued multi-GiB growth after repeated
visits to the same agents cannot be explained solely by making that file resident.
HTTP reproduction one-minute load was 2.46 [2.14, 2.58], sampled with every request; raw values are retained.
These RSS observations are diagnostic evidence, **not HTTP latency cells pooled
with the engine study**. They establish retention in Helix's independent server
path, excluding the in-process harness's result handling as its cause.

The stock server's BM25-enabled configuration first attempted to build an unused
full-text index and reached `MDB_MAP_FULL` at its 10 GiB map before serving requests.
The successful reproduction changed only generated application config
`bm25: Some(true)` to `Some(false)`, matching the engine study. It rebuilt a
separate HTTP executable, whose SHA-256 is
`2380637190af2e3d536b6bbe26537665ebe094b2b608db52a6527bdb1cdc284a`.
All pinned engine sources were verified unchanged; the HTTP W3 storage/traversal
core is byte-identical to the frozen adapter. The installed original HTTP
executable/config were restored. The comparison binary was never rebuilt.
The config delta, actual generated HTTP source, version/commit, server logs,
query-count/RSS curve and hashes are [archived separately](../tools/agentbench/results/helix-memory-diagnostic/).

Exact HelixQL:

```text
QUERY Recent(agent: U32, k: U32) =>
    memories <- N<Agent>({agent_num: agent})::Out<Remembers>::ORDER<Desc>(_::{created_at})::RANGE(0, k)
    RETURN memories
```

The likely mechanism is visible in pinned
[`ImmutablePropertiesMap`](https://github.com/HelixDB/helix-db/blob/17e7ecf764aecd553e1f54ca25320d654153a9aa/helix-db/src/utils/properties.rs#L25):
its Copy map stores raw arena-allocated `Value` pointers, populated by
`values.add(index).write(value)`, with no Drop implementation. `Value::String`
owns a String; bump arena teardown does not run those destructors. W3 decodes
roughly 15,640 neighbors per query before sorting/taking ten. The HTTP curve
and source support this explanation; no heap-profiler claim assigns every RSS
byte to that mechanism. Mapped pages and allocator overhead also contribute.
This is a version-specific allocation issue, not an inherent graph storage cost.

The [minimal standalone client and setup instructions](../tools/agentbench/engine/README.md#helix-memory-growth-repro)
are suitable for an upstream reproduction. Helix is memory-blocked in both large-N
mode tables: only fast mode was calibrated, but the read-only W3 allocation path
is shared with durable mode. No durable large-N Helix trial is implied.

## Large-N Kiban/LMDB results

All 12 large-N subset trials completed: 4,920,000 foreground calls, 1,440,000
timed writes with immediate full-value read-back, 24,000,000 base-value checks
before timing, and 12,660,000 final records checked after a new-process reopen.
The recorded campaign interval was about **5.7 minutes**, including bulk loads
and reopen processes. The frozen executable/hash is identical to small N;
no benchmark rebuild occurred. Durable W1/W4 each use 10,000 timed writes;
W2/W3/W5 use 100,000 calls. Fast uses 100,000 for all five workloads.
The different final populations are explicit beside each table.

Both modes are **preliminary, measured on a shared desktop with pinned cores**.
First-cycle medians passed; cycles two and three did not. Every trial is retained,
with median one-minute load 2.53–4.07 and maximum 4.07. Sampling coverage passed
for all trials. CPU sets, seed, warmup and verification rules match small N.
Helix has no valid large-N trial; its unmeasured cells below are memory-blocked,
not a latency extrapolation or a substitute storage path.

Kiban loses the large-N point-read comparison to raw LMDB: durable p50 medians
are 5.89 versus 1.31 µs, and fast 5.63 versus 1.38 µs, with non-overlapping
trial ranges. LMDB also has the lower recent-list p50 and better fast mixed
verified throughput (196,638 versus 160,223 ops/s). With this resident dataset,
LMDB's mmap reads avoid Kiban's SST/block-cache lookup and decoding path; the
larger dataset exceeds Kiban's default 32 MiB block cache. That is a plausible
structural explanation, not a CPU-profile attribution or a cold-cache result.

Kiban has lower remember/update p50 in both modes here. Durable medians are
541/541 µs versus LMDB's 1,180/1,311 µs. Kiban synchronizes an appended WAL
record, while LMDB's single-writer copy-on-write commit persists modified pages
and metadata. Fast mode removes per-write durability from both: Kiban defers
flush/compaction work and LMDB still performs a transaction commit/page update.
These are foreground acknowledgment latencies, not sustained compaction-drained
write capacity. The tables retain max latency and throughput: durable mixed
p50 favors LMDB's majority reads while total verified throughput favors Kiban's
less costly writes. Fast mixed throughput favors LMDB.

Kiban's lower end RSS does not establish lower peak memory. Whole-process peak
RSS during preload/work was roughly **1,614 MiB for Kiban versus 1,329 MiB for
LMDB**, including the bulk path, oracle, allocator and mapped pages. Fast Kiban
end RSS varied from 270 to 955 MiB; that full range is retained. LMDB files
were larger after these operations, but neither engine was force-compacted or
vacuumed to a common minimum-size state. No general engine winner is claimed.

[Compact large-N evidence](../tools/agentbench/results/engine-large-1000000-kv/)
contains all stage/config/resource/reopen JSON, command/source/binary hashes,
compressed load samples and an independent zero-error audit. Full source
snapshots/databases/logs remain at `target/enginebench-results/large-1000000-kv`.

## N=1,000,000, durable

Preliminary: load/coverage gate failed. Untimed verified/reopened base: 1,000,000 records. W1 inserts 10,000 fresh records; W2/W3/W5: 100,000 calls each; W4: 10,000 overwrites. Final population: 1,010,000. 3 alternating trials per engine.

| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| kiban | remember | 540.67 [524.29, 540.67] | 1,310.72 [1,114.11, 1,409.02] | 2,883.58 [2,818.05, 2,883.58] | 15,107.95 [9,027.25, 30,905.99] | 1,792.03 [1,777.62, 1,809.90] | 1,807.09 [1,789.95, 1,822.89] |
| lmdb | remember | 1,179.65 [1,179.65, 1,179.65] | 3,538.94 [3,407.87, 4,456.45] | 13,107.20 [10,485.76, 13,631.49] | 57,207.05 [24,579.55, 77,914.79] | 770.12 [745.51, 778.04] | 774.23 [749.06, 782.01] |
| helix | remember | memory-blocked | — | — | — | — | — |
| kiban | recall-by-id | 5.89 [5.76, 6.66] | 9.47 [8.96, 10.24] | 15.62 [15.36, 22.53] | 479.33 [478.77, 531.96] | 152,716.41 [136,296.72, 153,242.94] | 178,209.81 [158,590.08, 179,069.77] |
| lmdb | recall-by-id | 1.31 [1.28, 1.41] | 2.18 [2.11, 2.18] | 5.50 [4.61, 5.76] | 26.76 [17.41, 28.69] | 447,531.72 [421,556.64, 467,192.64] | 742,983.01 [705,038.52, 785,098.79] |
| helix | recall-by-id | memory-blocked | — | — | — | — | — |
| kiban | recent-for-agent | 3.07 [2.88, 3.07] | 5.25 [4.86, 5.38] | 11.52 [10.24, 12.29] | 49.80 [46.55, 150.01] | 137,668.42 [132,823.01, 141,067.60] | 320,473.79 [319,386.99, 339,257.00] |
| lmdb | recent-for-agent | 2.18 [2.11, 2.56] | 3.71 [3.58, 4.86] | 6.78 [5.89, 9.73] | 32.14 [32.00, 51.92] | 159,189.63 [131,336.19, 161,354.69] | 457,129.00 [372,313.06, 467,124.07] |
| helix | recent-for-agent | memory-blocked | — | — | — | — | — |
| kiban | update | 540.67 [524.29, 540.67] | 2,293.76 [1,736.70, 2,883.58] | 12,058.62 [6,422.53, 12,058.62] | 35,643.40 [15,223.99, 62,311.28] | 1,644.15 [1,568.85, 1,668.32] | 1,657.28 [1,580.00, 1,679.66] |
| lmdb | update | 1,310.72 [1,277.95, 1,376.26] | 11,534.34 [2,818.05, 12,058.62] | 33,030.14 [9,175.04, 33,554.43] | 174,432.62 [157,101.15, 220,275.10] | 580.18 [533.12, 703.58] | 582.59 [535.36, 707.91] |
| helix | update | memory-blocked | — | — | — | — | — |
| kiban | mixed-90-10 | 8.96 [7.94, 9.47] | 622.59 [622.59, 655.36] | 1,114.11 [1,114.11, 1,212.42] | 9,039.38 [6,143.68, 18,395.93] | 15,062.68 [14,493.03, 15,368.40] | 15,445.63 [14,858.37, 15,729.67] |
| lmdb | mixed-90-10 | 2.37 [2.30, 2.62] | 2,490.37 [2,293.76, 2,555.90] | 12,582.91 [11,272.19, 12,845.06] | 180,247.93 [168,263.84, 508,508.04] | 4,690.18 [4,358.31, 4,957.22] | 4,734.65 [4,395.62, 5,011.33] |
| helix | mixed-90-10 | memory-blocked | — | — | — | — | — |

| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | ---: | ---: | ---: |
| kiban | 100.88 [85.20, 106.33] | 1,613.90 [1,607.70, 1,615.82] | 323.91 [323.91, 323.91] | 323.92 [323.92, 323.92] |
| lmdb | 814.50 [814.44, 814.57] | 1,329.32 [1,329.29, 1,329.43] | 807.46 [807.46, 807.46] | 807.45 [807.45, 807.45] |
| helix | memory-blocked | — | — | — |

## N=1,000,000, fast

Preliminary: load/coverage gate failed. Untimed verified/reopened base: 1,000,000 records. W1 inserts 100,000 fresh records; W2/W3/W5: 100,000 calls each; W4: 100,000 overwrites. Final population: 1,100,000. 3 alternating trials per engine.

| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| kiban | remember | 0.74 [0.72, 0.78] | 5.25 [5.12, 5.63] | 10.50 [9.73, 12.29] | 194.47 [182.54, 305.75] | 453,601.57 [421,710.71, 463,941.42] | 970,477.28 [897,401.22, 998,253.27] |
| lmdb | remember | 7.68 [7.42, 7.68] | 12.03 [11.52, 12.29] | 16.38 [14.85, 18.94] | 290.62 [44.82, 298.88] | 104,966.95 [104,512.42, 108,445.50] | 125,721.54 [125,293.45, 130,321.75] |
| helix | remember | memory-blocked | — | — | — | — | — |
| kiban | recall-by-id | 5.63 [5.50, 6.27] | 7.68 [7.42, 8.70] | 12.29 [11.78, 14.08] | 456.44 [447.02, 474.84] | 167,169.37 [150,355.83, 170,215.24] | 196,770.44 [176,485.69, 200,647.14] |
| lmdb | recall-by-id | 1.38 [1.34, 1.41] | 1.86 [1.86, 1.95] | 4.86 [4.48, 4.99] | 25.11 [9.29, 75.32] | 434,591.65 [430,304.37, 453,514.89] | 722,342.33 [703,805.81, 747,439.01] |
| helix | recall-by-id | memory-blocked | — | — | — | — | — |
| kiban | recent-for-agent | 2.94 [2.94, 3.07] | 5.50 [4.99, 6.14] | 9.73 [9.73, 10.50] | 67.29 [62.27, 75.49] | 139,904.47 [132,881.71, 140,174.04] | 333,037.68 [319,171.26, 333,192.00] |
| lmdb | recent-for-agent | 2.11 [2.11, 2.18] | 3.46 [3.46, 3.78] | 6.14 [6.02, 6.27] | 44.90 [32.76, 74.61] | 159,978.44 [156,597.70, 160,140.19] | 461,775.82 [451,491.76, 463,696.64] |
| helix | recent-for-agent | memory-blocked | — | — | — | — | — |
| kiban | update | 1.54 [1.34, 1.57] | 6.66 [6.02, 6.78] | 11.78 [10.24, 13.06] | 262.45 [138.44, 278.07] | 310,405.84 [301,914.12, 346,074.75] | 546,321.76 [541,896.23, 640,245.20] |
| lmdb | update | 18.94 [17.41, 19.97] | 53.25 [49.15, 67.58] | 1,277.95 [966.66, 1,507.33] | 69,041.95 [68,594.14, 217,997.95] | 31,409.81 [28,816.91, 33,451.88] | 34,251.47 [31,024.45, 36,228.75] |
| helix | update | memory-blocked | — | — | — | — | — |
| kiban | mixed-90-10 | 6.40 [6.14, 7.17] | 9.22 [8.19, 10.50] | 14.08 [13.06, 16.90] | 300.23 [84.75, 653.12] | 160,223.18 [145,391.62, 169,229.76] | 189,821.46 [171,330.08, 201,042.24] |
| lmdb | mixed-90-10 | 1.79 [1.63, 1.86] | 29.70 [27.65, 31.74] | 104.45 [102.40, 118.78] | 3,160.58 [1,278.84, 17,689.74] | 196,638.37 [185,422.13, 201,830.46] | 262,581.64 [249,469.68, 263,528.18] |
| helix | mixed-90-10 | memory-blocked | — | — | — | — | — |

| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | ---: | ---: | ---: |
| kiban | 284.42 [269.75, 955.00] | 1,604.51 [1,604.43, 1,612.86] | 376.14 [376.14, 376.14] | 376.16 [376.16, 376.16] |
| lmdb | 886.38 [886.29, 886.43] | 1,329.46 [1,329.19, 1,329.52] | 879.32 [879.32, 879.32] | 879.31 [879.31, 879.31] |
| helix | memory-blocked | — | — | — |

## Load during every trial

Sampled every 200 ms plus actual engine-side boundary samples; median [minimum, maximum]. Warmup is excluded using engine-recorded timestamps; final sync and full in-process verification are included. Engine open is excluded. The separate reopen process is excluded. All three load windows are retained.

| N | Mode | Trial | Engine | 1-minute load | 5-minute load | 15-minute load | Samples / max gap | Gate |
| ---: | --- | ---: | --- | ---: | ---: | ---: | --- | --- |
| 1,000,000 | durable | 1 | kiban | 2.72 [2.40, 2.79] | 2.28 [2.20, 2.29] | 2.14 [2.12, 2.15] | 106 / 0.27s | pass |
| 1,000,000 | durable | 1 | lmdb | 2.53 [2.42, 2.72] | 2.29 [2.24, 2.32] | 2.15 [2.13, 2.16] | 276 / 0.20s | pass |
| 1,000,000 | fast | 1 | kiban | 2.92 [2.77, 2.92] | 2.40 [2.38, 2.40] | 2.19 [2.19, 2.19] | 26 / 0.20s | pass |
| 1,000,000 | fast | 1 | lmdb | 2.79 [2.79, 3.36] | 2.39 [2.39, 2.51] | 2.19 [2.19, 2.23] | 37 / 0.28s | pass |
| 1,000,000 | durable | 2 | kiban | 3.33 [3.07, 3.53] | 2.57 [2.48, 2.60] | 2.26 [2.23, 2.27] | 110 / 0.21s | preliminary |
| 1,000,000 | durable | 2 | lmdb | 3.73 [3.32, 3.93] | 2.75 [2.60, 2.84] | 2.33 [2.27, 2.37] | 248 / 0.20s | preliminary |
| 1,000,000 | fast | 2 | kiban | 3.38 [3.38, 3.51] | 2.78 [2.78, 2.82] | 2.36 [2.36, 2.37] | 26 / 0.20s | preliminary |
| 1,000,000 | fast | 2 | lmdb | 4.07 [3.55, 4.07] | 2.96 [2.84, 2.96] | 2.42 [2.38, 2.42] | 36 / 0.23s | preliminary |
| 1,000,000 | durable | 3 | kiban | 3.64 [3.59, 3.79] | 2.95 [2.90, 2.95] | 2.43 [2.41, 2.44] | 112 / 0.22s | preliminary |
| 1,000,000 | durable | 3 | lmdb | 3.71 [3.37, 4.03] | 3.04 [2.98, 3.10] | 2.50 [2.45, 2.52] | 279 / 0.20s | preliminary |
| 1,000,000 | fast | 3 | kiban | 3.60 [3.60, 3.83] | 3.11 [3.11, 3.14] | 2.53 [2.53, 2.54] | 24 / 0.20s | preliminary |
| 1,000,000 | fast | 3 | lmdb | 3.59 [3.55, 3.70] | 3.12 [3.10, 3.15] | 2.54 [2.54, 2.56] | 35 / 0.21s | preliminary |


## RSS and disk after every large-N run

| Mode | Trial | Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| durable | 1 | kiban | 106.33 | 1,615.82 | 323.91 | 323.92 |
| durable | 1 | lmdb | 814.57 | 1,329.32 | 807.46 | 807.45 |
| fast | 1 | kiban | 284.42 | 1,612.86 | 376.14 | 376.16 |
| fast | 1 | lmdb | 886.29 | 1,329.19 | 879.32 | 879.31 |
| durable | 2 | kiban | 85.20 | 1,613.90 | 323.91 | 323.92 |
| durable | 2 | lmdb | 814.50 | 1,329.43 | 807.46 | 807.45 |
| fast | 2 | kiban | 269.75 | 1,604.51 | 376.14 | 376.16 |
| fast | 2 | lmdb | 886.43 | 1,329.46 | 879.32 | 879.31 |
| durable | 3 | kiban | 100.88 | 1,607.70 | 323.91 | 323.92 |
| durable | 3 | lmdb | 814.44 | 1,329.29 | 807.46 | 807.45 |
| fast | 3 | kiban | 955.00 | 1,604.43 | 376.14 | 376.16 |
| fast | 3 | lmdb | 886.38 | 1,329.52 | 879.32 | 879.31 |


## Large-N read diagnosis: cache budget

**Cache budget explains most of the observed point-read gap, with a residual that needs profiling.** The contemporaneous W2 p50 gap narrowed from 4.35 µs (32 MiB) to 0.70 µs (512 MiB): about **84% closure** (80.6–85.9% when computed within each alternating cycle). At 1 GiB the residual was 0.58 µs, about 87% closure. These are differences between p50 medians, not an additive breakdown of read work.

**Preliminary:** 8 of 15 trials exceeded the <3 timed-median load gate; all remain included. Three cycles alternate LMDB, K32, K256, K512, K1024. N=1,000,000; seed 20261016; 100,000 calls per stage; benchmark CPUs 1–4, driver 12–21. All options except block-cache capacity remain default: range cache off, file cache 128, read-ahead 256 KiB, mmap off.

This is read-only and uses fast mode: reads pay no per-call fsync. W5 retains its stage-4 key sequence with writes converted to reads, explicitly **mixed-key-reads-only**, rather than 90/10. The original bulk-load/sync/verify/reopen/verify method warms caches; every revision stays zero and no W1 inserts expand the population. Compare against this sweep’s control/reference, rather than subtracting the older post-update measurements.

Cells are median [minimum, maximum] across three trials. RSS is the whole direct benchmark process after reads.

| Config | W2 p50 µs | W3 p50 µs | W5-key p50 µs | RSS MiB | W2 replay hit % |
| --- | ---: | ---: | ---: | ---: | ---: |
| LMDB | 1.41 [1.34, 1.44] | 2.37 [2.18, 2.43] | 1.38 [1.38, 1.44] | 806.76 [806.60, 806.89] | not instrumented |
| Kiban 32 MiB | 5.76 [5.63, 5.76] | 3.01 [2.94, 3.07] | 5.63 [5.50, 5.89] | 103.42 [91.61, 119.29] | 10.29 [10.29, 10.29] |
| Kiban 256 MiB | 2.50 [2.37, 2.75] | 3.33 [3.14, 3.39] | 2.50 [2.37, 2.50] | 416.86 [416.48, 448.15] | 82.79 [82.79, 82.79] |
| Kiban 512 MiB | 2.11 [2.05, 2.18] | 3.20 [3.20, 3.26] | 2.18 [2.11, 2.18] | 486.84 [486.25, 520.87] | 100.00 [100.00, 100.00] |
| Kiban 1024 MiB | 1.98 [1.95, 2.05] | 3.26 [3.20, 3.26] | 2.02 [1.89, 2.24] | 504.45 [489.75, 536.27] | 100.00 [100.00, 100.00] |

Every Kiban preload produced **one L0 SST and no other levels**, unchanged throughout. The separate replay’s resident cache was 32.00/256.00/309.14/309.14 MiB. Direct Kiban exposes no public cache counters, so the hit rates above are **proxy counters from an excluded SharedKiban replay**: fresh cache, identical files, full verification warmup, identical keys, all values checked. Its time/RSS is excluded. W3 already had 99.94% replay hits at 32 MiB; larger caches did not improve its latency.

The block working set fits at 512 MiB, yet point reads remain slower than LMDB with lower end RSS. More capacity alone does not account for the remaining gap. Profile SST filter/index/restart-block lookup, cache locking and value materialization next. Source shows Kiban copies a found value into a Vec before JSON decoding; LMDB decodes directly from its mmap-backed slice. Both return the same owned fields. No profile here assigns the residual to those candidates. Median peak RSS includes preload: about 1,614–1,617 MiB for Kiban versus 1,329 MiB for LMDB.

One new diagnostic executable (`430dd88d…`) served all configurations; the original frozen binary (`d3901e63…`) remains unchanged. No core source/dependency changed. All **4.5 million timed calls** and 15 million records checked after independent reopen passed, with zero timed writes.

[Full percentiles, max, throughput, RSS/peak/disk, counters and raw evidence](../tools/agentbench/results/cache-budget-1000000/REPORT.md); [one-command reproduction and detailed method](../tools/agentbench/engine/README.md#large-n-cache-budget-diagnosis). Full binary/source hashes are archived.

| Trial | Config | Timed 1-minute load median [min, max] | Gate |
| ---: | --- | ---: | --- |
| 1 | LMDB | 1.56 [1.56, 1.56] | pass |
| 1 | Kiban 32 MiB | 1.63 [1.63, 1.63] | pass |
| 1 | Kiban 256 MiB | 2.10 [2.10, 2.10] | pass |
| 1 | Kiban 512 MiB | 2.27 [2.27, 2.27] | pass |
| 1 | Kiban 1024 MiB | 3.47 [3.47, 3.47] | preliminary |
| 2 | LMDB | 3.32 [3.32, 3.32] | preliminary |
| 2 | Kiban 32 MiB | 3.28 [3.28, 3.34] | preliminary |
| 2 | Kiban 256 MiB | 3.06 [3.06, 3.06] | preliminary |
| 2 | Kiban 512 MiB | 2.90 [2.90, 2.90] | pass |
| 2 | Kiban 1024 MiB | 2.85 [2.78, 2.85] | pass |
| 3 | LMDB | 3.04 [2.96, 3.04] | preliminary |
| 3 | Kiban 32 MiB | 3.04 [2.96, 3.04] | preliminary |
| 3 | Kiban 256 MiB | 2.73 [2.73, 2.73] | pass |
| 3 | Kiban 512 MiB | 3.12 [3.12, 3.12] | preliminary |
| 3 | Kiban 1024 MiB | 3.02 [2.94, 3.02] | preliminary |

All timed-interval sampling coverage checks passed (200 ms plus engine-side boundaries). Bulk load, full verification and counter replay are excluded from that interval; their recent activity can contribute to system load. Process-wide raw load samples are retained too.
