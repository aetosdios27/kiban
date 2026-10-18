## Frozen screening choice

| Complete configuration | Read score µs | Write score µs | Selected family |
|---|---:|---:|---|
| bloom-16384-prefix | 5.506 | 39.496 |  |
| bloom-16384-total | 5.252 | 40.257 |  |
| bloom-4096-prefix | 5.633 | 40.085 |  |
| bloom-4096-total | 5.253 | 40.903 |  |
| ribbon-16384-prefix | 5.587 | 41.067 |  |
| ribbon-16384-total | 5.179 | 39.490 | read, write |
| ribbon-4096-prefix | 5.708 | 41.680 |  |
| ribbon-4096-total | 5.405 | 44.200 |  |

Scores are geometric aggregates of mean call times, not public-operation latencies. All losing rows and ranges are in SCREEN.md.

## 1M reads

Cells are **p50 µs / p99 µs / call kops/s**, each the median of five trials.

| Layout/cache | Operation | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | LMDB |
|---|---|---:|---:|---:|---:|---:|
| bulk/512 | recall-by-id | 1.760 / 2.752 / 563.6 | 0.960 / 1.440 / 1038.9 | 2.304 / 4.352 / 423.2 | 2.752 / 4.992 / 361.2 | 1.472 / 2.368 / 674.9 |
| bulk/512 | recent-for-agent | 3.008 / 6.400 / 315.7 | 3.072 / 6.912 / 309.2 | 4.736 / 9.728 / 200.9 | 5.632 / 11.008 / 172.4 | 2.432 / 5.120 / 389.3 |
| bulk/default | recall-by-id | 1.824 / 3.136 / 545.3 | 0.992 / 1.408 / 1010.3 | 2.688 / 4.864 / 357.6 | 2.496 / 4.032 / 402.4 | 1.408 / 2.176 / 716.6 |
| bulk/default | recent-for-agent | 3.136 / 5.888 / 313.9 | 3.008 / 6.144 / 325.9 | 5.248 / 8.960 / 187.5 | 5.248 / 9.984 / 184.6 | 2.496 / 5.376 / 387.7 |
| normal/512 | recall-by-id | 1.888 / 3.200 / 518.6 | 1.184 / 1.664 / 837.7 | 2.752 / 4.736 / 363.7 | 2.816 / 4.864 / 354.2 | 1.376 / 2.016 / 715.6 |
| normal/512 | recent-for-agent | 3.392 / 5.504 / 288.7 | 3.136 / 5.248 / 311.3 | 7.552 / 11.008 / 130.9 | 7.680 / 12.800 / 128.4 | 2.240 / 3.648 / 436.1 |
| normal/default | recall-by-id | 1.696 / 2.560 / 587.5 | 1.088 / 1.440 / 921.8 | 2.496 / 4.032 / 402.9 | 2.496 / 4.096 / 401.1 | 1.280 / 1.696 / 776.6 |
| normal/default | recent-for-agent | 3.072 / 4.992 / 319.5 | 3.008 / 4.864 / 326.3 | 7.168 / 10.496 / 138.1 | 7.168 / 10.496 / 138.7 | 2.112 / 3.392 / 475.0 |

## 10k writes

Same cell convention. RocksDB fast keeps WAL enabled; no-WAL explicitly removes that acknowledgment guarantee.

| Mode | Operation | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | RocksDB no-WAL | LMDB |
|---|---|---:|---:|---:|---:|---:|---:|
| fast | remember | 0.608 / 4.992 / 1173.4 | 0.592 / 5.120 / 1211.3 | 2.368 / 5.376 / 324.3 | 2.304 / 5.248 / 372.1 | 0.832 / 2.496 / 954.0 | 6.016 / 12.288 / 153.5 |
| fast | update | 0.672 / 5.248 / 948.1 | 0.688 / 6.144 / 1065.9 | 2.432 / 7.168 / 339.9 | 2.368 / 5.376 / 379.4 | 0.896 / 2.240 / 974.2 | 6.144 / 9.984 / 156.7 |
| fast | mixed-90-10 | 0.784 / 1.600 / 1195.6 | 0.592 / 1.216 / 1613.0 | 1.824 / 4.480 / 532.3 | 1.824 / 6.016 / 485.7 | 1.696 / 2.880 / 600.6 | 0.704 / 6.528 / 823.5 |
| durable | remember | 557.056 / 1638.400 / 1.5 | 557.056 / 1703.936 / 1.5 | 540.672 / 1540.096 / 1.7 | 540.672 / 1638.400 / 1.6 | — | 1179.648 / 2818.048 / 0.8 |
| durable | update | 557.056 / 1933.312 / 1.6 | 540.672 / 1671.168 / 1.5 | 540.672 / 1540.096 / 1.6 | 524.288 / 1671.168 / 1.6 | — | 1179.648 / 2686.976 / 0.7 |
| durable | mixed-90-10 | 2.112 / 884.736 / 15.1 | 1.440 / 622.592 / 15.7 | 3.456 / 704.512 / 16.1 | 3.328 / 671.744 / 14.6 | — | 1.344 / 1998.848 / 7.3 |

