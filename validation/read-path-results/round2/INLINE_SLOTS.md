Five alternating before/after trials, each followed by its LMDB control. Latencies are median trial p50 / p99 in microseconds. All timed results are checked outside their timer, with full dataset verification before and after the reads. Write trials use fresh copies and verify the final state.

| Scenario | Workload | Before Kiban p50 / p99 | After Kiban p50 / p99 | After LMDB p50 / p99 | Paired after K/LMDB p50 [range] |
| --- | --- | ---: | ---: | ---: | ---: |
| bulk-32 | recall-by-id | 4.736 / 7.808 | 5.120 / 8.960 | 1.824 / 2.816 | 2.877 [2.600, 3.019] |
| bulk-32 | recent-for-agent | 3.776 / 6.784 | 3.776 / 7.296 | 3.264 / 6.272 | 1.157 [1.098, 1.229] |
| bulk-512 | recall-by-id | 2.112 / 3.200 | 1.984 / 2.944 | 1.664 / 2.432 | 1.220 [1.157, 1.269] |
| bulk-512 | recent-for-agent | 3.776 / 6.912 | 3.648 / 7.296 | 3.008 / 5.632 | 1.213 [1.184, 1.283] |
| normal-32 | recall-by-id | 4.992 / 8.704 | 5.248 / 10.240 | 1.664 / 2.432 | 3.154 [3.019, 3.231] |
| normal-32 | recent-for-agent | 4.480 / 8.704 | 4.480 / 9.728 | 3.008 / 5.760 | 1.489 [1.388, 1.489] |
| normal-512 | recall-by-id | 2.240 / 3.520 | 2.240 / 3.456 | 1.728 / 2.496 | 1.296 [1.212, 1.451] |
| normal-512 | recent-for-agent | 4.352 / 8.704 | 4.352 / 8.704 | 3.008 / 5.504 | 1.447 [1.358, 1.489] |
| writes-durable | mixed-90-10 | 2.880 / 688.128 | 2.880 / 753.664 | 2.016 / 1769.472 | 1.438 [0.976, 1.587] |
| writes-durable | recall-by-id | 1.376 / 3.200 | 1.216 / 2.688 | 1.248 / 2.304 | 1.027 [0.760, 1.128] |
| writes-durable | recent-for-agent | 3.968 / 10.240 | 4.096 / 7.424 | 3.328 / 6.656 | 1.196 [0.780, 1.308] |
| writes-durable | remember | 573.440 / 1638.400 | 622.592 / 5242.880 | 1409.024 / 3670.016 | 0.432 [0.415, 0.487] |
| writes-durable | update | 589.824 / 1802.240 | 606.208 / 1605.632 | 1376.256 / 3276.800 | 0.449 [0.398, 0.464] |
| writes-fast | mixed-90-10 | 1.312 / 2.688 | 1.344 / 2.816 | 1.088 / 9.984 | 1.222 [1.135, 1.412] |
| writes-fast | recall-by-id | 1.248 / 2.176 | 1.248 / 2.304 | 1.088 / 1.664 | 1.114 [0.913, 1.206] |
| writes-fast | recent-for-agent | 3.904 / 8.448 | 4.224 / 9.728 | 3.328 / 6.656 | 1.240 [1.220, 1.269] |
| writes-fast | remember | 0.848 / 8.064 | 0.928 / 10.752 | 8.960 / 18.944 | 0.104 [0.097, 0.125] |
| writes-fast | update | 1.088 / 7.936 | 1.056 / 8.704 | 9.216 / 16.384 | 0.114 [0.107, 0.118] |
