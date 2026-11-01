# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 3.968 / 7.040 | 1.376 / 2.112 | 1.920 / 3.008 | 1.888 / 3.072 | 2.033 / 0.717 |
| bulk/1/32/1/0/recent-for-agent | 4.032 / 8.192 | 3.968 / 7.680 | 3.264 / 6.528 | 3.072 / 6.400 | 1.235 / 1.292 |
| bulk/1/512/1/0/recall-by-id | 1.216 / 1.952 | 1.344 / 2.112 | 1.952 / 3.264 | 1.952 / 3.200 | 0.623 / 0.672 |
| bulk/1/512/1/0/recent-for-agent | 4.032 / 8.960 | 3.904 / 8.448 | 3.264 / 7.424 | 3.200 / 7.168 | 1.216 / 1.220 |
| bulk/1/None/1/0/recall-by-id | 1.280 / 2.048 | 1.312 / 1.952 | 1.952 / 3.136 | 1.984 / 3.200 | 0.661 / 0.661 |
| bulk/1/None/1/0/recent-for-agent | 3.968 / 7.936 | 3.840 / 7.808 | 3.328 / 6.912 | 3.072 / 6.016 | 1.192 / 1.250 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.968 [3.904, 4.032] | 7.040 [6.528, 7.168] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.920 [1.888, 1.984] | 3.008 [2.880, 3.392] |
| bulk/1/32/1/0/recall-by-id | rebased-kiban | 1.376 [1.344, 1.536] | 2.112 [1.792, 2.176] |
| bulk/1/32/1/0/recall-by-id | rebased-lmdb | 1.888 [1.856, 1.984] | 3.072 [2.688, 3.200] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 4.032 [3.968, 4.096] | 8.192 [7.040, 9.216] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 3.264 [3.200, 3.392] | 6.528 [5.632, 7.680] |
| bulk/1/32/1/0/recent-for-agent | rebased-kiban | 3.968 [3.904, 3.968] | 7.680 [7.168, 7.680] |
| bulk/1/32/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.072, 3.200] | 6.400 [5.888, 7.040] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.216 [1.152, 1.344] | 1.952 [1.888, 2.048] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.952 [1.888, 2.048] | 3.264 [3.200, 3.456] |
| bulk/1/512/1/0/recall-by-id | rebased-kiban | 1.344 [1.184, 1.440] | 2.112 [1.600, 2.368] |
| bulk/1/512/1/0/recall-by-id | rebased-lmdb | 1.952 [1.920, 2.016] | 3.200 [2.944, 3.520] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 4.032 [3.968, 4.096] | 8.960 [8.704, 9.984] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 3.264 [3.264, 3.712] | 7.424 [6.784, 8.960] |
| bulk/1/512/1/0/recent-for-agent | rebased-kiban | 3.904 [3.776, 3.968] | 8.448 [7.680, 9.472] |
| bulk/1/512/1/0/recent-for-agent | rebased-lmdb | 3.200 [3.136, 3.584] | 7.168 [6.400, 8.960] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.280 [1.248, 1.536] | 2.048 [1.856, 2.304] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.952 [1.888, 1.984] | 3.136 [2.944, 3.520] |
| bulk/1/None/1/0/recall-by-id | rebased-kiban | 1.312 [1.184, 1.408] | 1.952 [1.792, 2.176] |
| bulk/1/None/1/0/recall-by-id | rebased-lmdb | 1.984 [1.856, 1.984] | 3.200 [3.008, 3.328] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.968 [3.840, 4.032] | 7.936 [7.680, 8.704] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 3.328 [3.200, 3.712] | 6.912 [5.888, 8.448] |
| bulk/1/None/1/0/recent-for-agent | rebased-kiban | 3.840 [3.776, 3.904] | 7.808 [6.656, 8.704] |
| bulk/1/None/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.008, 3.136] | 6.016 [5.120, 7.296] |
