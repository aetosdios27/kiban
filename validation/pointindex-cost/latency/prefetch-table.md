| Case | Workload | p50 µs baseline → candidate | p99 µs baseline → candidate | Paired p50 [min,max] | Paired p99 [min,max] | LMDB-normalized p50 / p99 |
|---|---|---:|---:|---:|---:|---:|
| bulk-default | Point | 0.912 → 0.896 | 1.216 → 1.152 | 0.964 [0.881,1.037] | 0.925 [0.868,1.059] | 0.914 / 0.897 |
| bulk-default | Latest-K | 3.008 → 2.944 | 5.248 → 5.120 | 0.979 [0.860,1.119] | 0.976 [0.814,1.237] | 0.979 / 0.892 |
