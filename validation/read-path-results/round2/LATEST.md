Five alternating before/after trials, each followed by its LMDB control. Latencies are median trial p50 / p99 in microseconds. All timed results are checked outside their timer, with full dataset verification before and after the reads. Write trials use fresh copies and verify the final state.

| Scenario | Workload | Before Kiban p50 / p99 | After Kiban p50 / p99 | After LMDB p50 / p99 | Paired after K/LMDB p50 [range] |
| --- | --- | ---: | ---: | ---: | ---: |
| bulk-32 | recall-by-id | 5.120 / 8.064 | 4.992 / 8.448 | 1.856 / 3.008 | 2.714 [2.606, 2.737] |
| bulk-32 | recent-for-agent | 4.032 / 8.064 | 4.096 / 8.960 | 3.584 / 7.168 | 1.241 [1.107, 1.280] |
| bulk-512 | recall-by-id | 2.240 / 3.328 | 2.368 / 3.648 | 1.888 / 2.880 | 1.254 [1.233, 1.298] |
| bulk-512 | recent-for-agent | 3.968 / 7.936 | 4.224 / 8.704 | 3.520 / 7.168 | 1.222 [1.200, 1.273] |
| normal-32 | recall-by-id | 5.376 / 9.728 | 5.376 / 9.728 | 1.888 / 3.008 | 2.897 [2.800, 2.966] |
| normal-32 | recent-for-agent | 4.736 / 9.728 | 4.992 / 10.240 | 3.520 / 7.168 | 1.418 [1.393, 1.444] |
| normal-512 | recall-by-id | 2.560 / 4.352 | 2.496 / 4.352 | 1.856 / 2.816 | 1.345 [1.279, 1.564] |
| normal-512 | recent-for-agent | 4.864 / 11.264 | 4.864 / 11.264 | 3.456 / 7.552 | 1.407 [1.382, 1.519] |
| writes-durable | mixed-90-10 | 2.816 / 901.120 | 3.008 / 1081.344 | 2.048 / 2555.904 | 1.438 [1.294, 1.619] |
| writes-durable | recall-by-id | 1.408 / 3.008 | 1.344 / 3.264 | 1.184 / 2.240 | 1.135 [1.026, 1.189] |
| writes-durable | recent-for-agent | 4.352 / 11.776 | 4.608 / 11.264 | 3.456 / 7.936 | 1.333 [1.214, 1.407] |
| writes-durable | remember | 655.360 / 2752.512 | 622.592 / 1900.544 | 1474.560 / 6815.744 | 0.426 [0.408, 0.451] |
| writes-durable | update | 638.976 / 3211.264 | 638.976 / 2097.152 | 1474.560 / 7733.248 | 0.429 [0.342, 0.443] |
| writes-fast | mixed-90-10 | 1.472 / 2.944 | 1.504 / 3.392 | 1.216 / 15.616 | 1.237 [0.922, 1.658] |
| writes-fast | recall-by-id | 1.344 / 2.560 | 1.440 / 2.944 | 1.152 / 1.824 | 1.111 [0.900, 1.286] |
| writes-fast | recent-for-agent | 4.480 / 13.568 | 4.608 / 21.504 | 3.648 / 7.296 | 1.263 [0.818, 2.000] |
| writes-fast | remember | 0.960 / 10.496 | 0.976 / 12.544 | 9.984 / 41.984 | 0.098 [0.067, 0.103] |
| writes-fast | update | 1.152 / 11.264 | 1.152 / 13.568 | 10.240 / 27.648 | 0.115 [0.079, 0.119] |
