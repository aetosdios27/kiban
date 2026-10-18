# Paired results

Microseconds. Median [min, max] of 3 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 5.120 / 9.728 | 2.240 / 3.712 | 1.920 / 3.264 | 1.920 / 3.136 | 2.712 / 1.186 |
| bulk/1/32/1/0/recent-for-agent | 4.352 / 8.960 | 4.096 / 8.704 | 3.392 / 6.784 | 3.200 / 7.040 | 1.283 / 1.280 |
| bulk/1/512/1/0/recall-by-id | 1.248 / 1.920 | 1.184 / 1.792 | 1.952 / 3.136 | 1.984 / 3.200 | 0.644 / 0.597 |
| bulk/1/512/1/0/recent-for-agent | 4.096 / 8.960 | 3.968 / 8.960 | 3.392 / 7.168 | 3.520 / 23.040 | 1.231 / 1.127 |
| bulk/1/None/1/0/recall-by-id | 1.248 / 2.016 | 1.376 / 2.112 | 1.952 / 3.264 | 1.952 / 3.200 | 0.639 / 0.717 |
| bulk/1/None/1/0/recent-for-agent | 4.096 / 9.472 | 3.968 / 8.960 | 3.456 / 7.680 | 3.264 / 7.168 | 1.185 / 1.216 |
| normal/1/32/1/0/recall-by-id | 5.120 / 8.704 | 2.496 / 4.352 | 1.888 / 3.072 | 1.984 / 3.328 | 2.667 / 1.246 |
| normal/1/32/1/0/recent-for-agent | 4.736 / 10.496 | 4.096 / 8.960 | 3.264 / 6.400 | 3.136 / 5.888 | 1.451 / 1.306 |
| normal/1/512/1/0/recall-by-id | 1.728 / 2.816 | 1.664 / 2.368 | 1.920 / 2.944 | 1.920 / 3.136 | 0.905 / 0.765 |
| normal/1/512/1/0/recent-for-agent | 4.608 / 9.472 | 4.096 / 8.448 | 3.264 / 6.400 | 3.136 / 6.528 | 1.400 / 1.306 |
| normal/1/None/1/0/recall-by-id | 1.600 / 2.560 | 1.696 / 2.496 | 1.920 / 3.264 | 1.920 / 3.072 | 0.833 / 0.869 |
| normal/1/None/1/0/recent-for-agent | 4.608 / 9.984 | 3.968 / 8.448 | 3.264 / 6.528 | 3.072 / 6.528 | 1.412 / 1.292 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 5.120 [4.992, 5.376] | 9.728 [9.728, 11.776] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.920 [1.888, 1.952] | 3.264 [2.944, 3.264] |
| bulk/1/32/1/0/recall-by-id | rebased-kiban | 2.240 [2.240, 2.304] | 3.712 [3.456, 3.776] |
| bulk/1/32/1/0/recall-by-id | rebased-lmdb | 1.920 [1.888, 1.952] | 3.136 [2.880, 3.456] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 4.352 [4.224, 4.352] | 8.960 [8.704, 10.752] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 3.392 [3.328, 3.392] | 6.784 [6.528, 7.808] |
| bulk/1/32/1/0/recent-for-agent | rebased-kiban | 4.096 [4.096, 4.224] | 8.704 [8.448, 9.216] |
| bulk/1/32/1/0/recent-for-agent | rebased-lmdb | 3.200 [3.136, 3.328] | 7.040 [6.528, 8.448] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.248 [1.216, 1.344] | 1.920 [1.856, 2.048] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.952 [1.888, 2.016] | 3.136 [2.944, 3.520] |
| bulk/1/512/1/0/recall-by-id | rebased-kiban | 1.184 [1.184, 1.248] | 1.792 [1.760, 1.792] |
| bulk/1/512/1/0/recall-by-id | rebased-lmdb | 1.984 [1.920, 3.328] | 3.200 [3.136, 5.376] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 4.096 [4.032, 4.224] | 8.960 [7.936, 9.984] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 3.392 [3.328, 3.520] | 7.168 [6.400, 7.936] |
| bulk/1/512/1/0/recent-for-agent | rebased-kiban | 3.968 [3.968, 4.224] | 8.960 [8.704, 9.984] |
| bulk/1/512/1/0/recent-for-agent | rebased-lmdb | 3.520 [3.200, 8.448] | 23.040 [7.040, 29.184] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.248 [1.216, 1.376] | 2.016 [1.696, 2.240] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.952 [1.920, 1.984] | 3.264 [3.072, 3.392] |
| bulk/1/None/1/0/recall-by-id | rebased-kiban | 1.376 [1.312, 1.408] | 2.112 [1.792, 2.112] |
| bulk/1/None/1/0/recall-by-id | rebased-lmdb | 1.952 [1.920, 1.984] | 3.200 [3.072, 3.456] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 4.096 [4.096, 4.096] | 9.472 [8.448, 11.008] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 3.456 [3.392, 3.776] | 7.680 [7.424, 25.600] |
| bulk/1/None/1/0/recent-for-agent | rebased-kiban | 3.968 [3.968, 3.968] | 8.960 [8.448, 10.752] |
| bulk/1/None/1/0/recent-for-agent | rebased-lmdb | 3.264 [3.200, 3.520] | 7.168 [6.400, 8.960] |
| normal/1/32/1/0/recall-by-id | before-kiban | 5.120 [4.992, 7.168] | 8.704 [7.424, 11.776] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.888 [1.888, 1.920] | 3.072 [3.072, 3.456] |
| normal/1/32/1/0/recall-by-id | rebased-kiban | 2.496 [2.432, 4.096] | 4.352 [3.776, 7.040] |
| normal/1/32/1/0/recall-by-id | rebased-lmdb | 1.984 [1.952, 2.016] | 3.328 [2.880, 3.328] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 4.736 [4.736, 11.520] | 10.496 [9.728, 22.016] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 3.264 [3.264, 3.840] | 6.400 [6.400, 8.448] |
| normal/1/32/1/0/recent-for-agent | rebased-kiban | 4.096 [4.032, 9.728] | 8.960 [8.448, 18.944] |
| normal/1/32/1/0/recent-for-agent | rebased-lmdb | 3.136 [3.072, 3.136] | 5.888 [5.376, 6.272] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.728 [1.568, 1.824] | 2.816 [2.368, 3.264] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.920 [1.856, 2.016] | 2.944 [2.880, 3.456] |
| normal/1/512/1/0/recall-by-id | rebased-kiban | 1.664 [1.344, 1.952] | 2.368 [2.112, 2.880] |
| normal/1/512/1/0/recall-by-id | rebased-lmdb | 1.920 [1.888, 2.176] | 3.136 [3.136, 3.840] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 4.608 [4.480, 4.608] | 9.472 [8.704, 9.472] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 3.264 [3.200, 3.328] | 6.400 [6.400, 7.296] |
| normal/1/512/1/0/recent-for-agent | rebased-kiban | 4.096 [3.968, 4.224] | 8.448 [6.784, 8.704] |
| normal/1/512/1/0/recent-for-agent | rebased-lmdb | 3.136 [3.072, 3.456] | 6.528 [6.144, 8.960] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.600 [1.568, 1.728] | 2.560 [2.432, 2.624] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.920 [1.888, 2.112] | 3.264 [3.008, 3.520] |
| normal/1/None/1/0/recall-by-id | rebased-kiban | 1.696 [1.472, 1.696] | 2.496 [2.368, 2.880] |
| normal/1/None/1/0/recall-by-id | rebased-lmdb | 1.920 [1.888, 1.952] | 3.072 [2.944, 3.328] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 4.608 [4.608, 4.736] | 9.984 [9.472, 10.752] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 3.264 [3.200, 3.456] | 6.528 [6.400, 7.680] |
| normal/1/None/1/0/recent-for-agent | rebased-kiban | 3.968 [3.968, 5.120] | 8.448 [8.064, 11.520] |
| normal/1/None/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.072, 3.200] | 6.528 [6.144, 7.168] |
