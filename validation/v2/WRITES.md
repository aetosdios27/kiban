# Paired 10k write results

Five alternating trials per mode, 1,000 operations per workload. Main 8bbb4934 versus optional locator-index writes. Per-call samples exclude final flush; end_sync_flush_verify_seconds is reported separately and includes verification. Durable mode syncs each mutating call.

| Mode | Workload | Before K p50 / p99 µs | After K p50 / p99 µs | LMDB after p50 / p99 µs | Paired K/LMDB p50 before → after | K call ops/s before → after |
|---|---|---:|---:|---:|---:|---:|
| fast | mixed-90-10 | 1.056 / 2.176 | 0.864 / 1.824 | 0.928 / 8.064 | 1.294 → 0.983 | 906235 → 1120537 |
| fast | recall-by-id | 0.976 / 1.888 | 0.704 / 1.024 | 0.880 / 1.248 | 1.151 → 0.815 | 966236 → 1425383 |
| fast | recent-for-agent | 3.072 / 6.912 | 3.264 / 5.888 | 2.688 / 4.224 | 1.237 → 1.220 | 291625 → 291997 |
| fast | remember | 0.672 / 6.016 | 0.672 / 6.144 | 7.040 / 14.592 | 0.094 → 0.095 | 1020721 → 1036310 |
| fast | update | 0.768 / 5.760 | 0.768 / 6.272 | 7.296 / 13.824 | 0.105 → 0.107 | 953264 → 920785 |
| durable | mixed-90-10 | 2.432 / 835.584 | 1.760 / 688.128 | 1.728 / 1605.632 | 1.393 → 1.000 | 15241 → 15627 |
| durable | recall-by-id | 1.056 / 2.688 | 0.848 / 1.760 | 0.912 / 1.632 | 1.143 → 0.932 | 873269 → 1137655 |
| durable | recent-for-agent | 3.136 / 6.912 | 3.264 / 7.424 | 2.688 / 4.480 | 1.256 → 1.214 | 281268 → 285892 |
| durable | remember | 557.056 / 1900.544 | 573.440 / 2424.832 | 1277.952 / 7077.888 | 0.447 → 0.462 | 1601 → 1550 |
| durable | update | 573.440 / 2293.760 | 573.440 / 2818.048 | 1277.952 / 4980.736 | 0.439 → 0.446 | 1548 → 1515 |
