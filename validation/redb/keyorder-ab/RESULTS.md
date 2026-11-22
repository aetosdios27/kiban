# Key order results

Five-trial medians [min, max]. Latencies are in µs.

| Scenario | Operation | Percentile | Descending | Plain | Outcome |
|---|---|---|---:|---:|---|
| read-1000000-bulk-512-fast | recall-by-id | p50 | 2.112 [2.016, 2.432] | 2.048 [1.920, 2.240] | Within noise |
| read-1000000-bulk-512-fast | recall-by-id | p99 | 3.328 [3.200, 3.840] | 3.264 [3.072, 3.584] | Within noise |
| read-1000000-bulk-512-fast | recent-for-agent | p50 | 3.584 [3.200, 4.992] | 3.392 [3.264, 3.648] | Within noise |
| read-1000000-bulk-512-fast | recent-for-agent | p99 | 7.808 [6.144, 10.752] | 7.424 [7.040, 8.704] | Within noise |
| read-1000000-bulk-None-fast | recall-by-id | p50 | 1.952 [1.664, 2.176] | 1.792 [1.664, 1.824] | Within noise |
| read-1000000-bulk-None-fast | recall-by-id | p99 | 2.880 [2.368, 3.328] | 2.496 [2.432, 2.560] | Within noise |
| read-1000000-bulk-None-fast | recent-for-agent | p50 | 3.200 [3.008, 3.264] | 3.264 [2.880, 3.392] | Within noise |
| read-1000000-bulk-None-fast | recent-for-agent | p99 | 4.864 [4.480, 6.912] | 5.120 [4.480, 6.272] | Within noise |
| read-1000000-normal-512-fast | recall-by-id | p50 | 2.016 [2.016, 2.240] | 2.112 [2.112, 2.240] | Within noise |
| read-1000000-normal-512-fast | recall-by-id | p99 | 4.736 [4.608, 5.248] | 4.736 [4.608, 5.120] | Within noise |
| read-1000000-normal-512-fast | recent-for-agent | p50 | 3.136 [3.008, 3.392] | 3.072 [2.944, 3.264] | Within noise |
| read-1000000-normal-512-fast | recent-for-agent | p99 | 4.992 [4.608, 5.376] | 4.736 [4.608, 4.992] | Within noise |
| read-1000000-normal-None-fast | recall-by-id | p50 | 2.496 [2.304, 2.688] | 2.624 [2.432, 3.264] | Within noise |
| read-1000000-normal-None-fast | recall-by-id | p99 | 5.504 [5.120, 5.888] | 5.376 [5.248, 6.784] | Within noise |
| read-1000000-normal-None-fast | recent-for-agent | p50 | 3.584 [3.328, 3.840] | 3.712 [3.328, 3.968] | Within noise |
| read-1000000-normal-None-fast | recent-for-agent | p99 | 5.376 [4.992, 7.552] | 5.376 [4.864, 7.296] | Within noise |
| write-10000-bulk-None-fast | remember | p50 | 8.960 [8.704, 9.216] | 8.960 [8.960, 9.216] | Within noise |
| write-10000-bulk-None-fast | remember | p99 | 13.056 [12.800, 13.568] | 13.568 [13.056, 14.336] | Within noise |
| write-10000-bulk-None-fast | recall-by-id | p50 | 1.184 [1.120, 1.184] | 1.184 [1.152, 1.248] | Within noise |
| write-10000-bulk-None-fast | recall-by-id | p99 | 1.600 [1.568, 1.632] | 1.600 [1.568, 1.728] | Within noise |
| write-10000-bulk-None-fast | recent-for-agent | p50 | 3.136 [3.072, 3.264] | 3.200 [3.136, 3.264] | Within noise |
| write-10000-bulk-None-fast | recent-for-agent | p99 | 4.224 [3.904, 4.736] | 4.224 [3.840, 6.912] | Within noise |
| write-10000-bulk-None-fast | update | p50 | 9.216 [8.960, 9.472] | 9.216 [9.216, 9.472] | Within noise |
| write-10000-bulk-None-fast | update | p99 | 13.056 [12.544, 14.080] | 14.336 [14.080, 15.104] | Within noise |
| write-10000-bulk-None-fast | mixed-90-10 | p50 | 1.184 [1.184, 1.344] | 1.216 [1.184, 1.248] | Within noise |
| write-10000-bulk-None-fast | mixed-90-10 | p99 | 10.240 [9.984, 12.032] | 10.496 [9.984, 11.264] | Within noise |
| write-10000-bulk-None-durable | remember | p50 | 704.512 [688.128, 737.280] | 737.280 [704.512, 770.048] | Within noise |
| write-10000-bulk-None-durable | remember | p99 | 1835.008 [1507.328, 2883.584] | 1998.848 [1835.008, 21495.808] | Within noise |
| write-10000-bulk-None-durable | recall-by-id | p50 | 1.344 [1.312, 1.536] | 1.408 [1.344, 1.728] | Within noise |
| write-10000-bulk-None-durable | recall-by-id | p99 | 2.560 [2.304, 2.880] | 2.624 [2.432, 3.072] | Within noise |
| write-10000-bulk-None-durable | recent-for-agent | p50 | 3.520 [3.520, 4.224] | 3.584 [3.520, 4.736] | Within noise |
| write-10000-bulk-None-durable | recent-for-agent | p99 | 4.992 [4.608, 7.168] | 6.272 [4.992, 11.264] | Within noise |
| write-10000-bulk-None-durable | update | p50 | 786.432 [753.664, 819.200] | 753.664 [688.128, 819.200] | Within noise |
| write-10000-bulk-None-durable | update | p99 | 2162.688 [1867.776, 3604.480] | 2490.368 [1769.472, 3407.872] | Within noise |
| write-10000-bulk-None-durable | mixed-90-10 | p50 | 2.176 [2.016, 2.240] | 2.304 [2.112, 2.496] | Within noise |
| write-10000-bulk-None-durable | mixed-90-10 | p99 | 819.200 [786.432, 917.504] | 1048.576 [851.968, 1310.720] | Within noise |
