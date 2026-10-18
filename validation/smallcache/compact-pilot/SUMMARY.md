# Paired results

Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 3.712 / 5.504 | 1.536 / 2.176 | 1.440 / 1.984 | 1.344 / 1.824 | 2.571 / 1.127 |
| bulk/1/32/1/0/recent-for-agent | 3.072 / 4.352 | 2.816 / 4.480 | 2.368 / 3.840 | 2.304 / 3.200 | 1.244 / 1.194 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.712 [3.456, 4.032] | 5.504 [4.864, 6.400] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.440 [1.344, 1.536] | 1.984 [1.856, 2.304] |
| bulk/1/32/1/0/recall-by-id | compact-kiban | 1.536 [1.504, 1.984] | 2.176 [1.984, 2.944] |
| bulk/1/32/1/0/recall-by-id | compact-lmdb | 1.344 [1.344, 1.760] | 1.824 [1.760, 2.752] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.072 [2.688, 3.392] | 4.352 [3.456, 6.400] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 2.368 [2.112, 2.624] | 3.840 [2.944, 4.608] |
| bulk/1/32/1/0/recent-for-agent | compact-kiban | 2.816 [2.752, 3.712] | 4.480 [3.712, 6.400] |
| bulk/1/32/1/0/recent-for-agent | compact-lmdb | 2.304 [2.240, 3.392] | 3.200 [2.752, 6.784] |
