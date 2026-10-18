# Paired results

Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | 3.136 / 4.608 | 3.136 / 4.736 | 1.504 / 2.048 | 1.504 / 2.112 | 2.085 / 2.085 |
| bulk/0/32/1/0/recent-for-agent | 3.264 / 6.144 | 3.392 / 5.376 | 2.496 / 3.968 | 2.752 / 4.608 | 1.308 / 1.233 |
| bulk/0/512/1/0/recall-by-id | 1.760 / 2.496 | 1.728 / 2.432 | 1.440 / 1.952 | 1.408 / 1.920 | 1.244 / 1.227 |
| bulk/0/512/1/0/recent-for-agent | 2.944 / 4.736 | 3.072 / 4.864 | 2.432 / 3.904 | 2.560 / 4.032 | 1.211 / 1.200 |
| bulk/0/None/1/0/recall-by-id | 1.952 / 2.752 | 1.920 / 2.752 | 1.504 / 2.016 | 1.536 / 2.176 | 1.256 / 1.200 |
| bulk/0/None/1/0/recent-for-agent | 3.200 / 6.016 | 3.328 / 5.376 | 2.624 / 5.376 | 2.752 / 4.096 | 1.357 / 1.225 |
| bulk/1/32/1/0/recall-by-id | 3.200 / 4.736 | 0.944 / 1.280 | 1.472 / 2.112 | 1.440 / 2.016 | 2.174 / 0.641 |
| bulk/1/32/1/0/recent-for-agent | 3.136 / 4.736 | 3.008 / 4.864 | 2.368 / 3.968 | 2.560 / 3.968 | 1.294 / 1.211 |
| bulk/1/512/1/0/recall-by-id | 1.056 / 1.408 | 0.944 / 1.216 | 1.440 / 1.952 | 1.504 / 1.984 | 0.733 / 0.641 |
| bulk/1/512/1/0/recent-for-agent | 3.200 / 5.504 | 3.264 / 5.120 | 2.496 / 4.096 | 2.752 / 4.480 | 1.256 / 1.186 |
| bulk/1/None/1/0/recall-by-id | 1.088 / 1.504 | 0.960 / 1.344 | 1.440 / 1.920 | 1.568 / 2.304 | 0.745 / 0.618 |
| bulk/1/None/1/0/recent-for-agent | 3.136 / 5.376 | 3.328 / 5.760 | 2.496 / 3.712 | 2.880 / 4.736 | 1.324 / 1.156 |
| normal/0/32/1/0/recall-by-id | 3.136 / 5.504 | 3.072 / 5.248 | 1.504 / 2.112 | 1.472 / 1.984 | 2.089 / 2.091 |
| normal/0/32/1/0/recent-for-agent | 3.584 / 6.912 | 3.776 / 6.528 | 2.432 / 4.352 | 2.624 / 4.224 | 1.474 / 1.439 |
| normal/0/512/1/0/recall-by-id | 1.984 / 3.072 | 1.888 / 2.944 | 1.440 / 1.984 | 1.472 / 2.016 | 1.333 / 1.341 |
| normal/0/512/1/0/recent-for-agent | 3.584 / 6.272 | 3.520 / 6.144 | 2.304 / 4.224 | 2.624 / 4.352 | 1.500 / 1.375 |
| normal/0/None/1/0/recall-by-id | 1.824 / 2.752 | 1.856 / 2.816 | 1.376 / 1.856 | 1.376 / 1.824 | 1.357 / 1.318 |
| normal/0/None/1/0/recent-for-agent | 3.392 / 5.504 | 3.584 / 5.632 | 2.240 / 3.776 | 2.432 / 3.968 | 1.529 / 1.459 |
| normal/1/32/1/0/recall-by-id | 2.368 / 4.096 | 1.120 / 1.664 | 1.376 / 1.920 | 1.472 / 2.048 | 1.721 / 0.778 |
| normal/1/32/1/0/recent-for-agent | 3.584 / 6.400 | 3.200 / 5.376 | 2.240 / 3.904 | 2.624 / 4.224 | 1.514 / 1.220 |
| normal/1/512/1/0/recall-by-id | 1.248 / 1.696 | 1.008 / 1.312 | 1.376 / 1.920 | 1.440 / 1.952 | 0.913 / 0.750 |
| normal/1/512/1/0/recent-for-agent | 3.392 / 6.016 | 3.008 / 5.248 | 2.176 / 3.712 | 2.496 / 4.224 | 1.529 / 1.205 |
| normal/1/None/1/0/recall-by-id | 1.152 / 1.568 | 1.024 / 1.344 | 1.376 / 1.856 | 1.408 / 1.888 | 0.878 / 0.716 |
| normal/1/None/1/0/recent-for-agent | 3.264 / 6.016 | 3.072 / 5.120 | 2.304 / 3.776 | 2.432 / 3.840 | 1.500 / 1.263 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | before-kiban | 3.136 [2.944, 3.712] | 4.608 [4.480, 6.272] |
| bulk/0/32/1/0/recall-by-id | before-lmdb | 1.504 [1.504, 1.632] | 2.048 [2.016, 2.752] |
| bulk/0/32/1/0/recall-by-id | ph3-kiban | 3.136 [3.008, 3.328] | 4.736 [4.352, 4.992] |
| bulk/0/32/1/0/recall-by-id | ph3-lmdb | 1.504 [1.408, 1.568] | 2.112 [1.824, 2.304] |
| bulk/0/32/1/0/recent-for-agent | before-kiban | 3.264 [3.200, 3.968] | 6.144 [4.608, 7.040] |
| bulk/0/32/1/0/recent-for-agent | before-lmdb | 2.496 [2.496, 2.816] | 3.968 [3.520, 5.120] |
| bulk/0/32/1/0/recent-for-agent | ph3-kiban | 3.392 [3.328, 3.520] | 5.376 [5.120, 6.912] |
| bulk/0/32/1/0/recent-for-agent | ph3-lmdb | 2.752 [2.688, 3.008] | 4.608 [3.840, 6.144] |
| bulk/0/512/1/0/recall-by-id | before-kiban | 1.760 [1.728, 1.792] | 2.496 [2.432, 2.496] |
| bulk/0/512/1/0/recall-by-id | before-lmdb | 1.440 [1.344, 1.472] | 1.952 [1.792, 2.048] |
| bulk/0/512/1/0/recall-by-id | ph3-kiban | 1.728 [1.728, 1.792] | 2.432 [2.368, 2.624] |
| bulk/0/512/1/0/recall-by-id | ph3-lmdb | 1.408 [1.344, 1.472] | 1.920 [1.760, 2.176] |
| bulk/0/512/1/0/recent-for-agent | before-kiban | 2.944 [2.816, 3.008] | 4.736 [4.224, 5.120] |
| bulk/0/512/1/0/recent-for-agent | before-lmdb | 2.432 [2.240, 2.432] | 3.904 [3.584, 4.096] |
| bulk/0/512/1/0/recent-for-agent | ph3-kiban | 3.072 [3.008, 3.200] | 4.864 [4.608, 5.120] |
| bulk/0/512/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.496, 2.688] | 4.032 [3.904, 4.992] |
| bulk/0/None/1/0/recall-by-id | before-kiban | 1.952 [1.728, 2.432] | 2.752 [2.432, 3.648] |
| bulk/0/None/1/0/recall-by-id | before-lmdb | 1.504 [1.376, 1.664] | 2.016 [1.792, 2.496] |
| bulk/0/None/1/0/recall-by-id | ph3-kiban | 1.920 [1.728, 2.176] | 2.752 [2.368, 3.008] |
| bulk/0/None/1/0/recall-by-id | ph3-lmdb | 1.536 [1.472, 1.600] | 2.176 [1.984, 2.240] |
| bulk/0/None/1/0/recent-for-agent | before-kiban | 3.200 [3.136, 3.648] | 6.016 [4.864, 7.936] |
| bulk/0/None/1/0/recent-for-agent | before-lmdb | 2.624 [2.304, 2.944] | 5.376 [3.648, 6.400] |
| bulk/0/None/1/0/recent-for-agent | ph3-kiban | 3.328 [3.136, 3.712] | 5.376 [4.992, 7.552] |
| bulk/0/None/1/0/recent-for-agent | ph3-lmdb | 2.752 [2.560, 3.072] | 4.096 [3.712, 5.760] |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.200 [2.752, 3.712] | 4.736 [4.096, 6.144] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.472 [1.344, 1.600] | 2.112 [1.824, 2.304] |
| bulk/1/32/1/0/recall-by-id | ph3-kiban | 0.944 [0.848, 1.024] | 1.280 [1.152, 1.376] |
| bulk/1/32/1/0/recall-by-id | ph3-lmdb | 1.440 [1.440, 1.472] | 2.016 [1.920, 2.304] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.136 [2.752, 4.032] | 4.736 [4.608, 8.448] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.368 [2.176, 2.816] | 3.968 [3.456, 4.864] |
| bulk/1/32/1/0/recent-for-agent | ph3-kiban | 3.008 [2.880, 3.264] | 4.864 [4.480, 5.248] |
| bulk/1/32/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.368, 2.752] | 3.968 [3.776, 4.992] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.056 [0.976, 1.088] | 1.408 [1.280, 1.536] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.440 [1.440, 1.632] | 1.952 [1.920, 2.368] |
| bulk/1/512/1/0/recall-by-id | ph3-kiban | 0.944 [0.912, 1.008] | 1.216 [1.120, 1.344] |
| bulk/1/512/1/0/recall-by-id | ph3-lmdb | 1.504 [1.440, 1.664] | 1.984 [1.888, 2.304] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 3.200 [3.008, 3.200] | 5.504 [4.736, 6.016] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 2.496 [2.496, 2.688] | 4.096 [3.968, 4.736] |
| bulk/1/512/1/0/recent-for-agent | ph3-kiban | 3.264 [3.200, 3.392] | 5.120 [4.992, 5.248] |
| bulk/1/512/1/0/recent-for-agent | ph3-lmdb | 2.752 [2.624, 3.072] | 4.480 [4.224, 4.992] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.088 [0.992, 1.120] | 1.504 [1.248, 1.536] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.440 [1.408, 1.504] | 1.920 [1.856, 2.112] |
| bulk/1/None/1/0/recall-by-id | ph3-kiban | 0.960 [0.944, 1.088] | 1.344 [1.248, 1.632] |
| bulk/1/None/1/0/recall-by-id | ph3-lmdb | 1.568 [1.440, 1.760] | 2.304 [1.888, 2.688] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.136 [3.072, 3.584] | 5.376 [4.864, 7.424] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 2.496 [2.368, 2.560] | 3.712 [3.712, 4.608] |
| bulk/1/None/1/0/recent-for-agent | ph3-kiban | 3.328 [3.200, 3.712] | 5.760 [4.480, 6.912] |
| bulk/1/None/1/0/recent-for-agent | ph3-lmdb | 2.880 [2.624, 3.200] | 4.736 [3.968, 6.912] |
| normal/0/32/1/0/recall-by-id | before-kiban | 3.136 [2.816, 3.328] | 5.504 [4.608, 5.632] |
| normal/0/32/1/0/recall-by-id | before-lmdb | 1.504 [1.440, 1.504] | 2.112 [1.952, 4.096] |
| normal/0/32/1/0/recall-by-id | ph3-kiban | 3.072 [2.944, 3.264] | 5.248 [4.736, 5.632] |
| normal/0/32/1/0/recall-by-id | ph3-lmdb | 1.472 [1.408, 1.504] | 1.984 [1.856, 2.176] |
| normal/0/32/1/0/recent-for-agent | before-kiban | 3.584 [3.392, 4.224] | 6.912 [5.376, 8.192] |
| normal/0/32/1/0/recent-for-agent | before-lmdb | 2.432 [2.368, 2.624] | 4.352 [3.968, 6.784] |
| normal/0/32/1/0/recent-for-agent | ph3-kiban | 3.776 [3.456, 4.096] | 6.528 [5.504, 6.784] |
| normal/0/32/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.496, 2.688] | 4.224 [3.904, 4.480] |
| normal/0/512/1/0/recall-by-id | before-kiban | 1.984 [1.888, 2.176] | 3.072 [3.008, 3.392] |
| normal/0/512/1/0/recall-by-id | before-lmdb | 1.440 [1.376, 1.632] | 1.984 [1.824, 2.304] |
| normal/0/512/1/0/recall-by-id | ph3-kiban | 1.888 [1.824, 2.304] | 2.944 [2.816, 3.904] |
| normal/0/512/1/0/recall-by-id | ph3-lmdb | 1.472 [1.408, 1.632] | 2.016 [1.856, 2.688] |
| normal/0/512/1/0/recent-for-agent | before-kiban | 3.584 [3.328, 3.840] | 6.272 [5.760, 6.912] |
| normal/0/512/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.752] | 4.224 [3.520, 4.864] |
| normal/0/512/1/0/recent-for-agent | ph3-kiban | 3.520 [3.456, 3.968] | 6.144 [5.504, 7.424] |
| normal/0/512/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.432, 3.328] | 4.352 [3.968, 6.912] |
| normal/0/None/1/0/recall-by-id | before-kiban | 1.824 [1.792, 1.952] | 2.752 [2.752, 3.136] |
| normal/0/None/1/0/recall-by-id | before-lmdb | 1.376 [1.344, 1.536] | 1.856 [1.792, 2.368] |
| normal/0/None/1/0/recall-by-id | ph3-kiban | 1.856 [1.792, 1.920] | 2.816 [2.688, 2.944] |
| normal/0/None/1/0/recall-by-id | ph3-lmdb | 1.376 [1.344, 1.472] | 1.824 [1.760, 2.048] |
| normal/0/None/1/0/recent-for-agent | before-kiban | 3.392 [3.264, 3.456] | 5.504 [4.992, 6.272] |
| normal/0/None/1/0/recent-for-agent | before-lmdb | 2.240 [2.176, 2.368] | 3.776 [3.584, 3.968] |
| normal/0/None/1/0/recent-for-agent | ph3-kiban | 3.584 [3.456, 3.648] | 5.632 [5.376, 5.888] |
| normal/0/None/1/0/recent-for-agent | ph3-lmdb | 2.432 [2.368, 2.624] | 3.968 [3.712, 4.480] |
| normal/1/32/1/0/recall-by-id | before-kiban | 2.368 [2.368, 2.816] | 4.096 [3.328, 4.480] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.376 [1.376, 1.504] | 1.920 [1.792, 2.112] |
| normal/1/32/1/0/recall-by-id | ph3-kiban | 1.120 [1.056, 1.152] | 1.664 [1.504, 1.696] |
| normal/1/32/1/0/recall-by-id | ph3-lmdb | 1.472 [1.440, 1.504] | 2.048 [1.952, 2.624] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 3.584 [3.264, 4.096] | 6.400 [5.504, 19.456] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 2.240 [2.176, 2.560] | 3.904 [3.520, 5.376] |
| normal/1/32/1/0/recent-for-agent | ph3-kiban | 3.200 [3.072, 3.264] | 5.376 [4.992, 5.888] |
| normal/1/32/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.496, 2.816] | 4.224 [3.904, 5.248] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.248 [1.120, 1.344] | 1.696 [1.536, 1.824] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.376 [1.312, 1.568] | 1.920 [1.728, 2.176] |
| normal/1/512/1/0/recall-by-id | ph3-kiban | 1.008 [0.976, 1.120] | 1.312 [1.280, 1.504] |
| normal/1/512/1/0/recall-by-id | ph3-lmdb | 1.440 [1.344, 1.472] | 1.952 [1.792, 1.984] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 3.392 [3.200, 3.584] | 6.016 [5.376, 6.272] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 2.176 [2.176, 2.496] | 3.712 [3.456, 4.352] |
| normal/1/512/1/0/recent-for-agent | ph3-kiban | 3.008 [3.008, 3.392] | 5.248 [4.736, 5.888] |
| normal/1/512/1/0/recent-for-agent | ph3-lmdb | 2.496 [2.432, 2.624] | 4.224 [3.840, 4.224] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.152 [1.152, 1.472] | 1.568 [1.504, 1.984] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.376 [1.312, 1.408] | 1.856 [1.760, 1.920] |
| normal/1/None/1/0/recall-by-id | ph3-kiban | 1.024 [0.992, 1.024] | 1.344 [1.312, 1.376] |
| normal/1/None/1/0/recall-by-id | ph3-lmdb | 1.408 [1.344, 1.504] | 1.888 [1.760, 2.112] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 3.264 [3.136, 3.776] | 6.016 [5.376, 7.296] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.304] | 3.776 [3.648, 3.968] |
| normal/1/None/1/0/recent-for-agent | ph3-kiban | 3.072 [2.880, 3.648] | 5.120 [4.864, 8.960] |
| normal/1/None/1/0/recent-for-agent | ph3-lmdb | 2.432 [2.368, 3.136] | 3.840 [3.648, 20.480] |
