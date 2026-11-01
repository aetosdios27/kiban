# Comparison tables

Cells are **p50 µs / p99 µs / call kops/s**, each the median of five trials. All ranges, verified throughput and paired ratios are in MEASURE.md, FDATASYNC.md and summary.json.

## 1M reads

| Layout/cache | Operation | Kiban index off | Kiban index on | SQLite page cache | SQLite mmap | LMDB |
|---|---|---:|---:|---:|---:|---:|
| bulk/512 | recall-by-id | 1.728 / 2.560 / 578.7 | 0.928 / 1.248 / 1073.5 | 2.176 / 3.008 / 463.1 | 1.856 / 2.496 / 526.0 | 1.376 / 1.856 / 725.2 |
| bulk/512 | recent-for-agent | 2.816 / 4.736 / 347.1 | 2.752 / 4.352 / 360.5 | 9.216 / 13.312 / 108.8 | 8.704 / 12.800 / 112.5 | 2.240 / 3.584 / 437.1 |
| bulk/default | recall-by-id | 1.728 / 2.560 / 576.3 | 0.960 / 1.248 / 1042.2 | 2.368 / 3.328 / 412.2 | 2.016 / 2.752 / 486.1 | 1.440 / 1.920 / 686.0 |
| bulk/default | recent-for-agent | 3.008 / 5.120 / 322.7 | 2.944 / 4.608 / 331.2 | 9.984 / 15.360 / 93.7 | 9.984 / 15.360 / 96.9 | 2.560 / 4.480 / 373.1 |
| normal/512 | recall-by-id | 1.920 / 3.008 / 498.8 | 1.248 / 1.728 / 791.6 | 2.560 / 4.096 / 391.3 | 2.048 / 3.584 / 473.6 | 1.440 / 2.048 / 690.0 |
| normal/512 | recent-for-agent | 3.520 / 6.016 / 274.6 | 3.584 / 6.144 / 268.0 | 10.496 / 17.408 / 93.8 | 9.984 / 16.384 / 96.2 | 2.432 / 4.096 / 390.6 |
| normal/default | recall-by-id | 2.016 / 3.072 / 488.3 | 1.408 / 1.984 / 694.7 | 2.432 / 4.224 / 399.0 | 2.112 / 4.032 / 444.5 | 1.472 / 2.112 / 676.9 |
| normal/default | recent-for-agent | 3.776 / 6.528 / 255.3 | 3.712 / 6.912 / 241.7 | 11.264 / 19.968 / 81.2 | 10.240 / 17.920 / 91.8 | 2.624 / 4.608 / 370.6 |

## 10k writes

NORMAL is relaxed in both groups. FULL is the durable SQLite comparison. Fast Kiban acknowledgments can be lost on process death.

| Mode | Operation | Kiban index off | Kiban index on | SQLite OFF or FULL | SQLite NORMAL | LMDB |
|---|---|---:|---:|---:|---:|---:|
| fast | remember | 0.736 / 6.400 / 983.6 | 0.720 / 6.016 / 982.4 | 11.008 / 40.960 / 77.4 | 11.008 / 41.984 / 39.0 | 7.424 / 13.056 / 126.4 |
| fast | update | 0.848 / 6.272 / 755.1 | 0.816 / 6.912 / 907.7 | 6.784 / 10.752 / 121.7 | 6.656 / 10.752 / 44.8 | 7.680 / 11.520 / 127.3 |
| fast | mixed-90-10 | 1.184 / 2.368 / 819.3 | 0.832 / 1.408 / 1175.6 | 1.792 / 7.168 / 437.9 | 1.856 / 7.168 / 427.9 | 0.912 / 8.704 / 613.8 |
| durable | remember | 557.056 / 1179.648 / 1.7 | 540.672 / 1245.184 / 1.6 | 655.360 / 1605.632 / 1.5 | 10.496 / 44.032 / 39.3 | 1212.416 / 2686.976 / 0.8 |
| durable | update | 557.056 / 1212.416 / 1.7 | 557.056 / 1245.184 / 1.7 | 557.056 / 1703.936 / 1.7 | 6.656 / 9.728 / 43.6 | 1212.416 / 2555.904 / 0.8 |
| durable | mixed-90-10 | 2.368 / 589.824 / 17.3 | 1.632 / 573.440 / 17.0 | 2.432 / 622.592 / 16.8 | 1.856 / 7.168 / 429.2 | 1.664 / 1277.952 / 8.1 |

## Linux fdatasync follow-up

