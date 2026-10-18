| Scenario | Workload | Trials | K p50/p99 before (us) | K p50/p99 after (us) | LMDB after (us) | Paired p50 before/after | Paired p99 before/after |
|---|---|---:|---:|---:|---:|---:|---:|
| bulk-512 | varied-bounds | 1 | 4.352/6.400 | 3.584/5.376 | 4.608/6.656 | 0.971/0.778 | 0.980/0.808 |
| bulk-default | varied-bounds | 1 | 4.352/6.528 | 3.904/7.296 | 4.992/7.424 | 0.895/0.782 | 0.773/0.983 |
| normal-512 | varied-bounds | 1 | 5.504/8.960 | 3.712/6.400 | 4.992/7.424 | 1.162/0.744 | 1.296/0.862 |
| normal-default | varied-bounds | 1 | 5.504/9.472 | 3.584/5.120 | 4.352/7.296 | 1.132/0.824 | 1.321/0.702 |
