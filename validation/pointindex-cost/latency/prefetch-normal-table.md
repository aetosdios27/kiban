| Case | Workload | p50 µs baseline → candidate | p99 µs baseline → candidate | Paired p50 [min,max] | Paired p99 [min,max] | LMDB-normalized p50 / p99 |
|---|---|---:|---:|---:|---:|---:|
| normal-default | Point | 1.280 → 1.248 | 1.792 → 1.696 | 1.000 [0.778,1.147] | 1.000 [0.746,1.255] | 0.993 / 1.089 |
| normal-default | Latest-K | 3.520 → 3.520 | 6.784 → 5.888 | 0.965 [0.803,1.204] | 0.868 [0.639,1.512] | 0.941 / 0.837 |