## Paired Kiban latency ratios

Each cell is **p50 ratio / p99 ratio**, Kiban divided by its same-round comparator, then median across five rounds. Below 1 favors Kiban.

| Layout/cache | Operation | Index | / RocksDB tuned | / LMDB |
|---|---|---|---:|---:|
| bulk-512-fast | recall-by-id | 0 | 0.705 / 0.603 | 1.196 / 1.108 |
| bulk-512-fast | recall-by-id | 1 | 0.403 / 0.317 | 0.652 / 0.581 |
| bulk-512-fast | recent-for-agent | 0 | 0.554 / 0.556 | 1.237 / 1.111 |
| bulk-512-fast | recent-for-agent | 1 | 0.568 / 0.600 | 1.231 / 1.306 |
| bulk-None-fast | recall-by-id | 0 | 0.692 / 0.679 | 1.295 / 1.342 |
| bulk-None-fast | recall-by-id | 1 | 0.372 / 0.366 | 0.705 / 0.695 |
| bulk-None-fast | recent-for-agent | 0 | 0.561 / 0.621 | 1.282 / 1.095 |
| bulk-None-fast | recent-for-agent | 1 | 0.573 / 0.545 | 1.235 / 1.262 |
| normal-512-fast | recall-by-id | 0 | 0.686 / 0.686 | 1.372 / 1.514 |
| normal-512-fast | recall-by-id | 1 | 0.430 / 0.371 | 0.837 / 0.823 |
| normal-512-fast | recent-for-agent | 0 | 0.432 / 0.477 | 1.514 / 1.536 |
| normal-512-fast | recent-for-agent | 1 | 0.415 / 0.477 | 1.400 / 1.439 |
| normal-None-fast | recall-by-id | 0 | 0.697 / 0.635 | 1.325 / 1.509 |
| normal-None-fast | recall-by-id | 1 | 0.447 / 0.355 | 0.850 / 0.833 |
| normal-None-fast | recent-for-agent | 0 | 0.429 / 0.476 | 1.460 / 1.472 |
| normal-None-fast | recent-for-agent | 1 | 0.420 / 0.463 | 1.460 / 1.434 |
| bulk-None-durable | remember | 0 | 1.048 / 1.000 | 0.471 / 0.568 |
| bulk-None-durable | remember | 1 | 1.030 / 1.106 | 0.459 / 0.750 |
| bulk-None-durable | update | 0 | 1.000 / 1.128 | 0.450 / 0.646 |
| bulk-None-durable | update | 1 | 1.029 / 1.085 | 0.438 / 0.622 |
| bulk-None-durable | mixed-90-10 | 0 | 0.635 / 1.385 | 1.600 / 0.500 |
| bulk-None-durable | mixed-90-10 | 1 | 0.417 / 0.974 | 1.071 / 0.378 |
| bulk-None-fast | remember | 0 | 0.250 / 0.929 | 0.101 / 0.424 |
| bulk-None-fast | remember | 1 | 0.257 / 0.976 | 0.101 / 0.444 |
| bulk-None-fast | update | 0 | 0.276 / 0.889 | 0.109 / 0.554 |
| bulk-None-fast | update | 1 | 0.284 / 0.964 | 0.114 / 0.649 |
| bulk-None-fast | mixed-90-10 | 0 | 0.431 / 0.379 | 1.195 / 0.245 |
| bulk-None-fast | mixed-90-10 | 1 | 0.333 / 0.243 | 0.804 / 0.190 |

## 1M disk and memory

Cells are **logical disk MiB / whole-process peak RSS MiB**, median of five runs. Allocated bytes and ranges are in MEASURE-RESOURCES.md.

| Layout/cache | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | LMDB |
|---|---:|---:|---:|---:|---:|
| bulk/512 | 314.564 / 433.574 | 391.176 / 426.590 | 304.064 / 337.227 | 217.766 / 337.281 | 799.305 / 820.203 |
| bulk/default | 314.564 / 436.105 | 391.176 / 428.441 | 304.234 / 336.707 | 217.935 / 336.918 | 799.305 / 820.309 |
| normal/512 | 314.573 / 441.742 | 391.197 / 477.758 | 304.114 / 337.152 | 218.615 / 337.238 | 799.305 / 820.281 |
| normal/default | 314.573 / 441.664 | 391.197 / 477.781 | 304.285 / 337.156 | 218.786 / 336.797 | 799.305 / 820.355 |
