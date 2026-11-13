# RocksDB index/filter placement: fairness check

Medians of five rotated trials, 1M records, 100k calls per operation. Ratios are same-round pairs, median over five rounds; below 1 favors the row engine.

## read-1000000-bulk-512-fast

| Operation | Engine | p50 µs | p99 µs | kops/s | peak RSS MiB | p50 / RocksDB in-cache | p50 / RocksDB outside |
|---|---|---:|---:|---:|---:|---:|---:|
| recall-by-id | kiban-index0 | 1.664 | 2.624 | 596.0 | 458 | 0.679 | 0.773 |
| recall-by-id | kiban-index1 | 0.896 | 1.152 | 1158.7 | 395 | 0.345 | 0.389 |
| recall-by-id | rocksdb-index-in-cache | 2.432 | 3.904 | 406.7 | 356 | 1.000 | 1.152 |
| recall-by-id | rocksdb-index-outside-cache | 2.176 | 3.520 | 460.6 | 356 | 0.868 | 1.000 |
| recall-by-id | lmdb | 1.376 | 1.952 | 730.1 | 839 | 0.539 | 0.625 |
| recent-for-agent | kiban-index0 | 3.072 | 5.632 | 316.5 | 458 | 0.548 | 0.598 |
| recent-for-agent | kiban-index1 | 2.944 | 5.376 | 326.8 | 395 | 0.548 | 0.575 |
| recent-for-agent | rocksdb-index-in-cache | 5.376 | 8.960 | 182.3 | 356 | 1.000 | 1.050 |
| recent-for-agent | rocksdb-index-outside-cache | 5.120 | 8.960 | 190.5 | 356 | 0.952 | 1.000 |
| recent-for-agent | lmdb | 2.432 | 4.864 | 394.0 | 839 | 0.452 | 0.475 |

## read-1000000-bulk-None-fast

| Operation | Engine | p50 µs | p99 µs | kops/s | peak RSS MiB | p50 / RocksDB in-cache | p50 / RocksDB outside |
|---|---|---:|---:|---:|---:|---:|---:|
| recall-by-id | kiban-index0 | 1.760 | 2.560 | 567.4 | 458 | 0.619 | 0.688 |
| recall-by-id | kiban-index1 | 0.896 | 1.184 | 1153.6 | 396 | 0.321 | 0.350 |
| recall-by-id | rocksdb-index-in-cache | 2.688 | 3.968 | 368.7 | 356 | 1.000 | 1.083 |
| recall-by-id | rocksdb-index-outside-cache | 2.560 | 3.840 | 391.6 | 356 | 0.923 | 1.000 |
| recall-by-id | lmdb | 1.536 | 2.176 | 632.0 | 839 | 0.564 | 0.605 |
| recent-for-agent | kiban-index0 | 3.008 | 5.248 | 326.3 | 458 | 0.545 | 0.571 |
| recent-for-agent | kiban-index1 | 2.944 | 5.504 | 327.1 | 396 | 0.548 | 0.548 |
| recent-for-agent | rocksdb-index-in-cache | 5.504 | 9.472 | 175.7 | 356 | 1.000 | 1.000 |
| recent-for-agent | rocksdb-index-outside-cache | 5.376 | 11.264 | 162.1 | 356 | 1.000 | 1.000 |
| recent-for-agent | lmdb | 2.624 | 4.480 | 370.6 | 839 | 0.466 | 0.480 |

## read-1000000-normal-512-fast

| Operation | Engine | p50 µs | p99 µs | kops/s | peak RSS MiB | p50 / RocksDB in-cache | p50 / RocksDB outside |
|---|---|---:|---:|---:|---:|---:|---:|
| recall-by-id | kiban-index0 | 1.760 | 2.880 | 557.8 | 460 | 0.671 | 0.730 |
| recall-by-id | kiban-index1 | 0.992 | 1.376 | 1025.3 | 433 | 0.369 | 0.397 |
| recall-by-id | rocksdb-index-in-cache | 2.688 | 4.608 | 371.3 | 356 | 1.000 | 1.079 |
| recall-by-id | rocksdb-index-outside-cache | 2.432 | 4.352 | 404.2 | 356 | 0.927 | 1.000 |
| recall-by-id | lmdb | 1.376 | 1.984 | 722.1 | 839 | 0.512 | 0.564 |
| recent-for-agent | kiban-index0 | 3.520 | 6.656 | 274.0 | 460 | 0.444 | 0.451 |
| recent-for-agent | kiban-index1 | 3.072 | 6.016 | 316.3 | 433 | 0.375 | 0.387 |
| recent-for-agent | rocksdb-index-in-cache | 8.192 | 13.312 | 120.6 | 356 | 1.000 | 1.032 |
| recent-for-agent | rocksdb-index-outside-cache | 7.936 | 12.800 | 124.3 | 356 | 0.969 | 1.000 |
| recent-for-agent | lmdb | 2.496 | 4.992 | 392.0 | 839 | 0.305 | 0.311 |

## read-1000000-normal-None-fast

| Operation | Engine | p50 µs | p99 µs | kops/s | peak RSS MiB | p50 / RocksDB in-cache | p50 / RocksDB outside |
|---|---|---:|---:|---:|---:|---:|---:|
| recall-by-id | kiban-index0 | 1.856 | 2.944 | 535.6 | 456 | 0.690 | 0.724 |
| recall-by-id | kiban-index1 | 1.008 | 1.408 | 992.9 | 430 | 0.384 | 0.393 |
| recall-by-id | rocksdb-index-in-cache | 2.688 | 4.608 | 362.6 | 356 | 1.000 | 1.050 |
| recall-by-id | rocksdb-index-outside-cache | 2.560 | 4.480 | 380.9 | 356 | 0.952 | 1.000 |
| recall-by-id | lmdb | 1.408 | 2.112 | 703.7 | 839 | 0.536 | 0.549 |
| recent-for-agent | kiban-index0 | 3.584 | 6.656 | 272.4 | 456 | 0.439 | 0.435 |
| recent-for-agent | kiban-index1 | 3.136 | 6.272 | 309.1 | 430 | 0.389 | 0.393 |
| recent-for-agent | rocksdb-index-in-cache | 8.064 | 14.336 | 119.6 | 356 | 1.000 | 1.000 |
| recent-for-agent | rocksdb-index-outside-cache | 8.064 | 13.056 | 120.9 | 356 | 1.000 | 1.000 |
| recent-for-agent | lmdb | 2.432 | 4.736 | 394.1 | 839 | 0.302 | 0.306 |
