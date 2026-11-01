# Agent-memory benchmark

Status: **PRELIMINARY — loaded host**.

Maximum sampled 1-minute load during measurements: 5.93; threshold <2.

Cells are median [minimum, maximum] of per-run metrics; percentiles are not pooled. Latencies are µs.

| Path | Kiban embedded | Kiban server (placeholder) | Helix client | Helix transport floor | Hydra hosted |
| --- | --- | --- | --- | --- | --- |
| Transport | in-process | **not implemented** | HTTP/1.1 loopback | same HTTP server, Noop | skipped |

| System | Workload | Runs | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| helix | mixed-90-10 | 5 | 88.06 [81.92, 92.16] | 6,422.53 [4,849.66, 12,845.06] | 16,777.22 [12,582.91, 21,495.81] | 134,885.14 [61,954.25, 186,430.69] | 1,858.71 [1,426.66, 1,901.10] | 1,949.05 [1,482.60, 2,026.89] |
| helix | recall-by-id | 5 | 71.68 [67.58, 90.11] | 1,081.34 [516.10, 1,409.02] | 3,014.66 [2,949.12, 3,080.19] | 3,410.92 [3,101.71, 5,852.28] | 8,567.73 [6,186.95, 9,676.75] | 8,766.06 [6,320.37, 9,943.12] |
| helix | recent-for-agent | 5 | 344.06 [303.10, 360.45] | 1,900.54 [1,507.33, 2,818.05] | 3,670.02 [3,473.41, 4,849.66] | 5,533.80 [3,727.37, 16,700.62] | 2,052.46 [2,022.41, 2,278.02] | 2,111.64 [2,081.64, 2,342.25] |
| helix | remember | 5 | 3,407.87 [2,555.90, 3,801.09] | 15,990.78 [13,369.34, 17,825.79] | 48,234.50 [26,214.40, 65,011.71] | 165,125.41 [56,943.10, 373,445.07] | 230.38 [172.73, 292.72] | 247.04 [183.23, 314.82] |
| helix | update | 5 | 2,883.58 [2,097.15, 2,949.12] | 13,893.63 [7,864.32, 14,155.78] | 36,700.16 [16,515.07, 71,303.17] | 225,838.98 [73,148.13, 347,581.58] | 282.30 [260.14, 377.78] | 306.18 [284.77, 418.72] |
| helix-floor | transport-floor | 5 | 58.37 [52.22, 65.54] | 901.12 [352.26, 1,966.08] | 2,883.58 [2,293.76, 3,014.66] | 4,299.86 [3,213.65, 7,785.83] | 9,787.12 [7,044.20, 13,705.11] | 9,793.14 [7,047.39, 13,714.13] |
| kiban | mixed-90-10 | 5 | 1.70 [1.57, 1.98] | 720.90 [655.36, 753.66] | 2,359.30 [901.12, 11,010.05] | 6,383.65 [2,895.73, 73,748.32] | 13,837.97 [11,438.04, 16,135.78] | 14,526.28 [11,987.91, 17,000.34] |
| kiban | recall-by-id | 5 | 0.66 [0.61, 0.75] | 1.63 [1.25, 2.02] | 4.35 [2.18, 5.38] | 19.14 [9.06, 88.81] | 477,831.21 [401,304.59, 511,565.52] | 1,453,225.82 [1,216,550.04, 1,557,232.90] |
| kiban | recent-for-agent | 5 | 2.62 [2.62, 2.69] | 5.38 [4.35, 7.17] | 9.47 [7.55, 15.10] | 32.80 [15.30, 165.29] | 97,888.49 [89,733.57, 101,636.20] | 353,521.23 [331,528.91, 373,880.40] |
| kiban | remember | 5 | 573.44 [557.06, 589.82] | 1,179.65 [1,081.34, 5,111.81] | 5,767.17 [2,883.58, 12,058.62] | 34,931.64 [30,409.56, 36,413.90] | 1,668.72 [1,376.43, 1,713.59] | 1,690.54 [1,392.21, 1,734.09] |
| kiban | update | 5 | 589.82 [557.06, 622.59] | 1,179.65 [1,081.34, 4,718.59] | 6,029.31 [2,883.58, 12,320.77] | 22,333.89 [7,738.43, 30,517.29] | 1,616.43 [1,327.92, 1,713.33] | 1,639.08 [1,344.92, 1,736.00] |

Observed load during each run (per-second samples; median [min, max]). The interval includes client setup and final verification. A sampling gap is explicitly marked; missing values are not inferred.

