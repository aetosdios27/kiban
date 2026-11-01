# Paired results

Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | 2.816 / 4.352 | 3.008 / 4.736 | 1.440 / 2.112 | 1.472 / 2.112 | 2.000 / 2.000 |
| bulk/0/32/1/0/recent-for-agent | 2.624 / 4.352 | 3.072 / 5.376 | 2.304 / 3.840 | 2.560 / 4.352 | 1.242 / 1.286 |
| bulk/0/512/1/0/recall-by-id | 2.016 / 2.816 | 1.952 / 2.688 | 1.472 / 1.984 | 1.440 / 1.952 | 1.340 / 1.378 |
| bulk/0/512/1/0/recent-for-agent | 3.264 / 5.376 | 3.392 / 5.120 | 2.496 / 3.776 | 2.688 / 4.032 | 1.308 / 1.333 |
| bulk/0/None/1/0/recall-by-id | 1.728 / 2.496 | 2.016 / 2.880 | 1.472 / 2.112 | 1.472 / 2.016 | 1.216 / 1.400 |
| bulk/0/None/1/0/recent-for-agent | 2.880 / 4.736 | 3.520 / 6.784 | 2.432 / 3.904 | 2.688 / 5.120 | 1.194 / 1.250 |
| bulk/1/32/1/0/recall-by-id | 2.752 / 4.224 | 1.056 / 1.376 | 1.376 / 1.856 | 1.312 / 1.728 | 2.044 / 0.800 |
| bulk/1/32/1/0/recent-for-agent | 2.688 / 4.864 | 2.880 / 4.352 | 2.304 / 3.712 | 2.240 / 3.840 | 1.206 / 1.229 |
| bulk/1/512/1/0/recall-by-id | 1.152 / 1.472 | 1.152 / 1.472 | 1.504 / 2.048 | 1.568 / 2.176 | 0.706 / 0.735 |
| bulk/1/512/1/0/recent-for-agent | 3.136 / 5.632 | 3.328 / 5.760 | 2.752 / 4.608 | 2.816 / 4.608 | 1.196 / 1.140 |
| bulk/1/None/1/0/recall-by-id | 1.120 / 1.472 | 1.120 / 1.408 | 1.472 / 2.048 | 1.408 / 2.048 | 0.750 / 0.750 |
| bulk/1/None/1/0/recent-for-agent | 3.200 / 5.376 | 3.264 / 4.992 | 2.496 / 4.480 | 2.560 / 4.480 | 1.256 / 1.175 |
| normal/0/32/1/0/recall-by-id | 3.648 / 6.656 | 3.520 / 6.144 | 1.664 / 2.304 | 1.600 / 2.240 | 2.235 / 2.160 |
| normal/0/32/1/0/recent-for-agent | 4.608 / 7.808 | 4.608 / 8.064 | 2.880 / 5.376 | 3.136 / 6.400 | 1.565 / 1.458 |
| normal/0/512/1/0/recall-by-id | 2.016 / 3.008 | 1.920 / 2.944 | 1.472 / 2.048 | 1.472 / 1.952 | 1.400 / 1.289 |
| normal/0/512/1/0/recent-for-agent | 3.520 / 5.632 | 3.776 / 5.632 | 2.496 / 3.968 | 2.624 / 4.224 | 1.462 / 1.429 |
| normal/0/None/1/0/recall-by-id | 2.176 / 3.520 | 2.176 / 3.328 | 1.504 / 1.984 | 1.472 / 1.984 | 1.510 / 1.364 |
| normal/0/None/1/0/recent-for-agent | 4.032 / 6.912 | 4.032 / 6.656 | 2.624 / 4.096 | 2.752 / 4.224 | 1.591 / 1.462 |
| normal/1/32/1/0/recall-by-id | 2.496 / 3.520 | 1.344 / 1.696 | 1.440 / 1.984 | 1.472 / 1.984 | 1.689 / 0.930 |
| normal/1/32/1/0/recent-for-agent | 3.648 / 6.144 | 3.520 / 6.144 | 2.496 / 4.224 | 2.560 / 4.224 | 1.450 / 1.341 |
| normal/1/512/1/0/recall-by-id | 1.312 / 1.696 | 1.120 / 1.504 | 1.344 / 1.792 | 1.536 / 2.112 | 0.932 / 0.780 |
| normal/1/512/1/0/recent-for-agent | 3.328 / 6.016 | 3.136 / 5.760 | 2.112 / 3.456 | 2.624 / 4.352 | 1.492 / 1.149 |
| normal/1/None/1/0/recall-by-id | 1.248 / 1.664 | 1.088 / 1.440 | 1.472 / 2.112 | 1.344 / 1.824 | 0.800 / 0.756 |
| normal/1/None/1/0/recent-for-agent | 3.392 / 5.632 | 3.072 / 4.864 | 2.368 / 4.032 | 2.432 / 3.968 | 1.350 / 1.229 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | before-kiban | 2.816 [2.752, 2.880] | 4.352 [4.224, 4.480] |
| bulk/0/32/1/0/recall-by-id | before-lmdb | 1.440 [1.312, 1.824] | 2.112 [1.760, 2.624] |
| bulk/0/32/1/0/recall-by-id | ph3-kiban | 3.008 [2.880, 3.264] | 4.736 [4.352, 5.120] |
| bulk/0/32/1/0/recall-by-id | ph3-lmdb | 1.472 [1.312, 1.536] | 2.112 [1.856, 2.176] |
| bulk/0/32/1/0/recent-for-agent | before-kiban | 2.624 [2.624, 2.944] | 4.352 [3.968, 4.992] |
| bulk/0/32/1/0/recent-for-agent | before-lmdb | 2.304 [2.016, 3.328] | 3.840 [3.072, 6.912] |
| bulk/0/32/1/0/recent-for-agent | ph3-kiban | 3.072 [2.880, 3.456] | 5.376 [4.992, 5.632] |
| bulk/0/32/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.240, 2.944] | 4.352 [3.840, 5.120] |
| bulk/0/512/1/0/recall-by-id | before-kiban | 2.016 [1.824, 2.176] | 2.816 [2.560, 3.264] |
| bulk/0/512/1/0/recall-by-id | before-lmdb | 1.472 [1.408, 1.504] | 1.984 [1.888, 2.112] |
| bulk/0/512/1/0/recall-by-id | ph3-kiban | 1.952 [1.856, 2.176] | 2.688 [2.624, 3.136] |
| bulk/0/512/1/0/recall-by-id | ph3-lmdb | 1.440 [1.376, 1.472] | 1.952 [1.888, 2.016] |
| bulk/0/512/1/0/recent-for-agent | before-kiban | 3.264 [3.136, 3.776] | 5.376 [4.864, 6.400] |
| bulk/0/512/1/0/recent-for-agent | before-lmdb | 2.496 [2.432, 2.560] | 3.776 [3.712, 3.968] |
| bulk/0/512/1/0/recent-for-agent | ph3-kiban | 3.392 [3.264, 3.776] | 5.120 [4.864, 6.784] |
| bulk/0/512/1/0/recent-for-agent | ph3-lmdb | 2.688 [2.368, 2.752] | 4.032 [3.840, 4.352] |
| bulk/0/None/1/0/recall-by-id | before-kiban | 1.728 [1.664, 1.984] | 2.496 [2.432, 2.944] |
| bulk/0/None/1/0/recall-by-id | before-lmdb | 1.472 [1.344, 1.632] | 2.112 [1.792, 2.816] |
| bulk/0/None/1/0/recall-by-id | ph3-kiban | 2.016 [1.824, 2.432] | 2.880 [2.560, 3.712] |
| bulk/0/None/1/0/recall-by-id | ph3-lmdb | 1.472 [1.440, 1.600] | 2.016 [1.952, 2.240] |
| bulk/0/None/1/0/recent-for-agent | before-kiban | 2.880 [2.752, 3.328] | 4.736 [4.352, 5.760] |
| bulk/0/None/1/0/recent-for-agent | before-lmdb | 2.432 [2.176, 2.816] | 3.904 [3.840, 5.120] |
| bulk/0/None/1/0/recent-for-agent | ph3-kiban | 3.520 [3.200, 3.776] | 6.784 [5.504, 7.424] |
| bulk/0/None/1/0/recent-for-agent | ph3-lmdb | 2.688 [2.560, 3.072] | 5.120 [3.840, 7.296] |
| bulk/1/32/1/0/recall-by-id | before-kiban | 2.752 [2.624, 3.008] | 4.224 [3.840, 4.608] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.376 [1.344, 1.440] | 1.856 [1.760, 2.016] |
| bulk/1/32/1/0/recall-by-id | ph3-kiban | 1.056 [1.008, 1.152] | 1.376 [1.312, 1.440] |
| bulk/1/32/1/0/recall-by-id | ph3-lmdb | 1.312 [1.312, 1.472] | 1.728 [1.728, 1.920] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 2.688 [2.624, 3.264] | 4.864 [4.096, 5.120] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.496] | 3.712 [3.072, 4.608] |
| bulk/1/32/1/0/recent-for-agent | ph3-kiban | 2.880 [2.624, 3.328] | 4.352 [4.224, 5.504] |
| bulk/1/32/1/0/recent-for-agent | ph3-lmdb | 2.240 [2.240, 2.688] | 3.840 [3.264, 4.736] |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.152 [0.944, 1.248] | 1.472 [1.248, 1.632] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.504 [1.408, 1.632] | 2.048 [1.856, 2.240] |
| bulk/1/512/1/0/recall-by-id | ph3-kiban | 1.152 [0.992, 1.280] | 1.472 [1.312, 1.664] |
| bulk/1/512/1/0/recall-by-id | ph3-lmdb | 1.568 [1.504, 1.664] | 2.176 [2.112, 2.496] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 3.136 [2.816, 3.520] | 5.632 [4.864, 5.760] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 2.752 [2.432, 2.944] | 4.608 [3.712, 6.272] |
| bulk/1/512/1/0/recent-for-agent | ph3-kiban | 3.328 [3.136, 3.648] | 5.760 [5.120, 6.656] |
| bulk/1/512/1/0/recent-for-agent | ph3-lmdb | 2.816 [2.624, 3.200] | 4.608 [4.032, 5.760] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.120 [1.056, 1.344] | 1.472 [1.440, 1.984] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.472 [1.440, 1.792] | 2.048 [1.920, 2.880] |
| bulk/1/None/1/0/recall-by-id | ph3-kiban | 1.120 [0.928, 1.152] | 1.408 [1.184, 1.568] |
| bulk/1/None/1/0/recall-by-id | ph3-lmdb | 1.408 [1.312, 1.536] | 2.048 [1.760, 2.176] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.200 [3.008, 3.840] | 5.376 [4.864, 8.704] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 2.496 [2.368, 3.008] | 4.480 [3.904, 6.272] |
| bulk/1/None/1/0/recent-for-agent | ph3-kiban | 3.264 [2.816, 3.392] | 4.992 [4.608, 6.400] |
| bulk/1/None/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.368, 2.880] | 4.480 [3.904, 4.992] |
| normal/0/32/1/0/recall-by-id | before-kiban | 3.648 [3.520, 4.224] | 6.656 [6.016, 8.448] |
| normal/0/32/1/0/recall-by-id | before-lmdb | 1.664 [1.568, 1.760] | 2.304 [2.240, 2.432] |
| normal/0/32/1/0/recall-by-id | ph3-kiban | 3.520 [3.456, 3.648] | 6.144 [5.760, 7.424] |
| normal/0/32/1/0/recall-by-id | ph3-lmdb | 1.600 [1.600, 1.696] | 2.240 [2.176, 2.432] |
| normal/0/32/1/0/recent-for-agent | before-kiban | 4.608 [4.480, 4.608] | 7.808 [7.168, 10.240] |
| normal/0/32/1/0/recent-for-agent | before-lmdb | 2.880 [2.816, 3.072] | 5.376 [4.480, 6.400] |
| normal/0/32/1/0/recent-for-agent | ph3-kiban | 4.608 [4.480, 4.736] | 8.064 [6.912, 8.960] |
| normal/0/32/1/0/recent-for-agent | ph3-lmdb | 3.136 [3.072, 3.392] | 6.400 [4.864, 15.616] |
| normal/0/512/1/0/recall-by-id | before-kiban | 2.016 [1.920, 2.240] | 3.008 [2.880, 3.584] |
| normal/0/512/1/0/recall-by-id | before-lmdb | 1.472 [1.344, 1.600] | 2.048 [1.792, 2.432] |
| normal/0/512/1/0/recall-by-id | ph3-kiban | 1.920 [1.824, 2.016] | 2.944 [2.752, 3.008] |
| normal/0/512/1/0/recall-by-id | ph3-lmdb | 1.472 [1.408, 1.632] | 1.952 [1.856, 2.432] |
| normal/0/512/1/0/recent-for-agent | before-kiban | 3.520 [3.392, 4.736] | 5.632 [5.504, 8.960] |
| normal/0/512/1/0/recent-for-agent | before-lmdb | 2.496 [2.304, 2.624] | 3.968 [3.712, 5.248] |
| normal/0/512/1/0/recent-for-agent | ph3-kiban | 3.776 [3.520, 3.904] | 5.632 [5.504, 5.888] |
| normal/0/512/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.432, 2.816] | 4.224 [3.840, 4.352] |
| normal/0/None/1/0/recall-by-id | before-kiban | 2.176 [1.920, 2.432] | 3.520 [2.880, 3.904] |
| normal/0/None/1/0/recall-by-id | before-lmdb | 1.504 [1.376, 1.664] | 1.984 [1.856, 2.304] |
| normal/0/None/1/0/recall-by-id | ph3-kiban | 2.176 [1.856, 2.496] | 3.328 [2.816, 3.840] |
| normal/0/None/1/0/recall-by-id | ph3-lmdb | 1.472 [1.376, 1.696] | 1.984 [1.856, 2.496] |
| normal/0/None/1/0/recent-for-agent | before-kiban | 4.032 [3.456, 4.736] | 6.912 [5.376, 10.752] |
| normal/0/None/1/0/recent-for-agent | before-lmdb | 2.624 [2.304, 2.816] | 4.096 [3.904, 4.736] |
| normal/0/None/1/0/recent-for-agent | ph3-kiban | 4.032 [3.648, 4.864] | 6.656 [5.760, 10.752] |
| normal/0/None/1/0/recent-for-agent | ph3-lmdb | 2.752 [2.496, 3.072] | 4.224 [3.840, 4.736] |
| normal/1/32/1/0/recall-by-id | before-kiban | 2.496 [2.240, 2.560] | 3.520 [3.456, 3.840] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.440 [1.376, 1.504] | 1.984 [1.856, 2.112] |
| normal/1/32/1/0/recall-by-id | ph3-kiban | 1.344 [1.088, 1.664] | 1.696 [1.408, 2.496] |
| normal/1/32/1/0/recall-by-id | ph3-lmdb | 1.472 [1.312, 1.504] | 1.984 [1.792, 2.240] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 3.648 [3.200, 3.712] | 6.144 [5.632, 7.552] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 2.496 [2.176, 2.560] | 4.224 [3.712, 7.296] |
| normal/1/32/1/0/recent-for-agent | ph3-kiban | 3.520 [2.816, 3.648] | 6.144 [5.376, 7.040] |
| normal/1/32/1/0/recent-for-agent | ph3-lmdb | 2.560 [2.240, 2.688] | 4.224 [3.136, 4.608] |
| normal/1/512/1/0/recall-by-id | before-kiban | 1.312 [1.152, 1.408] | 1.696 [1.504, 3.904] |
| normal/1/512/1/0/recall-by-id | before-lmdb | 1.344 [1.312, 1.408] | 1.792 [1.760, 1.952] |
| normal/1/512/1/0/recall-by-id | ph3-kiban | 1.120 [1.024, 1.344] | 1.504 [1.312, 1.760] |
| normal/1/512/1/0/recall-by-id | ph3-lmdb | 1.536 [1.312, 1.600] | 2.112 [1.792, 2.176] |
| normal/1/512/1/0/recent-for-agent | before-kiban | 3.328 [3.008, 3.648] | 6.016 [4.864, 6.656] |
| normal/1/512/1/0/recent-for-agent | before-lmdb | 2.112 [2.016, 2.560] | 3.456 [3.328, 4.480] |
| normal/1/512/1/0/recent-for-agent | ph3-kiban | 3.136 [2.752, 3.712] | 5.760 [4.352, 7.552] |
| normal/1/512/1/0/recent-for-agent | ph3-lmdb | 2.624 [2.240, 3.008] | 4.352 [3.520, 5.760] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.248 [1.152, 1.280] | 1.664 [1.568, 1.696] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.472 [1.312, 1.600] | 2.112 [1.760, 2.624] |
| normal/1/None/1/0/recall-by-id | ph3-kiban | 1.088 [0.992, 1.088] | 1.440 [1.248, 1.536] |
| normal/1/None/1/0/recall-by-id | ph3-lmdb | 1.344 [1.248, 1.760] | 1.824 [1.696, 2.560] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 3.392 [3.072, 3.520] | 5.632 [4.992, 5.632] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 2.368 [2.176, 2.816] | 4.032 [3.648, 6.272] |
| normal/1/None/1/0/recent-for-agent | ph3-kiban | 3.072 [2.752, 3.264] | 4.864 [4.480, 6.144] |
| normal/1/None/1/0/recent-for-agent | ph3-lmdb | 2.432 [2.176, 3.136] | 3.968 [3.584, 5.888] |
