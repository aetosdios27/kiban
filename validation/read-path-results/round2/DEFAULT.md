Five alternating before/after trials, each followed by its LMDB control. Latencies are median trial p50 / p99 in microseconds. All timed results are checked outside their timer, with full dataset verification before and after the reads. Write trials use fresh copies and verify the final state.

| Scenario | Workload | Before Kiban p50 / p99 | After Kiban p50 / p99 | After LMDB p50 / p99 | Paired after K/LMDB p50 [range] |
| --- | --- | ---: | ---: | ---: | ---: |
| bulk-32 | recall-by-id | 5.120 / 8.448 | 5.248 / 8.960 | 1.760 / 2.560 | 2.877 [2.764, 3.133] |
| bulk-32 | recent-for-agent | 4.352 / 8.960 | 4.224 / 8.704 | 3.264 / 6.528 | 1.320 [1.235, 2.000] |
| bulk-512 | recall-by-id | 2.368 / 3.584 | 2.176 / 3.520 | 1.920 / 3.136 | 1.193 [1.079, 1.367] |
| bulk-512 | recent-for-agent | 4.480 / 9.728 | 4.352 / 8.704 | 3.392 / 6.528 | 1.320 [1.189, 1.833] |
| bulk-default | recall-by-id | 4.992 / 8.192 | 2.176 / 3.392 | 1.856 / 3.008 | 1.172 [0.921, 1.236] |
| bulk-default | recent-for-agent | 4.352 / 8.960 | 4.224 / 8.960 | 3.328 / 7.040 | 1.269 [0.821, 1.925] |
| normal-32 | recall-by-id | 5.632 / 10.496 | 5.504 / 9.984 | 1.888 / 2.944 | 2.915 [2.343, 3.286] |
| normal-32 | recent-for-agent | 5.120 / 11.776 | 4.992 / 10.752 | 3.328 / 7.168 | 1.520 [1.423, 2.296] |
| normal-512 | recall-by-id | 2.688 / 4.352 | 2.496 / 4.032 | 1.760 / 2.688 | 1.418 [1.254, 1.600] |
| normal-512 | recent-for-agent | 5.248 / 11.264 | 4.864 / 10.496 | 3.264 / 6.784 | 1.519 [0.950, 2.353] |

Excluded normal-default: see `normal-default/EXCLUDED.md`.

| writes-durable | mixed-90-10 | 2.432 / 606.208 | 2.432 / 655.360 | 1.664 / 1441.792 | 1.462 [1.440, 1.615] |
| writes-durable | recall-by-id | 1.184 / 2.816 | 1.120 / 2.816 | 0.976 / 1.824 | 1.115 [1.059, 1.207] |
| writes-durable | recent-for-agent | 3.392 / 7.168 | 3.200 / 7.168 | 2.624 / 4.032 | 1.200 [1.190, 1.250] |
| writes-durable | remember | 557.056 / 1048.576 | 573.440 / 966.656 | 1245.184 / 2490.368 | 0.458 [0.419, 0.474] |
| writes-durable | update | 557.056 / 1146.880 | 589.824 / 1277.952 | 1245.184 / 5242.880 | 0.474 [0.446, 0.486] |
| writes-fast | mixed-90-10 | 1.120 / 2.048 | 1.152 / 2.112 | 0.992 / 8.960 | 1.161 [1.115, 1.213] |
| writes-fast | recall-by-id | 1.056 / 1.856 | 1.088 / 1.952 | 0.976 / 1.376 | 1.115 [0.974, 1.200] |
| writes-fast | recent-for-agent | 3.392 / 6.656 | 3.456 / 7.040 | 2.688 / 3.584 | 1.286 [1.220, 1.571] |
| writes-fast | remember | 0.736 / 6.656 | 0.736 / 7.040 | 7.424 / 15.104 | 0.099 [0.097, 0.100] |
| writes-fast | update | 0.864 / 6.400 | 0.848 / 6.784 | 7.936 / 14.336 | 0.107 [0.100, 0.109] |