| Trial | Path | 1-minute load | 5-minute load | 15-minute load | Samples / largest gap | Coverage |
| ---: | --- | ---: | ---: | ---: | --- | --- |
| 1 | kiban | 1.37 [1.32, 1.50] | 10.36 [10.23, 10.50] | 20.43 [20.33, 20.53] | 13 / 1.0s | sampled throughout |
| 1 | helix-floor | 1.78 [1.78, 1.78] | 10.15 [10.15, 10.15] | 20.25 [20.25, 20.25] | 2 / 1.0s | sampled throughout |
| 1 | helix | 2.40 [1.78, 3.12] | 9.14 [8.34, 10.15] | 19.43 [18.67, 20.25] | 92 / 1.0s | sampled throughout |
| 2 | kiban | 2.82 [2.67, 3.06] | 8.07 [7.95, 8.30] | 18.42 [18.33, 18.60] | 13 / 1.0s | sampled throughout |
| 2 | helix-floor | 2.46 [2.46, 2.46] | 7.82 [7.82, 7.82] | 18.23 [18.23, 18.23] | 2 / 1.0s | sampled throughout |
| 2 | helix | 3.04 [2.46, 3.38] | 7.22 [6.70, 7.82] | 17.53 [16.87, 18.23] | 92 / 1.0s | sampled throughout |
| 3 | kiban | 3.24 [3.16, 3.52] | 6.54 [6.52, 6.62] | 16.68 [16.59, 16.79] | 16 / 1.0s | sampled throughout |
| 3 | helix-floor | 4.20 [4.20, 4.20] | 6.61 [6.61, 6.61] | 16.57 [16.57, 16.57] | 1 / 1.0s | sampled throughout |
| 3 | helix | 4.83 [4.20, 5.85] | 6.51 [6.18, 6.68] | 16.05 [15.40, 16.57] | 98 / 1.0s | sampled throughout |
| 4 | kiban | 4.44 [4.35, 4.64] | 6.09 [6.06, 6.18] | 15.27 [15.21, 15.40] | 14 / 1.0s | sampled throughout |
| 4 | helix-floor | 4.65 [4.65, 4.65] | 6.07 [6.07, 6.07] | 15.16 [15.16, 15.16] | 2 / 1.0s | sampled throughout |
| 4 | helix | 5.57 [4.32, 5.93] | 6.15 [5.78, 6.30] | 14.73 [14.14, 15.19] | 99 / 1.0s | sampled throughout |
| 5 | kiban | 3.90 [3.67, 3.98] | 5.64 [5.56, 5.68] | 14.00 [13.93, 14.06] | 14 / 1.0s | sampled throughout |
| 5 | helix-floor | 3.85 [3.85, 3.85] | 5.57 [5.57, 5.57] | 13.89 [13.89, 13.89] | 1 / 0.7s | sampled throughout |
| 5 | helix | 3.19 [2.57, 3.71] | 5.19 [4.77, 5.51] | 13.42 [12.92, 13.83] | 77 / 1.0s | sampled throughout |

Durability: Kiban put+sync per write; Helix v2.3.5 LMDB default synchronous commit. No power-loss test was run. No-op has no durable writes.

`remember` and `update` both acknowledge durable writes per call. Kiban performs `put` then `sync` inside the timer; the WAL flushes its userspace buffer and calls `sync_data` (`fdatasync` on Linux), covering active and any frozen WAL. Helix commits a default synchronous LMDB write transaction before returning its HTTP acknowledgement: dirty data pages are synchronized and the metadata commit is written through a synchronous file descriptor. Neither `MDB_NOSYNC` nor `MDB_NOMETASYNC` is enabled. Thus both paths pay per-call persistence work, without cross-call batching; this does not imply one identical `fsync` syscall per call. Helix remember also creates an agent edge. Separate read-back verification is outside write latency.

Every memory write was immediately read back and compared field-for-field; every foreground read/list was checked. Every final record and each agent’s latest list was also verified after process/server restart. Any error invalidates the campaign. Warmup is a separate, discarded database per backend/trial.

Latency includes each foreground API call and serialization/deserialization. Separate write read-backs are excluded from latency. Verified throughput includes read-backs, generators, oracle comparisons and timer accounting; call-only throughput is count / sum of foreground call durations and excludes that harness work. Full post-run/reopen checks and sync/flush are outside both foreground metrics; their time is retained in JSONL.

p999 is unavailable below 10,000 samples. Histogram percentile upper bounds have <3.125% relative bucket error above 32 ns. Max is exact. Per-kind read/write distributions for mixed traffic are retained in raw JSONL.

These client-path measurements do not establish an engine-level winner, causal bottleneck or adoption ranking. These paths have different costs: embedded calls versus JSON/HTTP, and KV ordering versus graph adjacency plus sort. The Noop row includes HTTP, routing and an empty read transaction, so it is a transport floor proxy, not a pure wire measurement. Do not subtract percentiles or infer engine speedups from these rows.
