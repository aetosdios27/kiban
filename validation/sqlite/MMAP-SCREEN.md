# Mmap-screen latency results

## read-1000000-bulk-512-fast

| Workload | Engine | p50 µs [min,max] | p99 µs [min,max] | Call ops/s | Verified ops/s | p50 / SQLite | p99 / SQLite | p50 / LMDB | p99 / LMDB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| recall-by-id | clustered | 2.816 [2.688, 3.200] | 5.120 [3.648, 5.248] | 337363 | 283743 | — | — | 1.852 | 2.050 |
| recall-by-id | lmdb | 1.536 [1.472, 1.728] | 2.560 [2.176, 4.480] | 637218 | 480083 | — | — | 1.000 | 1.000 |
| recall-by-id | rowid | 2.240 [2.176, 2.496] | 3.776 [3.328, 4.608] | 407986 | 329645 | — | — | 1.447 | 1.559 |
| recent-for-agent | clustered | 8.704 [8.448, 9.728] | 16.128 [14.080, 17.408] | 109384 | 71684 | — | — | 3.167 | 3.022 |
| recent-for-agent | lmdb | 2.688 [2.688, 3.072] | 5.376 [4.864, 5.760] | 341107 | 134006 | — | — | 1.000 | 1.000 |
| recent-for-agent | rowid | 11.520 [10.496, 13.312] | 19.456 [17.920, 33.792] | 84490 | 58573 | — | — | 3.917 | 3.619 |

Ratios pair trial numbers before taking the median. Lower latency ratios are better.

## read-1000000-bulk-None-fast

| Workload | Engine | p50 µs [min,max] | p99 µs [min,max] | Call ops/s | Verified ops/s | p50 / SQLite | p99 / SQLite | p50 / LMDB | p99 / LMDB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| recall-by-id | clustered | 2.816 [2.432, 3.072] | 4.736 [3.456, 5.120] | 338133 | 285223 | — | — | 1.769 | 1.805 |
| recall-by-id | lmdb | 1.568 [1.440, 1.664] | 2.432 [2.176, 2.624] | 597185 | 452903 | — | — | 1.000 | 1.000 |
| recall-by-id | rowid | 2.304 [1.824, 2.432] | 3.968 [2.688, 4.224] | 414977 | 333030 | — | — | 1.462 | 1.553 |
| recent-for-agent | clustered | 8.960 [7.040, 9.728] | 16.896 [11.264, 22.016] | 100405 | 65454 | — | — | 3.234 | 2.870 |
| recent-for-agent | lmdb | 2.752 [2.496, 3.008] | 5.888 [4.736, 5.888] | 326346 | 125102 | — | — | 1.000 | 1.000 |
| recent-for-agent | rowid | 10.752 [8.704, 12.032] | 18.944 [13.568, 31.744] | 91115 | 63114 | — | — | 3.692 | 3.565 |

Ratios pair trial numbers before taking the median. Lower latency ratios are better.

## read-1000000-normal-512-fast

| Workload | Engine | p50 µs [min,max] | p99 µs [min,max] | Call ops/s | Verified ops/s | p50 / SQLite | p99 / SQLite | p50 / LMDB | p99 / LMDB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| recall-by-id | clustered | 2.688 [2.432, 3.328] | 4.608 [3.392, 5.760] | 361158 | 306172 | — | — | 1.909 | 2.426 |
| recall-by-id | lmdb | 1.440 [1.312, 1.504] | 2.304 [1.760, 2.560] | 691698 | 533595 | — | — | 1.000 | 1.000 |
| recall-by-id | rowid | 1.888 [1.824, 2.048] | 3.456 [2.560, 3.776] | 509043 | 410953 | — | — | 1.362 | 1.500 |
| recent-for-agent | clustered | 7.936 [7.424, 10.752] | 15.360 [12.800, 22.528] | 121192 | 78479 | — | — | 3.385 | 3.077 |
| recent-for-agent | lmdb | 2.432 [2.240, 2.560] | 4.864 [4.480, 5.248] | 398050 | 152111 | — | — | 1.000 | 1.000 |
| recent-for-agent | rowid | 9.216 [8.704, 9.728] | 16.896 [15.360, 18.432] | 103592 | 72302 | — | — | 3.789 | 3.568 |

Ratios pair trial numbers before taking the median. Lower latency ratios are better.

## read-1000000-normal-None-fast

| Workload | Engine | p50 µs [min,max] | p99 µs [min,max] | Call ops/s | Verified ops/s | p50 / SQLite | p99 / SQLite | p50 / LMDB | p99 / LMDB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| recall-by-id | clustered | 2.496 [2.432, 2.688] | 4.352 [3.200, 4.608] | 394916 | 332916 | — | — | 1.857 | 2.118 |
| recall-by-id | lmdb | 1.408 [1.344, 1.536] | 2.176 [1.824, 3.072] | 699710 | 542135 | — | — | 1.000 | 1.000 |
| recall-by-id | rowid | 1.920 [1.856, 2.176] | 3.264 [3.008, 4.096] | 509333 | 414475 | — | — | 1.381 | 1.500 |
| recent-for-agent | clustered | 7.424 [7.168, 8.448] | 13.568 [12.288, 16.896] | 128964 | 83664 | — | — | 3.222 | 3.059 |
| recent-for-agent | lmdb | 2.368 [2.240, 2.688] | 4.608 [4.352, 5.504] | 379284 | 146120 | — | — | 1.000 | 1.000 |
| recent-for-agent | rowid | 8.960 [8.704, 9.984] | 15.616 [15.360, 17.920] | 108257 | 74983 | — | — | 3.886 | 3.529 |

Ratios pair trial numbers before taking the median. Lower latency ratios are better.
