# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/1/512/1/0/recall-by-id | 1.216 / 1.632 | 1.056 / 1.504 | 1.856 / 2.816 | 1.824 / 2.880 | 0.547 / 0.579 |
| bulk/1/512/1/0/recent-for-agent | 3.904 / 8.704 | 3.904 / 8.960 | 3.456 / 8.192 | 3.264 / 7.680 | 1.151 / 1.196 |
| bulk/1/None/1/0/recall-by-id | 1.008 / 1.472 | 1.056 / 1.536 | 1.760 / 2.624 | 1.792 / 2.752 | 0.564 / 0.589 |
| bulk/1/None/1/0/recent-for-agent | 3.776 / 7.552 | 3.712 / 8.448 | 3.136 / 6.528 | 2.944 / 6.400 | 1.180 / 1.217 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/1/512/1/0/recall-by-id | before-kiban | 1.216 [1.008, 1.280] | 1.632 [1.376, 2.016] |
| bulk/1/512/1/0/recall-by-id | before-lmdb | 1.856 [1.760, 3.072] | 2.816 [2.688, 4.480] |
| bulk/1/512/1/0/recall-by-id | rebased-kiban | 1.056 [1.024, 1.248] | 1.504 [1.408, 1.824] |
| bulk/1/512/1/0/recall-by-id | rebased-lmdb | 1.824 [1.792, 2.688] | 2.880 [2.560, 4.608] |
| bulk/1/512/1/0/recent-for-agent | before-kiban | 3.904 [3.776, 4.736] | 8.704 [7.680, 11.520] |
| bulk/1/512/1/0/recent-for-agent | before-lmdb | 3.456 [3.072, 4.736] | 8.192 [6.784, 10.496] |
| bulk/1/512/1/0/recent-for-agent | rebased-kiban | 3.904 [3.776, 4.480] | 8.960 [8.448, 11.776] |
| bulk/1/512/1/0/recent-for-agent | rebased-lmdb | 3.264 [2.880, 3.904] | 7.680 [5.376, 9.728] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.008 [0.992, 1.088] | 1.472 [1.312, 1.664] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.760 [1.760, 2.016] | 2.624 [2.496, 3.264] |
| bulk/1/None/1/0/recall-by-id | rebased-kiban | 1.056 [1.056, 1.376] | 1.536 [1.472, 2.176] |
| bulk/1/None/1/0/recall-by-id | rebased-lmdb | 1.792 [1.728, 1.888] | 2.752 [2.560, 2.816] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 3.776 [3.456, 3.904] | 7.552 [6.528, 9.728] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 3.136 [3.072, 3.328] | 6.528 [6.400, 6.656] |
| bulk/1/None/1/0/recent-for-agent | rebased-kiban | 3.712 [3.584, 4.480] | 8.448 [7.808, 10.496] |
| bulk/1/None/1/0/recent-for-agent | rebased-lmdb | 2.944 [2.880, 3.456] | 6.400 [6.016, 7.040] |
