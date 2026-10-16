# Agent-memory benchmark

Status: **PRELIMINARY — loaded host**.

Maximum sampled 1-minute load during measurements: 5.55; threshold <2.

Cells are median [minimum, maximum] of per-run metrics; percentiles are not pooled. Latencies are µs.

| Path | Kiban embedded | Kiban server (placeholder) | Helix client | Helix transport floor | Hydra hosted |
| --- | --- | --- | --- | --- | --- |
| Transport | in-process | **not implemented** | HTTP/1.1 loopback | same HTTP server, Noop | skipped |

| System | Workload | Runs | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| helix | mixed-90-10 | 5 | 79.87 [75.78, 83.97] | 5,898.24 [3,735.55, 6,684.67] | 12,845.06 [11,796.48, 13,893.63] | 37,855.57 [28,155.23, 62,566.00] | 2,042.42 [1,984.39, 2,304.54] | 2,127.78 [2,086.17, 2,414.13] |
| helix | recall-by-id | 5 | 67.58 [60.42, 69.63] | 557.06 [335.87, 1,015.81] | 3,014.66 [2,490.37, 3,080.19] | 3,252.62 [3,097.18, 4,020.47] | 8,702.83 [8,130.58, 11,346.07] | 8,895.47 [8,310.14, 11,609.39] |
| helix | recent-for-agent | 5 | 393.22 [376.83, 425.98] | 1,835.01 [1,376.26, 2,883.58] | 3,670.02 [3,538.94, 4,194.30] | 9,452.16 [3,728.73, 10,558.77] | 1,953.55 [1,894.56, 1,976.08] | 2,004.94 [1,942.96, 2,026.38] |
| helix | remember | 5 | 3,211.26 [2,359.30, 3,342.34] | 14,680.06 [7,340.03, 15,204.35] | 31,981.57 [15,204.35, 54,525.95] | 169,821.52 [83,796.56, 270,229.03] | 268.31 [250.58, 346.01] | 284.85 [263.89, 373.74] |
| helix | update | 5 | 1,900.54 [1,867.78, 2,064.38] | 8,388.61 [7,471.10, 12,582.91] | 19,922.94 [16,777.22, 21,495.81] | 85,670.20 [68,604.30, 104,395.19] | 407.64 [364.14, 412.02] | 444.74 [389.91, 449.12] |
| helix-floor | transport-floor | 5 | 56.32 [50.18, 57.34] | 335.87 [278.53, 573.44] | 2,424.83 [1,802.24, 3,014.66] | 3,320.11 [2,971.73, 3,837.55] | 11,562.89 [9,334.38, 13,456.65] | 11,568.32 [9,339.42, 13,464.02] |
| kiban | mixed-90-10 | 5 | 1.50 [1.50, 1.60] | 688.13 [638.98, 720.90] | 1,114.11 [1,032.19, 2,555.90] | 5,705.82 [2,852.92, 32,157.27] | 15,208.51 [14,145.29, 16,798.42] | 15,935.10 [14,797.42, 17,752.60] |
| kiban | recall-by-id | 5 | 0.61 [0.61, 0.66] | 1.54 [1.44, 2.75] | 2.50 [2.43, 4.61] | 11.17 [8.15, 25.34] | 472,832.58 [439,113.26, 489,192.32] | 1,494,158.59 [1,423,724.46, 1,537,578.73] |
| kiban | recent-for-agent | 5 | 2.56 [2.50, 2.62] | 4.74 [4.22, 7.42] | 9.47 [6.27, 12.80] | 22.32 [11.12, 47.61] | 102,963.75 [97,890.98, 105,280.97] | 372,965.26 [355,033.98, 390,116.63] |
| kiban | remember | 5 | 540.67 [540.67, 540.67] | 1,114.11 [1,114.11, 1,245.18] | 5,636.10 [2,883.58, 5,767.17] | 29,744.28 [26,032.63, 32,921.91] | 1,736.82 [1,710.10, 1,743.03] | 1,755.89 [1,728.63, 1,761.63] |
| kiban | update | 5 | 540.67 [540.67, 573.44] | 1,032.19 [1,015.81, 1,114.11] | 5,505.02 [2,883.58, 5,636.10] | 11,253.12 [6,902.49, 19,611.70] | 1,733.28 [1,686.45, 1,782.74] | 1,752.57 [1,705.55, 1,802.15] |

Observed load during each run (per-second samples; median [min, max]). The interval includes client setup and final verification. A sampling gap is explicitly marked; missing values are not inferred.

