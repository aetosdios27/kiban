# In-process engine head-to-head

**PRELIMINARY — at least one per-trial median load gate or coverage check failed.**

Benchmark CPUs: `1-4`; driver CPUs: `12-21`. These affinities do not reserve the cores exclusively.

Cells are median [minimum, maximum] of per-trial metrics (five small-N trials, three large-N trials). Latencies are µs. The median 1-minute load must be <3 in every trial; maxima are reported, not gated.

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

## What these numbers include

All three paths execute in the same release binary, with the same MiMalloc allocator and compiler profile. There is no HTTP, router, socket, request parsing or response JSON transport. Helix executes the generated storage/traversal statements with typed inputs/results; `engine/equivalence.json` retains the exact compiler output used for extraction. Its graph layout, required indexes, UUID generation, arena, serialization and per-query transaction remain included. Unused automatic BM25 indexing is explicitly disabled for this study; the HTTP study left it enabled. Kiban and raw LMDB use the same primary key and JSON storage codec; this JSON is persistent data encoding, not transport.

`durable`: Kiban put + sync (WAL sync_data/fdatasync) for every write; Helix and raw LMDB commit each write transaction with neither NO_SYNC nor NO_META_SYNC. `fast`: Kiban put without per-write sync; Helix and raw LMDB enable NO_SYNC | NO_META_SYNC on their exclusively owned environment through heed3 before foreground calls. Kiban maintenance may still sync. Actual LMDB flag bits are recorded for every process. All backends explicitly synchronize once after timed work, before full verification and orderly close. Reopen checks do not establish power-loss durability for fast mode.

Every timed value written is immediately read back and checked field-for-field, outside write latency. All read/list outputs are checked. The complete population and all latest lists are checked again before close and in a new process after reopen. Any error invalidates the campaign. Warmup uses a separate discarded database in the same process, outside timing. The base dataset is loaded untimed: Kiban WriteBatch chunks of 10,000, Helix/LMDB one large transaction, with relaxed per-call durability and explicit final sync. Every base record is verified before close and again after reopening in the selected mode. Timed inserts use fresh IDs after that base; all later reads use the resulting population. Durable W1/W4 are capped at 10,000 calls; W5 retains its full 90/10 count.

Verified throughput includes generation, oracle comparisons and separate write read-backs. Call-only throughput divides call count by the sum of foreground API durations. Final sync/flush, full verification and reopen checks are excluded from both foreground metrics. p999 requires at least 10,000 samples. Histogram percentile bounds have <3.125% relative bucket error above 32 ns; maximum is exact.

RSS includes the entire benchmark process: engine, bulk preload, oracle revision vector, allocator and mapped resident pages. It is not isolated engine heap usage; kernel page cache outside the process is not counted. Disk sizes are sampled after orderly process exit and before reopen. Apparent bytes and st_blocks×512 allocated bytes differ on this compressed Btrfs filesystem. The larger dataset exceeds Kiban’s default 4 MiB memtable and 32 MiB block cache, but can still fit in host RAM. This is not an out-of-RAM or cold-cache campaign.
