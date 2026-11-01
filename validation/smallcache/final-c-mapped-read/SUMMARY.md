# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| normal/1/32/1/0/recall-by-id | 3.328 / 5.248 | 1.344 / 2.048 | 1.824 / 2.944 | 1.728 / 2.496 | 1.815 / 0.778 |
| normal/1/32/1/0/recent-for-agent | 4.224 / 8.192 | 3.712 / 7.296 | 3.008 / 6.272 | 2.816 / 5.760 | 1.417 / 1.273 |
| normal/1/512/1/0/recall-by-id | 1.376 / 2.112 | 1.280 / 2.016 | 1.792 / 2.688 | 1.760 / 2.688 | 0.811 / 0.727 |
| normal/1/512/1/0/recent-for-agent | 4.352 / 7.936 | 4.032 / 7.936 | 3.008 / 5.888 | 2.944 / 6.272 | 1.447 / 1.318 |
| normal/1/None/1/0/recall-by-id | 1.376 / 2.112 | 1.216 / 1.760 | 1.728 / 2.560 | 1.760 / 2.752 | 0.796 / 0.685 |
| normal/1/None/1/0/recent-for-agent | 4.352 / 8.192 | 3.776 / 7.808 | 2.944 / 5.888 | 3.008 / 6.144 | 1.432 / 1.255 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| normal/1/32/1/0/recall-by-id | before-kiban | 3.328 [3.136, 3.584] | 5.248 [4.864, 6.144] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.824 [1.696, 1.920] | 2.944 [2.496, 3.008] |
| normal/1/32/1/0/recall-by-id | rebased-kiban | 1.344 [1.312, 1.568] | 2.048 [1.984, 2.496] |
| normal/1/32/1/0/recall-by-id | rebased-lmdb | 1.728 [1.696, 1.792] | 2.496 [2.368, 2.752] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 4.224 [4.096, 4.352] | 8.192 [7.808, 8.960] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 3.008 [2.816, 3.264] | 6.272 [5.248, 7.040] |
| normal/1/32/1/0/recent-for-agent | rebased-kiban | 3.712 [3.456, 3.840] | 7.296 [7.040, 7.680] |
| normal/1/32/1/0/recent-for-agent | rebased-lmdb | 2.816 [2.688, 3.648] | 5.760 [4.736, 7.168] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.376 [1.344, 1.600] | 2.112 [1.920, 2.368] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.792 [1.664, 1.856] | 2.688 [2.304, 2.944] |
| normal/1/512/1/0/recall-by-id | rebased-kiban | 1.280 [1.184, 1.504] | 2.016 [1.632, 2.176] |
| normal/1/512/1/0/recall-by-id | rebased-lmdb | 1.760 [1.728, 1.792] | 2.688 [2.560, 2.816] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 4.352 [4.032, 4.608] | 7.936 [7.552, 10.496] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 3.008 [2.816, 3.136] | 5.888 [4.992, 6.912] |
| normal/1/512/1/0/recent-for-agent | rebased-kiban | 4.032 [3.712, 4.608] | 7.936 [7.552, 9.472] |
| normal/1/512/1/0/recent-for-agent | rebased-lmdb | 2.944 [2.816, 3.136] | 6.272 [5.248, 6.656] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.376 [1.344, 1.472] | 2.112 [1.920, 2.240] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.728 [1.696, 2.368] | 2.560 [2.432, 3.264] |
| normal/1/None/1/0/recall-by-id | rebased-kiban | 1.216 [1.184, 1.312] | 1.760 [1.664, 2.176] |
| normal/1/None/1/0/recall-by-id | rebased-lmdb | 1.760 [1.728, 2.048] | 2.752 [2.432, 3.072] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 4.352 [4.032, 4.608] | 8.192 [7.552, 9.728] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 2.944 [2.816, 3.200] | 5.888 [4.864, 7.424] |
| normal/1/None/1/0/recent-for-agent | rebased-kiban | 3.776 [3.776, 4.224] | 7.808 [7.168, 9.984] |
| normal/1/None/1/0/recent-for-agent | rebased-lmdb | 3.008 [2.816, 3.072] | 6.144 [5.632, 7.040] |
