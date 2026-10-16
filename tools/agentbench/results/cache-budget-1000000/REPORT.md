# Large-N read diagnosis: cache budget

**Preliminary: at least one trial median load failed <3.**

Three alternating trials per configuration; LMDB, K32, K256, K512, K1024. N=1,000,000; 100,000 calls per stage; seed 20261016; CPUs 1–4; driver 12–21. Fast only; no timed writes or per-read sync. W5 preserves the stage-4 key sequence, with writes converted to reads. It is labelled mixed-key-reads-only and is not the original 90/10 workload.

Cells: median [minimum, maximum] across trials. Latencies µs; percentiles use the original histogram.

| Config | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| LMDB | recall-by-id | 1.41 [1.34, 1.44] | 1.89 [1.89, 1.92] | 4.99 [4.35, 5.12] | 93.41 [58.72, 303.29] | 548,482.90 [545,765.97, 584,422.24] | 704,680.07 [700,840.11, 740,676.23] |
| Kiban 32 MiB | recall-by-id | 5.76 [5.63, 5.76] | 8.06 [7.94, 8.70] | 13.57 [13.06, 18.43] | 483.32 [438.65, 911.83] | 169,113.68 [168,149.90, 172,864.91] | 181,704.20 [180,545.26, 185,887.30] |
| Kiban 256 MiB | recall-by-id | 2.50 [2.37, 2.75] | 7.04 [6.66, 8.70] | 10.24 [9.98, 11.52] | 96.39 [90.59, 188.49] | 289,616.94 [262,479.15, 309,366.65] | 331,318.72 [302,531.27, 351,522.99] |
| Kiban 512 MiB | recall-by-id | 2.11 [2.05, 2.18] | 3.20 [3.07, 3.26] | 6.02 [6.02, 6.91] | 90.30 [85.28, 138.15] | 393,990.09 [392,908.00, 408,274.25] | 470,704.15 [468,376.65, 490,190.37] |
| Kiban 1024 MiB | recall-by-id | 1.98 [1.95, 2.05] | 3.07 [3.07, 3.14] | 6.02 [5.89, 6.14] | 75.12 [74.35, 123.82] | 421,096.44 [404,107.46, 421,601.30] | 502,897.00 [482,888.93, 507,219.16] |
| LMDB | recent-for-agent | 2.37 [2.18, 2.43] | 3.71 [3.58, 3.78] | 6.66 [6.02, 6.66] | 78.75 [25.96, 205.32] | 162,716.89 [159,346.62, 174,774.22] | 414,625.62 [406,323.32, 450,618.18] |
| Kiban 32 MiB | recent-for-agent | 3.01 [2.94, 3.07] | 5.38 [4.86, 5.50] | 10.24 [9.22, 10.50] | 78.22 [47.92, 86.88] | 149,959.92 [148,374.34, 152,018.83] | 329,256.90 [322,948.49, 333,441.79] |
| Kiban 256 MiB | recent-for-agent | 3.33 [3.14, 3.39] | 5.76 [5.50, 5.89] | 34.82 [14.85, 37.89] | 105.98 [63.91, 150.01] | 135,976.25 [134,313.03, 143,591.04] | 289,309.28 [287,621.09, 312,358.53] |
| Kiban 512 MiB | recent-for-agent | 3.20 [3.20, 3.26] | 5.12 [4.86, 5.12] | 8.70 [8.70, 9.98] | 88.42 [76.84, 215.76] | 140,999.36 [139,577.43, 141,296.53] | 309,211.50 [303,958.71, 310,372.86] |
| Kiban 1024 MiB | recent-for-agent | 3.26 [3.20, 3.26] | 4.99 [4.86, 6.14] | 30.21 [11.52, 37.89] | 91.06 [90.50, 101.86] | 142,404.56 [137,540.07, 142,890.69] | 300,132.11 [298,545.65, 307,421.07] |
| LMDB | mixed-key-reads-only | 1.38 [1.38, 1.44] | 1.95 [1.86, 1.98] | 5.12 [4.99, 5.12] | 38.66 [37.21, 105.16] | 560,308.21 [546,190.27, 573,692.43] | 719,883.59 [701,236.94, 728,044.88] |
| Kiban 32 MiB | mixed-key-reads-only | 5.63 [5.50, 5.89] | 7.42 [7.04, 7.81] | 11.01 [11.01, 12.03] | 97.97 [70.76, 154.35] | 172,591.00 [167,613.04, 177,903.46] | 185,283.16 [180,051.93, 191,087.03] |
| Kiban 256 MiB | mixed-key-reads-only | 2.50 [2.37, 2.50] | 7.30 [7.17, 7.42] | 10.75 [10.50, 11.78] | 1,188.60 [1,154.22, 1,196.37] | 289,478.41 [287,328.14, 301,786.21] | 330,323.22 [327,150.06, 342,833.90] |
| Kiban 512 MiB | mixed-key-reads-only | 2.18 [2.11, 2.18] | 3.26 [3.14, 3.26] | 6.53 [6.14, 41.98] | 86.66 [79.65, 311.32] | 387,997.22 [367,881.48, 394,475.39] | 460,900.89 [437,266.58, 470,866.38] |
| Kiban 1024 MiB | mixed-key-reads-only | 2.02 [1.89, 2.24] | 3.14 [2.88, 3.65] | 6.02 [5.89, 7.68] | 75.03 [67.18, 666.81] | 413,742.64 [366,360.39, 436,394.23] | 493,792.73 [438,579.27, 527,186.74] |

