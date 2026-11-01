# Paired results

Microseconds. Median [min, max] of 3 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 3.968 / 6.784 | 1.440 / 2.240 | 1.952 / 3.200 | 1.952 / 3.072 | 2.000 / 0.682 |
| bulk/1/32/1/0/recent-for-agent | 4.032 / 8.192 | 4.992 / 11.264 | 3.200 / 6.016 | 3.072 / 6.144 | 1.260 / 1.279 |
| bulk/1/512/1/0/recall-by-id | 1.216 / 1.728 | 1.280 / 1.856 | 1.952 / 3.200 | 1.920 / 3.072 | 0.623 / 0.667 |
| bulk/1/512/1/0/recent-for-agent | 4.032 / 9.472 | 4.096 / 8.960 | 3.264 / 6.272 | 3.072 / 6.016 | 1.235 / 1.362 |
| bulk/1/None/1/0/recall-by-id | 1.184 / 1.792 | 1.184 / 1.792 | 2.016 / 3.328 | 1.888 / 2.944 | 0.587 / 0.627 |
| bulk/1/None/1/0/recent-for-agent | 3.968 / 8.448 | 3.776 / 8.064 | 3.328 / 6.656 | 3.136 / 7.040 | 1.192 / 1.204 |
| normal/1/32/1/0/recall-by-id | 3.520 / 5.760 | 1.568 / 2.688 | 1.952 / 3.200 | 1.888 / 3.136 | 1.803 / 0.831 |
| normal/1/32/1/0/recent-for-agent | 4.480 / 8.448 | 3.968 / 7.808 | 3.200 / 6.144 | 3.072 / 5.760 | 1.400 / 1.292 |
| normal/1/512/1/0/recall-by-id | 1.920 / 2.944 | 1.504 / 2.560 | 1.984 / 3.328 | 1.984 / 3.264 | 0.808 / 0.758 |
| normal/1/512/1/0/recent-for-agent | 4.992 / 13.056 | 4.224 / 10.240 | 3.392 / 7.296 | 3.200 / 6.912 | 1.472 / 1.320 |
| normal/1/None/1/0/recall-by-id | 1.760 / 2.624 | 1.568 / 2.432 | 1.984 / 3.264 | 1.984 / 3.712 | 0.859 / 0.790 |
| normal/1/None/1/0/recent-for-agent | 4.608 / 9.472 | 4.224 / 9.984 | 3.328 / 6.784 | 3.264 / 8.064 | 1.396 / 1.294 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.968 [3.904, 4.224] | 6.784 [6.784, 7.168] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.952 [1.920, 2.624] | 3.200 [2.944, 4.352] |
| bulk/1/32/1/0/recall-by-id | rebased-kiban | 1.440 [1.280, 1.568] | 2.240 [2.016, 2.368] |
| bulk/1/32/1/0/recall-by-id | rebased-lmdb | 1.952 [1.888, 2.112] | 3.072 [3.072, 3.392] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 4.032 [4.032, 4.352] | 8.192 [7.296, 8.704] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 3.200 [3.200, 4.224] | 6.016 [6.016, 9.728] |
| bulk/1/32/1/0/recent-for-agent | rebased-kiban | 4.992 [3.840, 5.120] | 11.264 [7.296, 12.032] |
| bulk/1/32/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.008, 3.904] | 6.144 [5.504, 9.216] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.216 [1.184, 1.568] | 1.728 [1.632, 2.368] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.952 [1.920, 1.984] | 3.200 [2.944, 3.392] |
| bulk/1/512/1/0/recall-by-id | rebased-kiban | 1.280 [1.184, 1.344] | 1.856 [1.824, 2.176] |
| bulk/1/512/1/0/recall-by-id | rebased-lmdb | 1.920 [1.920, 1.920] | 3.072 [3.008, 3.264] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 4.032 [4.032, 4.096] | 9.472 [8.064, 9.728] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 3.264 [3.264, 3.328] | 6.272 [6.016, 7.424] |
| bulk/1/512/1/0/recent-for-agent | rebased-kiban | 4.096 [3.840, 4.224] | 8.960 [7.680, 9.472] |
| bulk/1/512/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.008, 3.200] | 6.016 [4.992, 9.728] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.184 [1.152, 1.184] | 1.792 [1.792, 1.856] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 2.016 [1.920, 3.008] | 3.328 [3.264, 4.352] |
| bulk/1/None/1/0/recall-by-id | rebased-kiban | 1.184 [1.184, 1.184] | 1.792 [1.664, 1.888] |
| bulk/1/None/1/0/recall-by-id | rebased-lmdb | 1.888 [1.856, 1.952] | 2.944 [2.752, 3.328] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.968 [3.968, 4.096] | 8.448 [7.808, 8.704] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 3.328 [3.328, 4.736] | 6.656 [6.144, 10.752] |
| bulk/1/None/1/0/recent-for-agent | rebased-kiban | 3.776 [3.776, 4.352] | 8.064 [7.296, 10.496] |
| bulk/1/None/1/0/recent-for-agent | rebased-lmdb | 3.136 [3.136, 3.584] | 7.040 [6.912, 7.296] |
| normal/1/32/1/0/recall-by-id | before-kiban | 3.520 [3.520, 3.584] | 5.760 [5.632, 6.016] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.952 [1.920, 2.016] | 3.200 [3.072, 3.264] |
| normal/1/32/1/0/recall-by-id | rebased-kiban | 1.568 [1.568, 1.664] | 2.688 [2.496, 2.752] |
| normal/1/32/1/0/recall-by-id | rebased-lmdb | 1.888 [1.888, 1.952] | 3.136 [2.816, 3.456] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 4.480 [4.480, 4.608] | 8.448 [7.680, 8.704] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 3.200 [3.200, 3.200] | 6.144 [5.504, 6.528] |
| normal/1/32/1/0/recent-for-agent | rebased-kiban | 3.968 [3.968, 4.224] | 7.808 [7.040, 11.008] |
| normal/1/32/1/0/recent-for-agent | rebased-lmdb | 3.072 [3.072, 3.072] | 5.760 [5.632, 6.528] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.920 [1.600, 2.016] | 2.944 [2.432, 3.200] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.984 [1.984, 2.496] | 3.328 [3.200, 4.032] |
| normal/1/512/1/0/recall-by-id | rebased-kiban | 1.504 [1.344, 1.568] | 2.560 [2.368, 2.688] |
| normal/1/512/1/0/recall-by-id | rebased-lmdb | 1.984 [1.952, 2.176] | 3.264 [3.072, 3.520] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 4.992 [4.864, 6.656] | 13.056 [11.776, 20.480] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 3.392 [3.328, 4.032] | 7.296 [6.784, 9.472] |
| normal/1/512/1/0/recent-for-agent | rebased-kiban | 4.224 [4.224, 4.352] | 10.240 [9.472, 11.008] |
| normal/1/512/1/0/recent-for-agent | rebased-lmdb | 3.200 [3.136, 3.456] | 6.912 [6.144, 7.168] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.760 [1.664, 1.984] | 2.624 [2.624, 2.816] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.984 [1.920, 2.048] | 3.264 [3.200, 3.264] |
| normal/1/None/1/0/recall-by-id | rebased-kiban | 1.568 [1.344, 1.792] | 2.432 [2.240, 2.688] |
| normal/1/None/1/0/recall-by-id | rebased-lmdb | 1.984 [1.952, 2.048] | 3.712 [3.456, 14.592] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 4.608 [4.608, 4.736] | 9.472 [8.960, 10.240] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 3.328 [3.264, 3.392] | 6.784 [6.528, 6.912] |
| normal/1/None/1/0/recent-for-agent | rebased-kiban | 4.224 [4.032, 5.120] | 9.984 [9.216, 13.824] |
| normal/1/None/1/0/recent-for-agent | rebased-lmdb | 3.264 [3.200, 3.264] | 8.064 [6.656, 9.216] |
