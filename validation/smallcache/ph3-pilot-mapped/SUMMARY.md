# Paired results

Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 3.264 / 5.376 | 1.152 / 1.536 | 1.472 / 2.176 | 1.504 / 2.240 | 2.318 / 0.795 |
| bulk/1/32/1/0/recent-for-agent | 3.520 / 6.912 | 3.136 / 6.016 | 2.432 / 4.096 | 2.560 / 4.352 | 1.417 / 1.184 |
| normal/1/32/1/0/recall-by-id | 2.624 / 3.520 | 1.248 / 1.600 | 1.440 / 1.888 | 1.440 / 2.176 | 1.867 / 0.886 |
| normal/1/32/1/0/recent-for-agent | 3.648 / 6.144 | 3.072 / 4.992 | 2.432 / 4.992 | 2.368 / 3.648 | 1.526 / 1.303 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.264 [2.752, 3.456] | 5.376 [4.096, 5.504] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.472 [1.408, 1.696] | 2.176 [1.920, 2.560] |
| bulk/1/32/1/0/recall-by-id | ph3-pilot-kiban | 1.152 [0.960, 1.312] | 1.536 [1.248, 1.824] |
| bulk/1/32/1/0/recall-by-id | ph3-pilot-lmdb | 1.504 [1.376, 1.536] | 2.240 [1.888, 2.688] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.520 [2.816, 3.776] | 6.912 [4.608, 15.872] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.432 [2.240, 2.944] | 4.096 [2.880, 5.760] |
| bulk/1/32/1/0/recent-for-agent | ph3-pilot-kiban | 3.136 [2.880, 3.264] | 6.016 [4.736, 7.296] |
| bulk/1/32/1/0/recent-for-agent | ph3-pilot-lmdb | 2.560 [2.176, 2.752] | 4.352 [3.328, 5.248] |
| normal/1/32/1/0/recall-by-id | before-kiban | 2.624 [2.240, 2.944] | 3.520 [3.200, 4.480] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.440 [1.344, 1.568] | 1.888 [1.792, 2.240] |
| normal/1/32/1/0/recall-by-id | ph3-pilot-kiban | 1.248 [1.152, 1.824] | 1.600 [1.504, 2.688] |
| normal/1/32/1/0/recall-by-id | ph3-pilot-lmdb | 1.440 [1.312, 1.824] | 2.176 [1.760, 2.752] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 3.648 [3.008, 3.776] | 6.144 [4.864, 6.272] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 2.432 [2.112, 2.688] | 4.992 [3.520, 5.760] |
| normal/1/32/1/0/recent-for-agent | ph3-pilot-kiban | 3.072 [2.752, 4.864] | 4.992 [4.480, 11.776] |
| normal/1/32/1/0/recent-for-agent | ph3-pilot-lmdb | 2.368 [2.112, 3.072] | 3.648 [3.328, 6.400] |
