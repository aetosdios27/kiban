# In-process engine head-to-head

**PRELIMINARY — at least one per-trial median load gate or coverage check failed.**

Benchmark CPUs: `1-4`; driver CPUs: `12-21`. These affinities do not reserve the cores exclusively.

Cells are median [minimum, maximum] of per-trial metrics (five small-N trials, three large-N trials). Latencies are µs. The median 1-minute load must be <3 in every trial; maxima are reported, not gated.

## N=1,000,000, durable

Preliminary: load/coverage gate failed. Untimed verified/reopened base: 1,000,000 records. W1 inserts 10,000 fresh records; W2/W3/W5: 100,000 calls each; W4: 10,000 overwrites. Final population: 1,010,000. 3 alternating trials per engine.

| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| kiban | remember | 540.67 [524.29, 540.67] | 1,310.72 [1,114.11, 1,409.02] | 2,883.58 [2,818.05, 2,883.58] | 15,107.95 [9,027.25, 30,905.99] | 1,792.03 [1,777.62, 1,809.90] | 1,807.09 [1,789.95, 1,822.89] |
| lmdb | remember | 1,179.65 [1,179.65, 1,179.65] | 3,538.94 [3,407.87, 4,456.45] | 13,107.20 [10,485.76, 13,631.49] | 57,207.05 [24,579.55, 77,914.79] | 770.12 [745.51, 778.04] | 774.23 [749.06, 782.01] |
| kiban | recall-by-id | 5.89 [5.76, 6.66] | 9.47 [8.96, 10.24] | 15.62 [15.36, 22.53] | 479.33 [478.77, 531.96] | 152,716.41 [136,296.72, 153,242.94] | 178,209.81 [158,590.08, 179,069.77] |
| lmdb | recall-by-id | 1.31 [1.28, 1.41] | 2.18 [2.11, 2.18] | 5.50 [4.61, 5.76] | 26.76 [17.41, 28.69] | 447,531.72 [421,556.64, 467,192.64] | 742,983.01 [705,038.52, 785,098.79] |
| kiban | recent-for-agent | 3.07 [2.88, 3.07] | 5.25 [4.86, 5.38] | 11.52 [10.24, 12.29] | 49.80 [46.55, 150.01] | 137,668.42 [132,823.01, 141,067.60] | 320,473.79 [319,386.99, 339,257.00] |
| lmdb | recent-for-agent | 2.18 [2.11, 2.56] | 3.71 [3.58, 4.86] | 6.78 [5.89, 9.73] | 32.14 [32.00, 51.92] | 159,189.63 [131,336.19, 161,354.69] | 457,129.00 [372,313.06, 467,124.07] |
| kiban | update | 540.67 [524.29, 540.67] | 2,293.76 [1,736.70, 2,883.58] | 12,058.62 [6,422.53, 12,058.62] | 35,643.40 [15,223.99, 62,311.28] | 1,644.15 [1,568.85, 1,668.32] | 1,657.28 [1,580.00, 1,679.66] |
| lmdb | update | 1,310.72 [1,277.95, 1,376.26] | 11,534.34 [2,818.05, 12,058.62] | 33,030.14 [9,175.04, 33,554.43] | 174,432.62 [157,101.15, 220,275.10] | 580.18 [533.12, 703.58] | 582.59 [535.36, 707.91] |
| kiban | mixed-90-10 | 8.96 [7.94, 9.47] | 622.59 [622.59, 655.36] | 1,114.11 [1,114.11, 1,212.42] | 9,039.38 [6,143.68, 18,395.93] | 15,062.68 [14,493.03, 15,368.40] | 15,445.63 [14,858.37, 15,729.67] |
| lmdb | mixed-90-10 | 2.37 [2.30, 2.62] | 2,490.37 [2,293.76, 2,555.90] | 12,582.91 [11,272.19, 12,845.06] | 180,247.93 [168,263.84, 508,508.04] | 4,690.18 [4,358.31, 4,957.22] | 4,734.65 [4,395.62, 5,011.33] |

| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | ---: | ---: | ---: |
| kiban | 100.88 [85.20, 106.33] | 1,613.90 [1,607.70, 1,615.82] | 323.91 [323.91, 323.91] | 323.92 [323.92, 323.92] |
| lmdb | 814.50 [814.44, 814.57] | 1,329.32 [1,329.29, 1,329.43] | 807.46 [807.46, 807.46] | 807.45 [807.45, 807.45] |

## N=1,000,000, fast

Preliminary: load/coverage gate failed. Untimed verified/reopened base: 1,000,000 records. W1 inserts 100,000 fresh records; W2/W3/W5: 100,000 calls each; W4: 100,000 overwrites. Final population: 1,100,000. 3 alternating trials per engine.

| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| kiban | remember | 0.74 [0.72, 0.78] | 5.25 [5.12, 5.63] | 10.50 [9.73, 12.29] | 194.47 [182.54, 305.75] | 453,601.57 [421,710.71, 463,941.42] | 970,477.28 [897,401.22, 998,253.27] |
| lmdb | remember | 7.68 [7.42, 7.68] | 12.03 [11.52, 12.29] | 16.38 [14.85, 18.94] | 290.62 [44.82, 298.88] | 104,966.95 [104,512.42, 108,445.50] | 125,721.54 [125,293.45, 130,321.75] |
| kiban | recall-by-id | 5.63 [5.50, 6.27] | 7.68 [7.42, 8.70] | 12.29 [11.78, 14.08] | 456.44 [447.02, 474.84] | 167,169.37 [150,355.83, 170,215.24] | 196,770.44 [176,485.69, 200,647.14] |
| lmdb | recall-by-id | 1.38 [1.34, 1.41] | 1.86 [1.86, 1.95] | 4.86 [4.48, 4.99] | 25.11 [9.29, 75.32] | 434,591.65 [430,304.37, 453,514.89] | 722,342.33 [703,805.81, 747,439.01] |
| kiban | recent-for-agent | 2.94 [2.94, 3.07] | 5.50 [4.99, 6.14] | 9.73 [9.73, 10.50] | 67.29 [62.27, 75.49] | 139,904.47 [132,881.71, 140,174.04] | 333,037.68 [319,171.26, 333,192.00] |
| lmdb | recent-for-agent | 2.11 [2.11, 2.18] | 3.46 [3.46, 3.78] | 6.14 [6.02, 6.27] | 44.90 [32.76, 74.61] | 159,978.44 [156,597.70, 160,140.19] | 461,775.82 [451,491.76, 463,696.64] |
| kiban | update | 1.54 [1.34, 1.57] | 6.66 [6.02, 6.78] | 11.78 [10.24, 13.06] | 262.45 [138.44, 278.07] | 310,405.84 [301,914.12, 346,074.75] | 546,321.76 [541,896.23, 640,245.20] |
| lmdb | update | 18.94 [17.41, 19.97] | 53.25 [49.15, 67.58] | 1,277.95 [966.66, 1,507.33] | 69,041.95 [68,594.14, 217,997.95] | 31,409.81 [28,816.91, 33,451.88] | 34,251.47 [31,024.45, 36,228.75] |
| kiban | mixed-90-10 | 6.40 [6.14, 7.17] | 9.22 [8.19, 10.50] | 14.08 [13.06, 16.90] | 300.23 [84.75, 653.12] | 160,223.18 [145,391.62, 169,229.76] | 189,821.46 [171,330.08, 201,042.24] |
| lmdb | mixed-90-10 | 1.79 [1.63, 1.86] | 29.70 [27.65, 31.74] | 104.45 [102.40, 118.78] | 3,160.58 [1,278.84, 17,689.74] | 196,638.37 [185,422.13, 201,830.46] | 262,581.64 [249,469.68, 263,528.18] |

| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |
| --- | ---: | ---: | ---: | ---: |
| kiban | 284.42 [269.75, 955.00] | 1,604.51 [1,604.43, 1,612.86] | 376.14 [376.14, 376.14] | 376.16 [376.16, 376.16] |
| lmdb | 886.38 [886.29, 886.43] | 1,329.46 [1,329.19, 1,329.52] | 879.32 [879.32, 879.32] | 879.31 [879.31, 879.31] |

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

## What these numbers include

All three paths execute in the same release binary, with the same MiMalloc allocator and compiler profile. There is no HTTP, router, socket, request parsing or response JSON transport. Helix executes the generated storage/traversal statements with typed inputs/results; `engine/equivalence.json` retains the exact compiler output used for extraction. Its graph layout, required indexes, UUID generation, arena, serialization and per-query transaction remain included. Unused automatic BM25 indexing is explicitly disabled for this study; the HTTP study left it enabled. Kiban and raw LMDB use the same primary key and JSON storage codec; this JSON is persistent data encoding, not transport.

`durable`: Kiban put + sync (WAL sync_data/fdatasync) for every write; Helix and raw LMDB commit each write transaction with neither NO_SYNC nor NO_META_SYNC. `fast`: Kiban put without per-write sync; Helix and raw LMDB enable NO_SYNC | NO_META_SYNC on their exclusively owned environment through heed3 before foreground calls. Kiban maintenance may still sync. Actual LMDB flag bits are recorded for every process. All backends explicitly synchronize once after timed work, before full verification and orderly close. Reopen checks do not establish power-loss durability for fast mode.

Every timed value written is immediately read back and checked field-for-field, outside write latency. All read/list outputs are checked. The complete population and all latest lists are checked again before close and in a new process after reopen. Any error invalidates the campaign. Warmup uses a separate discarded database in the same process, outside timing. The base dataset is loaded untimed: Kiban WriteBatch chunks of 10,000, Helix/LMDB one large transaction, with relaxed per-call durability and explicit final sync. Every base record is verified before close and again after reopening in the selected mode. Timed inserts use fresh IDs after that base; all later reads use the resulting population. Durable W1/W4 are capped at 10,000 calls; W5 retains its full 90/10 count.

Verified throughput includes generation, oracle comparisons and separate write read-backs. Call-only throughput divides call count by the sum of foreground API durations. Final sync/flush, full verification and reopen checks are excluded from both foreground metrics. p999 requires at least 10,000 samples. Histogram percentile bounds have <3.125% relative bucket error above 32 ns; maximum is exact.

RSS includes the entire benchmark process: engine, bulk preload, oracle revision vector, allocator and mapped resident pages. It is not isolated engine heap usage; kernel page cache outside the process is not counted. Disk sizes are sampled after orderly process exit and before reopen. Apparent bytes and st_blocks×512 allocated bytes differ on this compressed Btrfs filesystem. The larger dataset exceeds Kiban’s default 4 MiB memtable and 32 MiB block cache, but can still fit in host RAM. This is not an out-of-RAM or cold-cache campaign.
