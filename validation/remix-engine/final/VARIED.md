| Scenario | Workload | Trials | K p50/p99 before (us) | K p50/p99 after (us) | LMDB after (us) | Paired p50 before/after | Paired p99 before/after |
|---|---|---:|---:|---:|---:|---:|---:|
| bulk-512 | varied-bounds | 5 | 5.760/11.008 | 4.608/8.960 | 5.760/10.496 | 1.022/0.833 | 1.000/0.860 |
| bulk-default | varied-bounds | 5 | 5.376/10.240 | 4.352/9.216 | 5.376/10.752 | 1.023/0.810 | 0.955/0.881 |
| normal-512 | varied-bounds | 5 | 6.272/12.032 | 4.480/8.704 | 5.376/10.752 | 1.163/0.814 | 1.250/0.857 |
| normal-default | varied-bounds | 5 | 5.760/11.520 | 3.904/7.808 | 4.992/9.472 | 1.122/0.803 | 1.098/0.847 |
