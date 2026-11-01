# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 4.864 / 7.680 | 1.920 / 2.944 | 1.856 / 3.136 | 1.888 / 2.944 | 2.741 / 1.000 |
| bulk/1/32/1/0/recent-for-agent | 3.840 / 8.704 | 3.968 / 8.192 | 3.136 / 6.912 | 3.200 / 7.168 | 1.213 / 1.184 |
| bulk/1/512/1/0/recall-by-id | 1.120 / 1.536 | 1.088 / 1.568 | 1.760 / 2.624 | 1.760 / 2.624 | 0.600 / 0.618 |
| bulk/1/512/1/0/recent-for-agent | 4.608 / 9.472 | 3.968 / 8.960 | 3.456 / 6.528 | 3.072 / 6.016 | 1.220 / 1.319 |
| bulk/1/None/1/0/recall-by-id | 1.152 / 1.664 | 1.120 / 1.568 | 1.920 / 2.880 | 1.952 / 3.008 | 0.603 / 0.593 |
| bulk/1/None/1/0/recent-for-agent | 4.608 / 10.240 | 4.608 / 9.728 | 3.712 / 7.424 | 3.648 / 7.552 | 1.290 / 1.236 |
| normal/1/32/1/0/recall-by-id | 4.736 / 7.424 | 2.240 / 3.648 | 1.824 / 2.880 | 1.824 / 2.816 | 2.643 / 1.214 |
| normal/1/32/1/0/recent-for-agent | 4.736 / 10.496 | 4.352 / 9.216 | 3.328 / 6.528 | 3.328 / 6.784 | 1.451 / 1.269 |
| normal/1/512/1/0/recall-by-id | 1.472 / 2.176 | 1.216 / 1.792 | 1.760 / 2.624 | 1.760 / 2.688 | 0.836 / 0.704 |
| normal/1/512/1/0/recent-for-agent | 4.480 / 8.704 | 3.904 / 7.552 | 3.200 / 6.400 | 3.008 / 6.144 | 1.429 / 1.298 |
| normal/1/None/1/0/recall-by-id | 1.440 / 2.176 | 1.280 / 1.920 | 1.792 / 2.752 | 1.728 / 2.624 | 0.804 / 0.704 |
| normal/1/None/1/0/recent-for-agent | 4.608 / 8.960 | 4.096 / 8.704 | 3.264 / 6.016 | 3.072 / 5.504 | 1.451 / 1.347 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 4.864 [4.736, 6.912] | 7.680 [6.912, 12.032] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.856 [1.728, 2.880] | 3.136 [2.496, 4.352] |
| bulk/1/32/1/0/recall-by-id | rebased-kiban | 1.920 [1.792, 2.112] | 2.944 [2.752, 3.200] |
| bulk/1/32/1/0/recall-by-id | rebased-lmdb | 1.888 [1.696, 2.048] | 2.944 [2.560, 3.264] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.840 [3.584, 5.376] | 8.704 [6.144, 12.288] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 3.136 [3.008, 3.968] | 6.912 [5.376, 8.448] |
| bulk/1/32/1/0/recent-for-agent | rebased-kiban | 3.968 [3.712, 4.032] | 8.192 [7.680, 8.704] |
| bulk/1/32/1/0/recent-for-agent | rebased-lmdb | 3.200 [2.880, 3.584] | 7.168 [5.376, 7.936] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.120 [1.056, 1.312] | 1.536 [1.376, 1.984] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.760 [1.728, 2.176] | 2.624 [2.496, 3.456] |
| bulk/1/512/1/0/recall-by-id | rebased-kiban | 1.088 [1.056, 1.344] | 1.568 [1.472, 1.856] |
| bulk/1/512/1/0/recall-by-id | rebased-lmdb | 1.760 [1.760, 1.888] | 2.624 [2.496, 2.816] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 4.608 [3.904, 4.864] | 9.472 [7.040, 11.520] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 3.456 [3.136, 3.840] | 6.528 [5.504, 8.960] |
| bulk/1/512/1/0/recent-for-agent | rebased-kiban | 3.968 [3.904, 4.736] | 8.960 [7.552, 10.752] |
| bulk/1/512/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.008, 3.520] | 6.016 [5.760, 7.296] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.152 [1.120, 1.408] | 1.664 [1.600, 2.016] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.920 [1.792, 2.112] | 2.880 [2.688, 3.392] |
| bulk/1/None/1/0/recall-by-id | rebased-kiban | 1.120 [1.056, 1.216] | 1.568 [1.536, 1.920] |
| bulk/1/None/1/0/recall-by-id | rebased-lmdb | 1.952 [1.792, 2.048] | 3.008 [2.816, 3.072] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 4.608 [4.608, 5.120] | 10.240 [7.680, 11.776] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 3.712 [3.328, 3.968] | 7.424 [6.656, 8.064] |
| bulk/1/None/1/0/recent-for-agent | rebased-kiban | 4.608 [3.776, 4.736] | 9.728 [7.808, 11.008] |
| bulk/1/None/1/0/recent-for-agent | rebased-lmdb | 3.648 [3.264, 3.968] | 7.552 [6.912, 8.704] |
| normal/1/32/1/0/recall-by-id | before-kiban | 4.736 [4.352, 4.864] | 7.424 [6.272, 7.936] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.824 [1.760, 1.856] | 2.880 [2.688, 2.944] |
| normal/1/32/1/0/recall-by-id | rebased-kiban | 2.240 [2.176, 2.624] | 3.648 [3.392, 4.224] |
| normal/1/32/1/0/recall-by-id | rebased-lmdb | 1.824 [1.760, 2.112] | 2.816 [2.688, 3.392] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 4.736 [4.480, 5.120] | 10.496 [8.960, 11.264] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 3.328 [3.264, 3.392] | 6.528 [5.888, 7.168] |
| normal/1/32/1/0/recent-for-agent | rebased-kiban | 4.352 [4.224, 4.864] | 9.216 [8.448, 10.752] |
| normal/1/32/1/0/recent-for-agent | rebased-lmdb | 3.328 [3.072, 3.456] | 6.784 [5.888, 7.168] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.472 [1.376, 1.952] | 2.176 [2.016, 3.008] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.760 [1.728, 1.792] | 2.624 [2.432, 2.816] |
| normal/1/512/1/0/recall-by-id | rebased-kiban | 1.216 [1.184, 1.248] | 1.792 [1.760, 1.824] |
| normal/1/512/1/0/recall-by-id | rebased-lmdb | 1.760 [1.728, 1.920] | 2.688 [2.496, 3.008] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 4.480 [4.480, 5.632] | 8.704 [8.064, 12.032] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 3.200 [3.136, 3.392] | 6.400 [5.888, 7.040] |
| normal/1/512/1/0/recent-for-agent | rebased-kiban | 3.904 [3.840, 4.608] | 7.552 [7.168, 9.216] |
| normal/1/512/1/0/recent-for-agent | rebased-lmdb | 3.008 [2.944, 3.136] | 6.144 [5.888, 7.040] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.440 [1.408, 1.472] | 2.176 [2.112, 2.368] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.792 [1.728, 1.824] | 2.752 [2.688, 3.008] |
| normal/1/None/1/0/recall-by-id | rebased-kiban | 1.280 [1.184, 1.504] | 1.920 [1.728, 2.176] |
| normal/1/None/1/0/recall-by-id | rebased-lmdb | 1.728 [1.696, 1.824] | 2.624 [2.432, 2.944] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 4.608 [4.480, 4.736] | 8.960 [8.192, 11.008] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 3.264 [3.072, 3.328] | 6.016 [5.760, 6.784] |
| normal/1/None/1/0/recent-for-agent | rebased-kiban | 4.096 [3.968, 4.224] | 8.704 [8.448, 9.728] |
| normal/1/None/1/0/recent-for-agent | rebased-lmdb | 3.072 [2.944, 3.200] | 5.504 [5.120, 7.424] |