| Operation | Kiban index off | Kiban index on | SQLite FULL fsync | SQLite FULL fdatasync | LMDB |
|---|---:|---:|---:|---:|---:|
| remember | 573.440 / 2064.384 / 1.6 | 540.672 / 6815.744 / 1.3 | 655.360 / 3080.192 / 1.3 | 638.976 / 2818.048 / 1.3 | 1212.416 / 6946.816 / 0.8 |
| update | 573.440 / 2490.368 / 1.5 | 540.672 / 6029.312 / 1.4 | 557.056 / 1638.400 / 1.7 | 557.056 / 1736.704 / 1.6 | 1212.416 / 3932.160 / 0.7 |
| mixed-90-10 | 2.496 / 753.664 / 15.9 | 1.696 / 589.824 / 17.0 | 2.368 / 622.592 / 16.4 | 2.496 / 737.280 / 14.5 | 1.632 / 1441.792 / 7.5 |

## Paired 1M latency ratios

Each cell is **p50 ratio / p99 ratio** for Kiban divided by its same-round comparator. Below 1 favors Kiban.

| Layout/cache | Operation | Index | / SQLite page cache | / SQLite mmap | / LMDB |
|---|---|---|---:|---:|---:|
| bulk/512 | recall-by-id | 0 | 0.794 / 0.826 | 0.931 / 1.026 | 1.256 / 1.333 |
| bulk/512 | recall-by-id | 1 | 0.426 / 0.406 | 0.500 / 0.500 | 0.674 / 0.649 |
| bulk/512 | recent-for-agent | 0 | 0.307 / 0.349 | 0.324 / 0.385 | 1.250 / 1.250 |
| bulk/512 | recent-for-agent | 1 | 0.306 / 0.327 | 0.316 / 0.340 | 1.200 / 1.214 |
| bulk/default | recall-by-id | 0 | 0.736 / 0.776 | 0.869 / 0.905 | 1.209 / 1.273 |
| bulk/default | recall-by-id | 1 | 0.410 / 0.382 | 0.492 / 0.443 | 0.667 / 0.650 |
| bulk/default | recent-for-agent | 0 | 0.303 / 0.328 | 0.295 / 0.319 | 1.200 / 1.088 |
| bulk/default | recent-for-agent | 1 | 0.302 / 0.318 | 0.303 / 0.310 | 1.150 / 1.059 |
| normal/512 | recall-by-id | 0 | 0.750 / 0.742 | 0.937 / 0.839 | 1.378 / 1.485 |
| normal/512 | recall-by-id | 1 | 0.500 / 0.422 | 0.587 / 0.482 | 0.864 / 0.852 |
| normal/512 | recent-for-agent | 0 | 0.346 / 0.375 | 0.362 / 0.375 | 1.421 / 1.469 |
| normal/512 | recent-for-agent | 1 | 0.346 / 0.393 | 0.369 / 0.386 | 1.439 / 1.406 |
| normal/default | recall-by-id | 0 | 0.814 / 0.750 | 0.955 / 0.897 | 1.432 / 1.600 |
| normal/default | recall-by-id | 1 | 0.547 / 0.484 | 0.647 / 0.463 | 0.977 / 0.939 |
| normal/default | recent-for-agent | 0 | 0.351 / 0.354 | 0.363 / 0.386 | 1.487 / 1.545 |
| normal/default | recent-for-agent | 1 | 0.335 / 0.359 | 0.372 / 0.386 | 1.450 / 1.545 |

## 1M disk and memory

Each cell is **logical disk MiB / peak RSS MiB**, median of five process runs. Full resource ranges and allocated bytes are in MEASURE-RESOURCES.md.

| Layout/cache | Kiban index off | Kiban index on | SQLite page cache | SQLite mmap | LMDB |
|---|---:|---:|---:|---:|---:|
| bulk/512 | 314.564 / 433.609 | 391.176 / 422.598 | 314.559 / 337.504 | 314.559 / 316.891 | 799.305 / 812.055 |
| bulk/default | 314.564 / 433.590 | 391.176 / 422.523 | 314.559 / 338.289 | 314.559 / 316.215 | 799.305 / 812.324 |
| normal/512 | 314.573 / 430.105 | 391.197 / 453.223 | 314.559 / 337.926 | 314.559 / 316.027 | 799.305 / 812.172 |
| normal/default | 314.573 / 428.688 | 391.197 / 448.422 | 314.559 / 337.793 | 314.559 / 316.285 | 799.305 / 812.309 |
