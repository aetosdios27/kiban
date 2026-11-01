# Isolated hardware counters

All user events ran 100% of their enabled interval. Counts are divided by the configuration operation count. IPC is instructions/cycles. Profiling timings include destruction of returned objects and are excluded from latency comparisons.

| Layout | MiB | Workload | Engine | Cycles/op | Instructions/op | IPC | L1 load misses/op | LLC load misses/op | Branch misses/op | dTLB load misses/op |
| --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bulk | 32 | w2 | kiban | 20686.925 | 25896.059 | 1.252 | 115.217 | 20.778 | 8.899 | 0.369 |
| bulk | 32 | w2 | lmdb | 5334.952 | 7889.381 | 1.479 | 45.747 | 15.251 | 11.204 | 1.973 |
| bulk | 32 | w3 | kiban | 14149.694 | 51792.799 | 3.660 | 121.169 | 0.093 | 25.002 | 0.540 |
| bulk | 32 | w3 | lmdb | 11100.517 | 42341.936 | 3.814 | 63.035 | 0.082 | 24.235 | 0.290 |
| bulk | 512 | w2 | kiban | 8503.650 | 8129.041 | 0.956 | 58.783 | 20.966 | 3.803 | 0.415 |
| bulk | 512 | w2 | lmdb | 5452.327 | 7889.381 | 1.447 | 44.653 | 15.399 | 11.297 | 1.978 |
| bulk | 512 | w3 | kiban | 15163.532 | 51865.133 | 3.420 | 128.264 | 0.246 | 24.641 | 0.651 |
| bulk | 512 | w3 | lmdb | 11016.240 | 42341.942 | 3.844 | 60.372 | 0.307 | 24.399 | 0.485 |
| normal | 32 | w2 | kiban | 22292.714 | 27551.031 | 1.236 | 206.431 | 23.219 | 14.907 | 1.290 |
| normal | 32 | w2 | lmdb | 5390.180 | 7889.382 | 1.464 | 45.737 | 15.689 | 11.421 | 2.033 |
| normal | 32 | w3 | kiban | 18683.312 | 59236.757 | 3.171 | 248.729 | 2.817 | 41.195 | 2.190 |
| normal | 32 | w3 | lmdb | 10897.403 | 42341.939 | 3.886 | 63.896 | 0.232 | 25.533 | 0.541 |
| normal | 512 | w2 | kiban | 9223.934 | 9481.208 | 1.028 | 69.836 | 20.417 | 7.157 | 0.413 |
| normal | 512 | w2 | lmdb | 5800.465 | 7889.391 | 1.360 | 49.979 | 16.267 | 11.855 | 2.093 |
| normal | 512 | w3 | kiban | 18752.826 | 59278.156 | 3.161 | 256.755 | 1.056 | 41.729 | 1.555 |
| normal | 512 | w3 | lmdb | 11428.045 | 42341.937 | 3.705 | 62.600 | 0.093 | 24.632 | 0.257 |

## Component microbenchmarks

One isolated million-operation loop per component. These costs overlap and include their access pattern; do not add them or substitute them for end-to-end latency.

| Component | ns/op | Cycles/op | Instructions/op | LLC load misses/op |
| --- | ---: | ---: | ---: | ---: |
| micro-arc | 16.286 | 50.468 | 39.004 | 0.001 |
| micro-block-linear | 808.176 | 2494.968 | 3651.195 | 7.295 |
| micro-block-seek | 863.820 | 2634.666 | 1684.548 | 7.914 |
| micro-bloom | 70.950 | 216.500 | 393.004 | 0.045 |
| micro-bulk-iter-construct | 389.233 | 1134.028 | 1958.550 | 0.007 |
| micro-bulk-iter-first | 862.376 | 2479.337 | 6344.943 | 0.025 |
| micro-bulk-iter-ten | 1894.044 | 5312.566 | 14401.731 | 0.067 |
| micro-cache | 489.264 | 1497.154 | 379.106 | 1.698 |
| micro-copy | 34.760 | 106.181 | 251.890 | 0.182 |
| micro-decode | 266.302 | 788.407 | 3645.309 | 0.009 |
| micro-hash | 59.310 | 180.484 | 243.004 | 0.316 |
| micro-index | 624.801 | 1844.000 | 911.917 | 2.682 |
| micro-lock | 19.134 | 58.116 | 53.004 | 0.000 |
| micro-loop | 2.345 | 6.850 | 28.004 | 0.000 |
| micro-memtable-hit | 163.742 | 500.067 | 921.516 | 0.003 |
| micro-memtable-miss | 27.841 | 84.554 | 387.004 | 0.001 |
| micro-normal-iter-construct | 471.463 | 1305.024 | 3256.243 | 0.021 |
| micro-normal-iter-first | 1862.111 | 5132.653 | 12975.028 | 0.166 |
| micro-normal-iter-ten | 3400.130 | 9378.704 | 21937.676 | 0.361 |
| micro-verify-block | 3192.572 | 9459.977 | 15929.491 | 3.874 |
