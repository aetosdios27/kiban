# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | 3.584 / 6.144 | 1.184 / 1.600 | 1.728 / 2.560 | 1.728 / 2.688 | 2.074 / 0.685 |
| bulk/1/32/1/0/recent-for-agent | 3.840 / 8.960 | 3.712 / 8.704 | 3.008 / 6.144 | 2.816 / 6.016 | 1.277 / 1.326 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/32/1/0/recall-by-id | before-kiban | 3.584 [3.584, 3.776] | 6.144 [5.888, 6.912] |
| bulk/1/32/1/0/recall-by-id | before-lmdb | 1.728 [1.696, 1.856] | 2.560 [2.496, 3.136] |
| bulk/1/32/1/0/recall-by-id | rebased-kiban | 1.184 [1.184, 1.248] | 1.600 [1.504, 1.920] |
| bulk/1/32/1/0/recall-by-id | rebased-lmdb | 1.728 [1.696, 1.824] | 2.688 [2.496, 2.816] |
| bulk/1/32/1/0/recent-for-agent | before-kiban | 3.840 [3.584, 4.224] | 8.960 [7.936, 10.752] |
| bulk/1/32/1/0/recent-for-agent | before-lmdb | 3.008 [2.944, 3.200] | 6.144 [5.376, 7.296] |
| bulk/1/32/1/0/recent-for-agent | rebased-kiban | 3.712 [3.520, 4.032] | 8.704 [7.424, 9.728] |
| bulk/1/32/1/0/recent-for-agent | rebased-lmdb | 2.816 [2.752, 2.880] | 6.016 [5.376, 7.296] |