| Config | RSS before reads MiB | RSS after reads MiB | Peak RSS through reads MiB | Disk apparent MiB |
| --- | ---: | ---: | ---: | ---: |
| LMDB | 813.81 [813.65, 815.50] | 806.76 [806.60, 806.89] | 1,329.19 [1,327.69, 1,329.31] | 799.31 [799.31, 799.31] |
| Kiban 32 MiB | 110.69 [99.20, 116.28] | 103.42 [91.61, 119.29] | 1,613.51 [1,611.41, 1,616.00] | 314.57 [314.57, 314.57] |
| Kiban 256 MiB | 424.15 [423.77, 447.93] | 416.86 [416.48, 448.15] | 1,616.38 [1,615.31, 1,616.73] | 314.57 [314.57, 314.57] |
| Kiban 512 MiB | 494.24 [486.15, 521.07] | 486.84 [486.25, 520.87] | 1,616.95 [1,611.45, 1,617.02] | 314.57 [314.57, 314.57] |
| Kiban 1024 MiB | 504.35 [497.15, 536.06] | 504.45 [489.75, 536.27] | 1,614.46 [1,613.64, 1,615.66] | 314.57 [314.57, 314.57] |

Cache hit rates below come from an **excluded SharedKiban counter replay**, not the timed direct Kiban API. It reopens the same database with a fresh cache, repeats full base verification and exactly the same read sequences. Published SST identities and levels must remain unchanged; every replay value is verified. Shared snapshot ranges supply W3. Direct Kiban exposes options but no public cache counters; changing its timed API would add another variable.

| Cache MiB | W2 hit % | W3 hit % | W5-key hit % | Resident cache after replay MiB | SST files per level after preload |
| ---: | ---: | ---: | ---: | ---: | --- |
| 32 | 10.29 [10.29, 10.29] | 99.94 [99.94, 99.94] | 10.47 [10.47, 10.47] | 32.00 [32.00, 32.00] | L0: 1 |
| 256 | 82.79 [82.79, 82.79] | 100.00 [100.00, 100.00] | 82.68 [82.68, 82.68] | 256.00 [256.00, 256.00] | L0: 1 |
| 512 | 100.00 [100.00, 100.00] | 100.00 [100.00, 100.00] | 100.00 [100.00, 100.00] | 309.14 [309.14, 309.14] | L0: 1 |
| 1024 | 100.00 [100.00, 100.00] | 100.00 [100.00, 100.00] | 100.00 [100.00, 100.00] | 309.14 [309.14, 309.14] | L0: 1 |

## Load and RSS during every trial

| Trial | Config | 1-minute load median [min, max] | Samples / max gap | Gate | RSS after reads MiB | Peak RSS MiB |
| ---: | --- | ---: | --- | --- | ---: | ---: |
| 1 | LMDB | 1.56 [1.56, 1.56] | 7 / 0.20s | pass | 806.60 | 1327.69 |
| 1 | Kiban 32 MiB | 1.63 [1.63, 1.63] | 11 / 0.20s | pass | 103.42 | 1613.51 |
| 1 | Kiban 256 MiB | 2.10 [2.10, 2.10] | 9 / 0.20s | pass | 448.15 | 1616.73 |
| 1 | Kiban 512 MiB | 2.27 [2.27, 2.27] | 8 / 0.20s | pass | 520.87 | 1611.45 |
| 1 | Kiban 1024 MiB | 3.47 [3.47, 3.47] | 8 / 0.20s | preliminary | 504.45 | 1615.66 |
| 2 | LMDB | 3.32 [3.32, 3.32] | 7 / 0.20s | preliminary | 806.76 | 1329.31 |
| 2 | Kiban 32 MiB | 3.28 [3.28, 3.34] | 11 / 0.20s | preliminary | 91.61 | 1611.41 |
| 2 | Kiban 256 MiB | 3.06 [3.06, 3.06] | 9 / 0.20s | preliminary | 416.48 | 1616.38 |
| 2 | Kiban 512 MiB | 2.90 [2.90, 2.90] | 8 / 0.20s | pass | 486.84 | 1617.02 |
| 2 | Kiban 1024 MiB | 2.85 [2.78, 2.85] | 8 / 0.20s | pass | 489.75 | 1614.46 |
| 3 | LMDB | 3.04 [2.96, 3.04] | 6 / 0.20s | preliminary | 806.89 | 1329.19 |
| 3 | Kiban 32 MiB | 3.04 [2.96, 3.04] | 11 / 0.20s | preliminary | 119.29 | 1616.00 |
| 3 | Kiban 256 MiB | 2.73 [2.73, 2.73] | 9 / 0.20s | pass | 416.86 | 1615.31 |
| 3 | Kiban 512 MiB | 3.12 [3.12, 3.12] | 9 / 0.20s | preliminary | 486.25 | 1616.95 |
| 3 | Kiban 1024 MiB | 3.02 [2.94, 3.02] | 8 / 0.20s | preliminary | 536.27 | 1613.64 |

Read latency includes key creation inside get/recent and stored-value decoding, but excludes input sampling, oracle comparison and instrumentation snapshots. Verified throughput includes those checks. Bulk preload uses the original fastest correct paths and a final sync, verifies all N values, closes/reopens and verifies again. The base is exactly N, all revision zero; this differs from the earlier five-workload campaign after W1 inserts and W4 updates. All configs share this read-only population. Full verification warms each engine before measurement; cache clearing and OS page-cache dropping are absent. Warmup is a discarded 100-record database. Final full verification and independent reopen are excluded. RSS is captured before the separate counter replay; peak includes untimed preload. Only block_cache_bytes changes; range cache remains off, file cache 128, range read-ahead default 256 KiB, mmap disabled.
