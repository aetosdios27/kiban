# Paired results

Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | 3.840 / 5.632 | 3.840 / 5.504 | 1.408 / 1.888 | 1.440 / 2.112 | 2.636 / 2.652 |
| bulk/0/32/1/0/recent-for-agent | 3.072 / 4.992 | 3.264 / 5.248 | 2.496 / 4.352 | 2.560 / 4.352 | 1.231 / 1.282 |
| bulk/0/None/1/0/recall-by-id | 1.952 / 2.752 | 1.952 / 3.008 | 1.440 / 2.368 | 1.568 / 2.560 | 1.333 / 1.239 |
| bulk/0/None/1/0/recent-for-agent | 3.264 / 6.016 | 3.328 / 6.528 | 2.432 / 4.608 | 2.816 / 5.760 | 1.263 / 1.268 |
| bulk/1/32/1/0/recall-by-id | 4.224 / 6.272 | 1.696 / 2.304 | 1.664 / 2.624 | 1.632 / 2.624 | 2.581 / 1.065 |
| bulk/1/32/1/0/recent-for-agent | 3.456 / 5.760 | 3.264 / 6.016 | 2.880 / 5.888 | 2.944 / 4.608 | 1.222 / 1.159 |
| bulk/1/None/1/0/recall-by-id | 1.088 / 1.440 | 1.280 / 1.696 | 1.472 / 2.240 | 1.632 / 2.240 | 0.739 / 0.741 |
| bulk/1/None/1/0/recent-for-agent | 3.264 / 6.144 | 3.712 / 8.192 | 2.496 / 4.864 | 3.008 / 6.400 | 1.289 / 1.137 |
| normal/0/32/1/0/recall-by-id | 4.736 / 8.704 | 4.224 / 7.424 | 1.600 / 2.304 | 1.504 / 2.432 | 2.960 / 2.773 |
| normal/0/32/1/0/recent-for-agent | 4.224 / 7.296 | 4.352 / 8.704 | 2.944 / 5.632 | 2.688 / 5.632 | 1.526 / 1.575 |
| normal/0/None/1/0/recall-by-id | 2.176 / 3.584 | 2.112 / 3.392 | 1.536 / 2.368 | 1.536 / 2.176 | 1.417 / 1.388 |
| normal/0/None/1/0/recent-for-agent | 4.032 / 8.064 | 4.224 / 8.192 | 2.688 / 5.376 | 2.752 / 5.376 | 1.500 / 1.463 |
| normal/1/32/1/0/recall-by-id | 3.968 / 6.528 | 2.496 / 3.712 | 1.664 / 2.560 | 1.600 / 2.560 | 2.531 / 1.532 |
| normal/1/32/1/0/recent-for-agent | 4.032 / 8.704 | 4.096 / 7.808 | 2.816 / 6.528 | 2.880 / 6.528 | 1.412 / 1.429 |
| normal/1/None/1/0/recall-by-id | 1.472 / 2.240 | 1.408 / 1.888 | 1.504 / 2.048 | 1.472 / 2.048 | 0.936 / 0.904 |
| normal/1/None/1/0/recent-for-agent | 4.352 / 8.448 | 3.776 / 5.760 | 2.688 / 5.120 | 2.752 / 5.248 | 1.526 / 1.450 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/0/32/1/0/recall-by-id | before-kiban | 3.840 [3.648, 4.608] | 5.632 [5.248, 7.296] |
| bulk/0/32/1/0/recall-by-id | before-lmdb | 1.408 [1.408, 1.856] | 1.888 [1.856, 2.816] |
| bulk/0/32/1/0/recall-by-id | after-kiban | 3.840 [3.520, 3.968] | 5.504 [4.992, 6.528] |
| bulk/0/32/1/0/recall-by-id | after-lmdb | 1.440 [1.408, 1.504] | 2.112 [1.888, 2.304] |
| bulk/0/32/1/0/recent-for-agent | before-kiban | 3.072 [3.072, 3.776] | 4.992 [4.608, 8.192] |
| bulk/0/32/1/0/recent-for-agent | before-lmdb | 2.496 [2.432, 3.200] | 4.352 [3.904, 7.552] |
| bulk/0/32/1/0/recent-for-agent | after-kiban | 3.264 [3.072, 3.520] | 5.248 [4.608, 6.016] |
| bulk/0/32/1/0/recent-for-agent | after-lmdb | 2.560 [2.496, 2.624] | 4.352 [3.648, 4.608] |
| bulk/0/None/1/0/recall-by-id | before-kiban | 1.952 [1.728, 2.176] | 2.752 [2.432, 3.392] |
| bulk/0/None/1/0/recall-by-id | before-lmdb | 1.440 [1.408, 1.632] | 2.368 [1.856, 2.624] |
| bulk/0/None/1/0/recall-by-id | after-kiban | 1.952 [1.824, 2.880] | 3.008 [2.624, 4.608] |
| bulk/0/None/1/0/recall-by-id | after-lmdb | 1.568 [1.440, 1.632] | 2.560 [1.888, 2.688] |
| bulk/0/None/1/0/recent-for-agent | before-kiban | 3.264 [3.072, 3.520] | 6.016 [4.736, 7.424] |
| bulk/0/None/1/0/recent-for-agent | before-lmdb | 2.432 [2.368, 3.520] | 4.608 [3.520, 7.808] |
| bulk/0/None/1/0/recent-for-agent | after-kiban | 3.328 [3.136, 5.120] | 6.528 [4.992, 12.032] |
| bulk/0/None/1/0/recent-for-agent | after-lmdb | 2.816 [2.624, 3.008] | 5.760 [3.712, 6.400] |
| bulk/1/32/1/0/recall-by-id | before-kiban | 4.224 [3.648, 5.120] | 6.272 [5.760, 8.448] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.664 [1.376, 1.984] | 2.624 [1.856, 3.200] |
| bulk/1/32/1/0/recall-by-id | after-kiban | 1.696 [1.568, 1.920] | 2.304 [2.112, 2.624] |
| bulk/1/32/1/0/recall-by-id | after-lmdb | 1.632 [1.472, 1.792] | 2.624 [2.112, 2.880] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.456 [2.816, 4.608] | 5.760 [5.376, 11.008] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.880 [2.304, 3.392] | 5.888 [3.712, 7.936] |
| bulk/1/32/1/0/recent-for-agent | after-kiban | 3.264 [3.136, 3.456] | 6.016 [4.992, 6.528] |
| bulk/1/32/1/0/recent-for-agent | after-lmdb | 2.944 [2.560, 3.648] | 4.608 [4.096, 7.552] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.088 [1.024, 1.152] | 1.440 [1.312, 1.696] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.472 [1.408, 1.632] | 2.240 [1.888, 2.560] |
| bulk/1/None/1/0/recall-by-id | after-kiban | 1.280 [0.944, 1.408] | 1.696 [1.248, 2.112] |
| bulk/1/None/1/0/recall-by-id | after-lmdb | 1.632 [1.440, 1.760] | 2.240 [1.952, 2.944] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.264 [3.008, 3.328] | 6.144 [4.992, 7.040] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 2.496 [2.432, 2.816] | 4.864 [4.352, 6.528] |
| bulk/1/None/1/0/recent-for-agent | after-kiban | 3.712 [2.816, 3.840] | 8.192 [4.736, 8.704] |
| bulk/1/None/1/0/recent-for-agent | after-lmdb | 3.008 [2.560, 3.584] | 6.400 [4.352, 7.808] |
| normal/0/32/1/0/recall-by-id | before-kiban | 4.736 [3.840, 5.504] | 8.704 [6.400, 11.008] |
| normal/0/32/1/0/recall-by-id | before-lmdb | 1.600 [1.440, 1.792] | 2.304 [1.984, 3.264] |
| normal/0/32/1/0/recall-by-id | after-kiban | 4.224 [3.904, 6.016] | 7.424 [6.528, 12.032] |
| normal/0/32/1/0/recall-by-id | after-lmdb | 1.504 [1.408, 2.016] | 2.432 [1.888, 3.328] |
| normal/0/32/1/0/recent-for-agent | before-kiban | 4.224 [3.712, 5.376] | 7.296 [5.632, 12.800] |
| normal/0/32/1/0/recent-for-agent | before-lmdb | 2.944 [2.432, 3.008] | 5.632 [4.352, 7.296] |
| normal/0/32/1/0/recent-for-agent | after-kiban | 4.352 [3.712, 6.272] | 8.704 [6.400, 14.080] |
| normal/0/32/1/0/recent-for-agent | after-lmdb | 2.688 [2.496, 3.904] | 5.632 [3.840, 8.192] |
| normal/0/None/1/0/recall-by-id | before-kiban | 2.176 [1.920, 2.624] | 3.584 [2.880, 4.480] |
| normal/0/None/1/0/recall-by-id | before-lmdb | 1.536 [1.472, 1.632] | 2.368 [2.304, 2.432] |
| normal/0/None/1/0/recall-by-id | after-kiban | 2.112 [1.984, 2.240] | 3.392 [2.944, 3.712] |
| normal/0/None/1/0/recall-by-id | after-lmdb | 1.536 [1.376, 1.600] | 2.176 [1.856, 2.688] |
| normal/0/None/1/0/recent-for-agent | before-kiban | 4.032 [3.584, 4.864] | 8.064 [5.760, 11.008] |
| normal/0/None/1/0/recent-for-agent | before-lmdb | 2.688 [2.624, 2.752] | 5.376 [4.224, 6.016] |
| normal/0/None/1/0/recent-for-agent | after-kiban | 4.224 [3.840, 4.352] | 8.192 [5.504, 8.960] |
| normal/0/None/1/0/recent-for-agent | after-lmdb | 2.752 [2.432, 2.944] | 5.376 [3.520, 6.656] |
| normal/1/32/1/0/recall-by-id | before-kiban | 3.968 [3.840, 7.552] | 6.528 [5.760, 12.288] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.664 [1.504, 1.760] | 2.560 [2.048, 2.816] |
| normal/1/32/1/0/recall-by-id | after-kiban | 2.496 [2.304, 2.624] | 3.712 [3.200, 4.352] |
| normal/1/32/1/0/recall-by-id | after-lmdb | 1.600 [1.504, 1.728] | 2.560 [2.048, 2.752] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 4.032 [3.712, 6.144] | 8.704 [6.784, 14.592] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 2.816 [2.752, 3.264] | 6.528 [4.992, 9.728] |
| normal/1/32/1/0/recent-for-agent | after-kiban | 4.096 [3.904, 4.480] | 7.808 [6.144, 9.472] |
| normal/1/32/1/0/recent-for-agent | after-lmdb | 2.880 [2.752, 3.584] | 6.528 [4.736, 8.704] |
| normal/1/None/1/0/recall-by-id | before-kiban | 1.472 [1.312, 1.696] | 2.240 [1.760, 2.624] |
| normal/1/None/1/0/recall-by-id | before-lmdb | 1.504 [1.408, 1.728] | 2.048 [1.856, 3.008] |
| normal/1/None/1/0/recall-by-id | after-kiban | 1.408 [1.216, 1.600] | 1.888 [1.632, 2.432] |
| normal/1/None/1/0/recall-by-id | after-lmdb | 1.472 [1.440, 1.664] | 2.048 [1.920, 2.816] |
| normal/1/None/1/0/recent-for-agent | before-kiban | 4.352 [3.712, 4.608] | 8.448 [5.888, 8.704] |
| normal/1/None/1/0/recent-for-agent | before-lmdb | 2.688 [2.432, 2.944] | 5.120 [3.968, 7.040] |
| normal/1/None/1/0/recent-for-agent | after-kiban | 3.776 [3.648, 4.608] | 5.760 [5.504, 9.984] |
| normal/1/None/1/0/recent-for-agent | after-lmdb | 2.752 [2.560, 3.072] | 5.248 [3.968, 7.040] |