| Trial | Path | 1-minute load | 5-minute load | 15-minute load | Samples / largest gap | Coverage |
| ---: | --- | ---: | ---: | ---: | --- | --- |
| 1 | kiban | 1.45 [1.40, 1.89] | 2.15 [2.15, 2.23] | 8.70 [8.69, 8.74] | 13 / 1.0s | sampled throughout |
| 1 | helix-floor | 1.90 [1.90, 1.90] | 2.22 [2.22, 2.22] | 8.66 [8.66, 8.66] | 1 / 0.7s | sampled throughout |
| 1 | helix | 1.80 [1.71, 2.03] | 2.15 [2.12, 2.22] | 8.39 [8.16, 8.66] | 68 / 1.0s | sampled throughout |
| 2 | kiban | 5.19 [4.93, 5.55] | 2.86 [2.85, 2.89] | 8.33 [8.29, 8.37] | 13 / 1.0s | sampled throughout |
| 2 | helix-floor | 4.54 [4.54, 4.54] | 2.80 [2.80, 2.80] | 8.25 [8.25, 8.25] | 1 / 0.9s | sampled throughout |
| 2 | helix | 3.63 [3.09, 4.54] | 2.75 [2.65, 2.81] | 8.01 [7.80, 8.25] | 74 / 1.0s | sampled throughout |
| 3 | kiban | 3.16 [3.00, 3.39] | 2.75 [2.72, 2.81] | 7.75 [7.74, 7.77] | 13 / 1.0s | sampled throughout |
| 3 | helix-floor | 3.12 [3.12, 3.12] | 2.76 [2.76, 2.76] | 7.70 [7.70, 7.70] | 1 / 1.0s | sampled throughout |
| 3 | helix | 2.75 [2.11, 3.41] | 2.74 [2.58, 2.84] | 7.48 [7.23, 7.70] | 79 / 1.0s | sampled throughout |
| 4 | kiban | 1.94 [1.94, 2.19] | 2.54 [2.53, 2.57] | 7.16 [7.15, 7.19] | 13 / 1.0s | sampled throughout |
| 4 | helix-floor | 2.09 [2.09, 2.09] | 2.54 [2.54, 2.54] | 7.12 [7.12, 7.12] | 2 / 1.0s | sampled throughout |
| 4 | helix | 2.60 [1.92, 2.72] | 2.61 [2.49, 2.64] | 6.97 [6.80, 7.12] | 69 / 1.0s | sampled throughout |
| 5 | kiban | 2.24 [2.14, 2.34] | 2.53 [2.51, 2.56] | 6.74 [6.70, 6.77] | 13 / 1.0s | sampled throughout |
| 5 | helix-floor | 1.97 [1.97, 1.97] | 2.47 [2.47, 2.47] | 6.67 [6.67, 6.67] | 1 / 0.8s | sampled throughout |
| 5 | helix | 1.91 [1.76, 2.24] | 2.39 [2.34, 2.47] | 6.45 [6.29, 6.67] | 77 / 1.0s | sampled throughout |

Durability: Kiban put+sync per write; Helix v2.3.5 LMDB default synchronous commit. No power-loss test was run. No-op has no durable writes.

`remember` and `update` both acknowledge durable writes per call. Kiban performs `put` then `sync` inside the timer; the WAL flushes its userspace buffer and calls `sync_data` (`fdatasync` on Linux), covering active and any frozen WAL. Helix commits a default synchronous LMDB write transaction before returning its HTTP acknowledgement: dirty data pages are synchronized and the metadata commit is written through a synchronous file descriptor. Neither `MDB_NOSYNC` nor `MDB_NOMETASYNC` is enabled. Thus both paths pay per-call persistence work, without cross-call batching; this does not imply one identical `fsync` syscall per call. Helix remember also creates an agent edge. Separate read-back verification is outside write latency.

Every memory write was immediately read back and compared field-for-field; every foreground read/list was checked. Every final record and each agent’s latest list was also verified after process/server restart. Any error invalidates the campaign. Warmup is a separate, discarded database per backend/trial.

Latency includes each foreground API call and serialization/deserialization. Separate write read-backs are excluded from latency. Verified throughput includes read-backs, generators, oracle comparisons and timer accounting; call-only throughput is count / sum of foreground call durations and excludes that harness work. Full post-run/reopen checks and sync/flush are outside both foreground metrics; their time is retained in JSONL.

p999 is unavailable below 10,000 samples. Histogram percentile upper bounds have <3.125% relative bucket error above 32 ns. Max is exact. Per-kind read/write distributions for mixed traffic are retained in raw JSONL.

These client-path measurements do not establish an engine-level winner, causal bottleneck or adoption ranking. These paths have different costs: embedded calls versus JSON/HTTP, and KV ordering versus graph adjacency plus sort. The Noop row includes HTTP, routing and an empty read transaction, so it is a transport floor proxy, not a pure wire measurement. Do not subtract percentiles or infer engine speedups from these rows.
