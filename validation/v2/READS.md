# Paired point-read and latest-K results

Microseconds; median [min,max] of five alternating frozen-before/frozen-after trials. Each candidate is followed by its own LMDB control. CPU 8 only; 100,000 verified timed calls per stage. Every command verifies the 1M-record database before and after timing. W2 includes key formatting, copying Kiban values and JSON decoding; W3 returns and decodes ten newest records for an agent. LMDB retains the existing harness semantics.

| Layout | Cache | Workload | Before p50 / p99 | After p50 / p99 | LMDB after p50 / p99 | Paired K/LMDB p50 before → after |
|---|---|---|---:|---:|---:|---:|
| bulk | 512 | recall-by-id | 1.888 / 2.880 | 1.152 / 1.536 | 1.568 / 2.240 | 1.220 → 0.740 |
| bulk | 512 | recent-for-agent | 3.072 / 5.888 | 3.136 / 6.016 | 2.624 / 4.608 | 1.237 → 1.195 |
| bulk | 32 | recall-by-id | 3.840 / 8.960 | 4.096 / 9.216 | 1.568 / 2.240 | 2.500 → 2.571 |
| bulk | 32 | recent-for-agent | 3.008 / 6.400 | 3.264 / 6.656 | 2.752 / 5.248 | 1.231 → 1.186 |
| bulk | default | recall-by-id | 1.952 / 2.944 | 1.248 / 1.600 | 1.504 / 2.112 | 1.255 → 0.830 |
| bulk | default | recent-for-agent | 3.008 / 6.400 | 3.136 / 6.400 | 2.624 / 4.992 | 1.256 → 1.200 |
| normal | 512 | recall-by-id | 2.496 / 5.248 | 1.600 / 2.368 | 1.792 / 2.944 | 1.362 → 0.833 |
| normal | 512 | recent-for-agent | 4.224 / 10.752 | 4.032 / 10.496 | 3.200 / 7.424 | 1.514 → 1.360 |
| normal | 32 | recall-by-id | 5.376 / 14.080 | 4.992 / 12.544 | 1.920 / 3.520 | 2.820 → 2.621 |
| normal | 32 | recent-for-agent | 4.480 / 12.800 | 4.480 / 11.264 | 3.264 / 7.808 | 1.545 → 1.373 |
| normal | default | recall-by-id | 2.048 / 3.712 | 1.376 / 1.952 | 1.600 / 2.560 | 1.306 → 0.860 |
| normal | default | recent-for-agent | 3.584 / 6.912 | 3.584 / 6.656 | 2.688 / 4.736 | 1.474 → 1.333 |

Default uses KibanOptions::default().block_cache_bytes in both builds (cache_mib=null), 428.229 MiB on this host (the exact byte budget is recorded by the range probe). Default trials are separate repetitions, not reused 512-MiB observations.

## Full distributions

| Layout/cache/workload | Candidate | p50 µs | p99 µs | Paired p50 ratio |
|---|---|---:|---:|---:|
| bulk/512/recall-by-id | before | 1.888 [1.792, 1.952] | 2.880 [2.752, 3.008] | 1.220 [1.167, 1.255] |
| bulk/512/recall-by-id | after | 1.152 [1.120, 1.184] | 1.536 [1.472, 1.600] | 0.740 [0.706, 0.766] |
| bulk/512/recent-for-agent | before | 3.072 [3.008, 3.264] | 5.888 [5.504, 8.448] | 1.237 [1.231, 1.308] |
| bulk/512/recent-for-agent | after | 3.136 [3.072, 3.200] | 6.016 [5.376, 6.656] | 1.195 [1.140, 1.220] |
| bulk/32/recall-by-id | before | 3.840 [3.840, 3.968] | 8.960 [8.448, 9.472] | 2.500 [2.449, 2.583] |
| bulk/32/recall-by-id | after | 4.096 [4.032, 4.096] | 9.216 [8.448, 9.984] | 2.571 [2.327, 2.667] |
| bulk/32/recent-for-agent | before | 3.008 [2.944, 3.072] | 6.400 [6.400, 7.040] | 1.231 [1.205, 1.263] |
| bulk/32/recent-for-agent | after | 3.264 [3.200, 3.264] | 6.656 [6.400, 8.960] | 1.186 [1.020, 1.214] |
| bulk/default/recall-by-id | before | 1.952 [1.856, 1.984] | 2.944 [2.752, 3.136] | 1.255 [1.184, 1.319] |
| bulk/default/recall-by-id | after | 1.248 [1.216, 1.376] | 1.600 [1.568, 2.016] | 0.830 [0.796, 0.860] |
| bulk/default/recent-for-agent | before | 3.008 [2.944, 3.136] | 6.400 [5.888, 6.528] | 1.256 [1.179, 1.270] |
| bulk/default/recent-for-agent | after | 3.136 [3.072, 3.648] | 6.400 [6.144, 11.776] | 1.200 [1.171, 1.425] |
| normal/512/recall-by-id | before | 2.496 [1.984, 2.688] | 5.248 [3.520, 11.264] | 1.362 [1.319, 1.667] |
| normal/512/recall-by-id | after | 1.600 [1.440, 1.664] | 2.368 [1.984, 2.560] | 0.833 [0.776, 1.083] |
| normal/512/recent-for-agent | before | 4.224 [3.456, 4.480] | 10.752 [7.552, 15.616] | 1.514 [1.421, 1.795] |
| normal/512/recent-for-agent | after | 4.032 [3.776, 4.736] | 10.496 [7.424, 13.312] | 1.360 [1.180, 1.575] |
| normal/32/recall-by-id | before | 5.376 [5.248, 5.760] | 14.080 [12.800, 15.616] | 2.820 [2.754, 3.103] |
| normal/32/recall-by-id | after | 4.992 [4.864, 5.248] | 12.544 [11.520, 14.080] | 2.621 [2.557, 2.733] |
| normal/32/recent-for-agent | before | 4.480 [4.224, 4.736] | 12.800 [11.776, 14.080] | 1.545 [1.404, 1.644] |
| normal/32/recent-for-agent | after | 4.480 [4.032, 4.608] | 11.264 [9.472, 12.288] | 1.373 [1.207, 1.458] |
| normal/default/recall-by-id | before | 2.048 [2.016, 2.240] | 3.712 [3.648, 4.096] | 1.306 [1.235, 1.375] |
| normal/default/recall-by-id | after | 1.376 [1.344, 1.408] | 1.952 [1.824, 2.240] | 0.860 [0.840, 0.880] |
| normal/default/recent-for-agent | before | 3.584 [3.456, 3.904] | 6.912 [6.528, 9.216] | 1.474 [1.375, 1.525] |
| normal/default/recent-for-agent | after | 3.584 [3.456, 3.648] | 6.656 [6.144, 6.912] | 1.333 [1.317, 1.390] |
