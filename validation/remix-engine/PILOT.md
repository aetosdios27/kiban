| Scenario | Workload | Trials | K p50/p99 before (us) | K p50/p99 after (us) | LMDB after (us) | Paired p50 before/after | Paired p99 before/after |
|---|---|---:|---:|---:|---:|---:|---:|
| bulk-512 | recall-by-id | 1 | 0.928/1.312 | 0.896/1.216 | 1.760/3.072 | 0.580/0.509 | 0.500/0.396 |
| bulk-512 | recent-for-agent | 1 | 3.200/6.400 | 2.176/4.480 | 2.880/6.528 | 1.136/0.756 | 1.042/0.686 |
| normal-512 | recall-by-id | 1 | 1.536/2.432 | 1.248/1.824 | 1.472/2.304 | 0.960/0.848 | 0.974/0.792 |
| normal-512 | recent-for-agent | 1 | 4.224/9.472 | 2.112/3.712 | 2.432/4.608 | 1.535/0.868 | 1.644/0.806 |
