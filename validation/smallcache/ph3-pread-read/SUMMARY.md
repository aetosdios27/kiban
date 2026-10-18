# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | 3.712 / 5.376 | 3.968 / 5.760 | 1.408 / 1.920 | 1.472 / 2.016 | 2.683 / 2.571 |
| bulk/0/32/1/0/recent-for-agent | 2.880 / 5.376 | 3.200 / 4.992 | 2.304 / 3.776 | 2.624 / 4.352 | 1.256 / 1.200 |
| bulk/0/512/1/0/recall-by-id | 1.920 / 2.944 | 1.888 / 2.816 | 1.440 / 2.176 | 1.440 / 2.304 | 1.319 / 1.318 |
| bulk/0/512/1/0/recent-for-agent | 2.944 / 6.016 | 3.072 / 5.888 | 2.304 / 4.864 | 2.560 / 4.992 | 1.257 / 1.211 |
| bulk/0/None/1/0/recall-by-id | 1.888 / 2.688 | 1.952 / 2.816 | 1.408 / 1.888 | 1.504 / 2.112 | 1.341 / 1.277 |
| bulk/0/None/1/0/recent-for-agent | 3.008 / 4.736 | 3.200 / 5.120 | 2.304 / 3.840 | 2.752 / 4.480 | 1.306 / 1.190 |
| bulk/1/32/1/0/recall-by-id | 3.968 / 6.272 | 1.504 / 2.304 | 1.472 / 2.016 | 1.408 / 1.888 | 2.696 / 1.024 |
| bulk/1/32/1/0/recent-for-agent | 3.008 / 5.888 | 3.072 / 5.376 | 2.304 / 3.904 | 2.560 / 4.224 | 1.306 / 1.211 |
| bulk/1/512/1/0/recall-by-id | 0.912 / 1.248 | 0.864 / 1.120 | 1.376 / 1.888 | 1.344 / 1.888 | 0.663 / 0.643 |
| bulk/1/512/1/0/recent-for-agent | 2.816 / 4.736 | 2.880 / 4.864 | 2.304 / 4.032 | 2.432 / 3.840 | 1.167 / 1.184 |
| bulk/1/None/1/0/recall-by-id | 1.120 / 1.440 | 0.976 / 1.248 | 1.536 / 2.240 | 1.568 / 2.240 | 0.720 / 0.622 |
| bulk/1/None/1/0/recent-for-agent | 3.136 / 5.504 | 3.328 / 6.016 | 2.560 / 4.480 | 2.624 / 4.608 | 1.225 / 1.182 |
| normal/0/32/1/0/recall-by-id | 3.968 / 6.400 | 3.968 / 6.400 | 1.408 / 1.984 | 1.440 / 2.112 | 2.783 / 2.844 |
| normal/0/32/1/0/recent-for-agent | 3.520 / 5.888 | 3.520 / 6.016 | 2.304 / 4.224 | 2.432 / 4.224 | 1.541 / 1.421 |
| normal/0/512/1/0/recall-by-id | 1.888 / 3.008 | 1.888 / 3.008 | 1.376 / 1.888 | 1.408 / 1.952 | 1.439 / 1.405 |
| normal/0/512/1/0/recent-for-agent | 3.392 / 6.016 | 3.200 / 5.760 | 2.048 / 3.584 | 2.368 / 3.776 | 1.531 / 1.429 |
| normal/0/None/1/0/recall-by-id | 1.984 / 3.328 | 1.856 / 2.880 | 1.440 / 1.952 | 1.408 / 2.048 | 1.400 / 1.349 |
| normal/0/None/1/0/recent-for-agent | 3.520 / 6.400 | 3.584 / 5.760 | 2.304 / 4.096 | 2.432 / 4.224 | 1.528 / 1.474 |
| normal/1/32/1/0/recall-by-id | 3.456 / 5.504 | 1.664 / 2.432 | 1.344 / 1.792 | 1.568 / 2.304 | 2.537 / 1.118 |
| normal/1/32/1/0/recent-for-agent | 3.392 / 6.400 | 3.008 / 5.632 | 2.240 / 4.480 | 2.624 / 4.736 | 1.455 / 1.178 |
| normal/1/512/1/0/recall-by-id | 1.184 / 1.600 | 1.056 / 1.440 | 1.440 / 1.952 | 1.344 / 1.792 | 0.800 / 0.756 |
| normal/1/512/1/0/recent-for-agent | 3.264 / 5.376 | 3.328 / 5.504 | 2.368 / 3.712 | 2.368 / 3.712 | 1.378 / 1.333 |
| normal/1/None/1/0/recall-by-id | 1.216 / 1.760 | 1.056 / 1.376 | 1.408 / 1.984 | 1.408 / 1.888 | 0.872 / 0.708 |
| normal/1/None/1/0/recent-for-agent | 3.264 / 5.504 | 3.136 / 4.992 | 2.240 / 3.968 | 2.624 / 4.480 | 1.429 / 1.186 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | before-kiban | 3.712 [3.520, 4.224] | 5.376 [4.864, 6.272] |
| bulk/0/32/1/0/recall-by-id | before-lmdb | 1.408 [1.312, 1.536] | 1.920 [1.728, 2.176] |
| bulk/0/32/1/0/recall-by-id | ph3-kiban | 3.968 [3.648, 4.224] | 5.760 [5.120, 6.784] |
| bulk/0/32/1/0/recall-by-id | ph3-lmdb | 1.472 [1.408, 1.568] | 2.016 [1.856, 2.176] |
| bulk/0/32/1/0/recent-for-agent | before-kiban | 2.880 [2.752, 3.456] | 5.376 [4.480, 5.888] |
| bulk/0/32/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.752] | 3.776 [3.520, 5.120] |
| bulk/0/32/1/0/recent-for-agent | ph3-kiban | 3.200 [3.072, 3.456] | 4.992 [4.864, 5.760] |
| bulk/0/32/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.496, 2.816] | 4.352 [4.032, 4.736] |
| bulk/0/512/1/0/recall-by-id | before-kiban | 1.920 [1.792, 2.176] | 2.944 [2.816, 3.200] |
| bulk/0/512/1/0/recall-by-id | before-lmdb | 1.440 [1.344, 1.536] | 2.176 [1.952, 2.560] |
| bulk/0/512/1/0/recall-by-id | ph3-kiban | 1.888 [1.760, 1.952] | 2.816 [2.752, 3.072] |
| bulk/0/512/1/0/recall-by-id | ph3-lmdb | 1.440 [1.408, 1.568] | 2.304 [2.112, 2.496] |
| bulk/0/512/1/0/recent-for-agent | before-kiban | 2.944 [2.816, 3.328] | 6.016 [5.376, 7.168] |
| bulk/0/512/1/0/recent-for-agent | before-lmdb | 2.304 [2.112, 2.560] | 4.864 [3.712, 5.248] |
| bulk/0/512/1/0/recent-for-agent | ph3-kiban | 3.072 [2.944, 3.328] | 5.888 [5.760, 6.272] |
| bulk/0/512/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.432, 2.688] | 4.992 [4.864, 5.376] |
| bulk/0/None/1/0/recall-by-id | before-kiban | 1.888 [1.664, 1.952] | 2.688 [2.432, 2.816] |
| bulk/0/None/1/0/recall-by-id | before-lmdb | 1.408 [1.344, 1.472] | 1.888 [1.888, 2.496] |
| bulk/0/None/1/0/recall-by-id | ph3-kiban | 1.952 [1.760, 2.048] | 2.816 [2.496, 3.008] |
| bulk/0/None/1/0/recall-by-id | ph3-lmdb | 1.504 [1.440, 1.664] | 2.112 [1.888, 2.432] |
| bulk/0/None/1/0/recent-for-agent | before-kiban | 3.008 [2.816, 3.072] | 4.736 [4.608, 5.248] |
| bulk/0/None/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.368] | 3.840 [3.776, 4.224] |
| bulk/0/None/1/0/recent-for-agent | ph3-kiban | 3.200 [2.944, 3.712] | 5.120 [4.480, 6.784] |
| bulk/0/None/1/0/recent-for-agent | ph3-lmdb | 2.752 [2.560, 2.880] | 4.480 [4.352, 4.992] |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.968 [3.776, 4.480] | 6.272 [6.144, 7.424] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.472 [1.408, 1.536] | 2.016 [1.888, 2.496] |
| bulk/1/32/1/0/recall-by-id | ph3-kiban | 1.504 [1.344, 1.664] | 2.304 [1.856, 3.072] |
| bulk/1/32/1/0/recall-by-id | ph3-lmdb | 1.408 [1.312, 1.536] | 1.888 [1.760, 2.432] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.008 [2.880, 3.648] | 5.888 [5.120, 7.040] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.304 [2.304, 2.496] | 3.904 [3.520, 5.504] |
| bulk/1/32/1/0/recent-for-agent | ph3-kiban | 3.072 [2.752, 3.328] | 5.376 [4.608, 5.888] |
| bulk/1/32/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.240, 2.688] | 4.224 [3.520, 5.120] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 0.912 [0.912, 0.976] | 1.248 [1.216, 1.280] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.376 [1.344, 1.472] | 1.888 [1.856, 2.240] |
| bulk/1/512/1/0/recall-by-id | ph3-kiban | 0.864 [0.816, 0.880] | 1.120 [1.088, 1.184] |
| bulk/1/512/1/0/recall-by-id | ph3-lmdb | 1.344 [1.312, 1.408] | 1.888 [1.728, 1.920] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 2.816 [2.624, 2.944] | 4.736 [4.608, 5.760] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.560] | 4.032 [3.968, 6.144] |
| bulk/1/512/1/0/recent-for-agent | ph3-kiban | 2.880 [2.880, 2.880] | 4.864 [4.480, 4.864] |
| bulk/1/512/1/0/recent-for-agent | ph3-lmdb | 2.432 [2.368, 2.496] | 3.840 [3.712, 4.608] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.120 [0.960, 1.312] | 1.440 [1.280, 1.952] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.536 [1.440, 1.952] | 2.240 [1.984, 16.896] |
| bulk/1/None/1/0/recall-by-id | ph3-kiban | 0.976 [0.912, 0.992] | 1.248 [1.184, 1.312] |
| bulk/1/None/1/0/recall-by-id | ph3-lmdb | 1.568 [1.376, 1.632] | 2.240 [1.888, 2.368] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.136 [2.816, 3.776] | 5.504 [4.736, 8.192] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 2.560 [2.368, 3.008] | 4.480 [3.968, 6.656] |
| bulk/1/None/1/0/recent-for-agent | ph3-kiban | 3.328 [2.944, 3.456] | 6.016 [5.248, 7.040] |
| bulk/1/None/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.624, 2.880] | 4.608 [4.096, 5.632] |
| normal/0/32/1/0/recall-by-id | before-kiban | 3.968 [3.840, 4.096] | 6.400 [6.272, 6.784] |
| normal/0/32/1/0/recall-by-id | before-lmdb | 1.408 [1.408, 1.472] | 1.984 [1.888, 2.176] |
| normal/0/32/1/0/recall-by-id | ph3-kiban | 3.968 [3.840, 4.352] | 6.400 [6.144, 7.296] |
| normal/0/32/1/0/recall-by-id | ph3-lmdb | 1.440 [1.344, 1.632] | 2.112 [1.856, 2.432] |
| normal/0/32/1/0/recent-for-agent | before-kiban | 3.520 [3.264, 3.648] | 5.888 [5.248, 6.400] |
| normal/0/32/1/0/recent-for-agent | before-lmdb | 2.304 [2.240, 2.368] | 4.224 [3.776, 4.352] |
| normal/0/32/1/0/recent-for-agent | ph3-kiban | 3.520 [3.456, 3.648] | 6.016 [5.632, 6.656] |
| normal/0/32/1/0/recent-for-agent | ph3-lmdb | 2.432 [2.432, 2.752] | 4.224 [3.840, 4.864] |
| normal/0/512/1/0/recall-by-id | before-kiban | 1.888 [1.824, 2.368] | 3.008 [2.752, 4.224] |
| normal/0/512/1/0/recall-by-id | before-lmdb | 1.376 [1.280, 1.632] | 1.888 [1.760, 2.240] |
| normal/0/512/1/0/recall-by-id | ph3-kiban | 1.888 [1.792, 2.176] | 3.008 [2.752, 3.456] |
| normal/0/512/1/0/recall-by-id | ph3-lmdb | 1.408 [1.280, 1.504] | 1.952 [1.920, 2.176] |
| normal/0/512/1/0/recent-for-agent | before-kiban | 3.392 [3.136, 3.456] | 6.016 [5.248, 6.912] |
| normal/0/512/1/0/recent-for-agent | before-lmdb | 2.048 [2.016, 2.496] | 3.584 [3.200, 3.904] |
| normal/0/512/1/0/recent-for-agent | ph3-kiban | 3.200 [3.200, 3.968] | 5.760 [5.248, 7.680] |
| normal/0/512/1/0/recent-for-agent | ph3-lmdb | 2.368 [2.240, 2.496] | 3.776 [3.648, 3.904] |
| normal/0/None/1/0/recall-by-id | before-kiban | 1.984 [1.920, 2.176] | 3.328 [2.944, 6.016] |
| normal/0/None/1/0/recall-by-id | before-lmdb | 1.440 [1.376, 1.440] | 1.952 [1.824, 2.048] |
| normal/0/None/1/0/recall-by-id | ph3-kiban | 1.856 [1.824, 2.112] | 2.880 [2.752, 3.520] |
| normal/0/None/1/0/recall-by-id | ph3-lmdb | 1.408 [1.376, 1.504] | 2.048 [1.856, 2.112] |
| normal/0/None/1/0/recent-for-agent | before-kiban | 3.520 [3.328, 3.712] | 6.400 [5.120, 7.168] |
| normal/0/None/1/0/recent-for-agent | before-lmdb | 2.304 [2.240, 2.304] | 4.096 [3.840, 4.608] |
| normal/0/None/1/0/recent-for-agent | ph3-kiban | 3.584 [3.456, 3.968] | 5.760 [5.504, 8.448] |
| normal/0/None/1/0/recent-for-agent | ph3-lmdb | 2.432 [2.304, 2.624] | 4.224 [3.840, 5.120] |
| normal/1/32/1/0/recall-by-id | before-kiban | 3.456 [3.328, 4.032] | 5.504 [4.736, 7.040] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.344 [1.312, 1.632] | 1.792 [1.760, 2.752] |
| normal/1/32/1/0/recall-by-id | ph3-kiban | 1.664 [1.504, 1.824] | 2.432 [2.112, 3.200] |
| normal/1/32/1/0/recall-by-id | ph3-lmdb | 1.568 [1.376, 1.632] | 2.304 [1.856, 2.688] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 3.392 [3.072, 3.712] | 6.400 [4.864, 8.192] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 2.240 [2.112, 2.688] | 4.480 [3.840, 6.400] |
| normal/1/32/1/0/recent-for-agent | ph3-kiban | 3.008 [2.944, 3.392] | 5.632 [4.736, 7.680] |
| normal/1/32/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.432, 2.880] | 4.736 [4.032, 6.400] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.184 [1.152, 1.248] | 1.600 [1.504, 1.696] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.440 [1.312, 1.664] | 1.952 [1.760, 2.688] |
| normal/1/512/1/0/recall-by-id | ph3-kiban | 1.056 [0.976, 1.088] | 1.440 [1.280, 1.664] |
| normal/1/512/1/0/recall-by-id | ph3-lmdb | 1.344 [1.344, 1.440] | 1.792 [1.792, 1.952] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 3.264 [3.200, 3.392] | 5.376 [4.992, 5.632] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 2.368 [2.240, 2.688] | 3.712 [3.648, 4.352] |
| normal/1/512/1/0/recent-for-agent | ph3-kiban | 3.328 [2.944, 3.456] | 5.504 [4.736, 6.784] |
| normal/1/512/1/0/recent-for-agent | ph3-lmdb | 2.368 [2.304, 2.560] | 3.712 [3.648, 4.608] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.216 [1.152, 1.312] | 1.760 [1.568, 1.888] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.408 [1.344, 1.504] | 1.984 [1.888, 2.304] |
| normal/1/None/1/0/recall-by-id | ph3-kiban | 1.056 [0.960, 1.120] | 1.376 [1.280, 1.504] |
| normal/1/None/1/0/recall-by-id | ph3-lmdb | 1.408 [1.376, 1.536] | 1.888 [1.856, 2.240] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 3.264 [3.136, 3.456] | 5.504 [5.376, 6.912] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 2.240 [2.112, 2.432] | 3.968 [3.776, 4.608] |
| normal/1/None/1/0/recent-for-agent | ph3-kiban | 3.136 [2.944, 3.264] | 4.992 [4.736, 5.376] |
| normal/1/None/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.496, 2.752] | 4.480 [3.712, 5.504] |
