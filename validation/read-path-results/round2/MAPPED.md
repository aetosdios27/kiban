Five alternating before/after trials, each followed by its LMDB control. Latencies are median trial p50 / p99 in microseconds. All timed results are checked outside their timer, with full dataset verification before and after the reads. Write trials use fresh copies and verify the final state.

| Scenario | Workload | Before Kiban p50 / p99 | After Kiban p50 / p99 | After LMDB p50 / p99 | Paired after K/LMDB p50 [range] |
| --- | --- | ---: | ---: | ---: | ---: |
| bulk-32 | recall-by-id | 3.840 / 6.528 | 2.432 / 3.584 | 1.856 / 2.816 | 1.322 [1.288, 1.404] |
| bulk-32 | recent-for-agent | 4.224 / 8.448 | 4.224 / 8.704 | 3.392 / 6.912 | 1.231 [1.212, 1.245] |
| bulk-512 | recall-by-id | 2.496 / 3.840 | 2.560 / 3.904 | 1.728 / 2.624 | 1.367 [1.303, 1.481] |
| bulk-512 | recent-for-agent | 4.224 / 8.960 | 4.224 / 8.704 | 3.328 / 7.040 | 1.245 [1.217, 1.296] |
| normal-32 | recall-by-id | 4.096 / 7.296 | 2.688 / 4.352 | 1.824 / 2.752 | 1.455 [1.311, 1.509] |
| normal-32 | recent-for-agent | 5.248 / 10.752 | 4.864 / 9.984 | 3.392 / 6.912 | 1.451 [1.370, 1.472] |
| normal-512 | recall-by-id | 2.368 / 3.904 | 2.688 / 4.352 | 1.888 / 2.816 | 1.418 [1.333, 1.673] |
| normal-512 | recent-for-agent | 4.864 / 10.240 | 4.864 / 9.984 | 3.456 / 7.424 | 1.407 [1.370, 1.500] |
| writes-durable | mixed-90-10 | 2.752 / 688.128 | 2.752 / 671.744 | 1.920 / 1605.632 | 1.429 [1.048, 1.627] |
| writes-durable | recall-by-id | 1.376 / 3.008 | 1.248 / 2.816 | 1.120 / 2.016 | 1.118 [1.054, 1.571] |
| writes-durable | recent-for-agent | 4.352 / 8.704 | 4.032 / 9.984 | 3.392 / 5.632 | 1.207 [1.189, 1.250] |
| writes-durable | remember | 589.824 / 2064.384 | 606.208 / 1867.776 | 1310.720 / 5242.880 | 0.450 [0.440, 0.463] |
| writes-durable | update | 589.824 / 2097.152 | 589.824 / 2293.760 | 1376.256 / 7864.320 | 0.430 [0.417, 0.450] |
| writes-fast | mixed-90-10 | 1.760 / 3.136 | 1.568 / 3.264 | 1.216 / 15.616 | 1.132 [0.961, 1.361] |
| writes-fast | recall-by-id | 1.248 / 2.496 | 1.248 / 2.304 | 1.120 / 1.632 | 1.139 [1.086, 1.278] |
| writes-fast | recent-for-agent | 4.736 / 10.240 | 4.352 / 8.960 | 3.520 / 6.656 | 1.236 [1.185, 2.036] |
| writes-fast | remember | 0.944 / 8.064 | 0.896 / 8.064 | 9.984 / 21.504 | 0.088 [0.085, 0.092] |
| writes-fast | update | 1.376 / 11.520 | 1.056 / 7.808 | 10.240 / 19.456 | 0.103 [0.098, 0.174] |
