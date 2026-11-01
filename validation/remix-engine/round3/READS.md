| Scenario | Workload | Trials | K p50/p99 before (us) | K p50/p99 after (us) | LMDB after (us) | Paired p50 before/after | Paired p99 before/after |
|---|---|---:|---:|---:|---:|---:|---:|
| bulk-512 | recall-by-id | 5 | 1.216/1.920 | 1.184/1.856 | 1.888/3.200 | 0.600/0.627 | 0.600/0.580 |
| bulk-512 | recent-for-agent | 5 | 3.968/7.680 | 2.816/5.120 | 3.200/6.144 | 1.240/0.860 | 1.294/0.870 |
