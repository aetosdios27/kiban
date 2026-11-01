# Paired results

Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 1.152 / 1.536 | 1.008 / 1.344 | 1.312 / 1.792 | 1.408 / 2.016 | 0.857 / 0.732 |
| bulk/1/32/1/0/recent-for-agent | 2.752 / 4.736 | 3.072 / 6.144 | 2.304 / 4.032 | 2.304 / 3.648 | 1.222 / 1.283 |
| normal/1/32/1/0/recall-by-id | 1.152 / 1.536 | 1.088 / 1.600 | 1.312 / 1.888 | 1.376 / 1.824 | 0.829 / 0.791 |
| normal/1/32/1/0/recent-for-agent | 2.880 / 4.864 | 2.944 / 6.272 | 2.240 / 4.096 | 2.240 / 3.456 | 1.257 / 1.286 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 1.152 [0.960, 1.344] | 1.536 [1.248, 1.760] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.312 [1.248, 1.568] | 1.792 [1.728, 2.112] |
| bulk/1/32/1/0/recall-by-id | ph3-nobloom-kiban | 1.008 [0.864, 1.312] | 1.344 [1.152, 1.824] |
| bulk/1/32/1/0/recall-by-id | ph3-nobloom-lmdb | 1.408 [1.312, 1.792] | 2.016 [1.760, 2.688] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 2.752 [2.752, 3.392] | 4.736 [4.480, 5.248] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.304 [2.176, 2.816] | 4.032 [3.648, 4.352] |
| bulk/1/32/1/0/recent-for-agent | ph3-nobloom-kiban | 3.072 [2.688, 4.352] | 6.144 [4.352, 7.936] |
| bulk/1/32/1/0/recent-for-agent | ph3-nobloom-lmdb | 2.304 [2.176, 3.392] | 3.648 [3.456, 6.016] |
| normal/1/32/1/0/recall-by-id | before-kiban | 1.152 [1.088, 1.344] | 1.536 [1.440, 1.728] |
| normal/1/32/1/0/recall-by-id | before-lmdb | 1.312 [1.312, 1.472] | 1.888 [1.760, 2.112] |
| normal/1/32/1/0/recall-by-id | ph3-nobloom-kiban | 1.088 [0.912, 1.504] | 1.600 [1.312, 2.560] |
| normal/1/32/1/0/recall-by-id | ph3-nobloom-lmdb | 1.376 [1.280, 1.536] | 1.824 [1.696, 2.176] |
| normal/1/32/1/0/recent-for-agent | before-kiban | 2.880 [2.816, 3.328] | 4.864 [4.864, 6.016] |
| normal/1/32/1/0/recent-for-agent | before-lmdb | 2.240 [2.240, 2.816] | 4.096 [3.648, 4.736] |
| normal/1/32/1/0/recent-for-agent | ph3-nobloom-kiban | 2.944 [2.816, 4.096] | 6.272 [4.480, 7.680] |
| normal/1/32/1/0/recent-for-agent | ph3-nobloom-lmdb | 2.240 [2.240, 2.752] | 3.456 [3.392, 5.504] |
