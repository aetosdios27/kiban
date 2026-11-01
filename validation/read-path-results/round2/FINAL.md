# Round-two retained result

The target of beating LMDB is **not met**. Only the RAM/cgroup cache-budget
policy is retained. Point handles, mapped points and the latest-K merge
candidate were rejected; see [REJECTED.md](REJECTED.md) for source patches,
paired results and cold/concurrent tails. Public APIs, SST format and
`mmap_max_level` semantics are unchanged.

The table below selects the `before` binary from [latest](latest), whose
engine is exactly the cache-budget commit. LMDB controls are from those same
trial halves. Reproduce the audit with:

```sh
python3 tools/agentbench/readpath/round2_report.py validation/read-path-results/round2/latest --retained before
```

The cache-budget commit's separate [before/after matrix](DEFAULT.md) covers
all four required explicit-cache scenarios plus 10k fast/durable write checks.
Its fast insert/update p50 were 0.736 → 0.736 and 0.864 → 0.848 µs;
durable 557.056 → 573.440 and 557.056 → 589.824 µs. Ranges overlap,
with noisy fsync tails; no clear retained write regression is established.
The final table below is a later contemporary comparison with LMDB, not a
cross-run baseline comparison. Each write stage has 1,000 calls on a fresh
10k-record dataset.

Five alternating before/after trials, each followed by its LMDB control. Latencies are median trial p50 / p99 in microseconds. All timed results are checked outside their timer, with full dataset verification before and after the reads. Write trials use fresh copies and verify the final state.

Retained engine: `before`; controls are from the same trial half.

| Scenario | Workload | Kiban p50 / p99 | LMDB p50 / p99 | Paired K/LMDB p50 [range] |
| --- | --- | ---: | ---: | ---: |
| bulk-32 | recall-by-id | 5.120 / 8.064 | 1.856 / 2.816 | 2.741 [2.710, 2.786] |
| bulk-32 | recent-for-agent | 4.032 / 8.064 | 3.328 / 6.656 | 1.236 [1.212, 1.286] |
| bulk-512 | recall-by-id | 2.240 / 3.328 | 1.856 / 2.880 | 1.241 [1.207, 1.263] |
| bulk-512 | recent-for-agent | 3.968 / 7.936 | 3.264 / 6.784 | 1.196 [1.151, 1.260] |
| normal-32 | recall-by-id | 5.376 / 9.728 | 1.824 / 2.944 | 2.947 [2.847, 3.172] |
| normal-32 | recent-for-agent | 4.736 / 9.728 | 3.200 / 6.528 | 1.480 [1.423, 1.538] |
| normal-512 | recall-by-id | 2.560 / 4.352 | 1.824 / 2.816 | 1.404 [1.258, 1.464] |
| normal-512 | recent-for-agent | 4.864 / 11.264 | 3.200 / 7.424 | 1.520 [1.370, 1.551] |
| writes-durable | mixed-90-10 | 2.816 / 901.120 | 2.112 / 2031.616 | 1.410 [1.324, 1.606] |
| writes-durable | recall-by-id | 1.408 / 3.008 | 1.216 / 2.560 | 1.194 [0.854, 1.378] |
| writes-durable | recent-for-agent | 4.352 / 11.776 | 3.456 / 8.960 | 1.222 [1.088, 1.283] |
| writes-durable | remember | 655.360 / 2752.512 | 1474.560 / 3670.016 | 0.446 [0.411, 0.455] |
| writes-durable | update | 638.976 / 3211.264 | 1507.328 / 3997.696 | 0.424 [0.378, 0.443] |
| writes-fast | mixed-90-10 | 1.472 / 2.944 | 1.248 / 16.384 | 1.222 [1.024, 1.538] |
| writes-fast | recall-by-id | 1.344 / 2.560 | 1.120 / 1.952 | 1.200 [0.722, 1.800] |
| writes-fast | recent-for-agent | 4.480 / 13.568 | 3.456 / 8.192 | 1.296 [0.681, 1.852] |
| writes-fast | remember | 0.960 / 10.496 | 10.240 / 28.672 | 0.096 [0.064, 0.116] |
| writes-fast | update | 1.152 / 11.264 | 12.800 / 167.936 | 0.113 [0.072, 0.118] |

Final retained-code gates all passed: workspace debug/release suites,
Clippy with warnings denied, formatting, readbench tests/checks, both fixed
crash corpora, loom and site-facts validation. Both release crash/model
suites passed seeds 0–19,999 each: 40,000 seeds and 160,000 simulated
crashes. No storage correctness failure was observed.
[Gate statuses](final-checks/status.json), [source hashes](final-checks/source-hashes.json) and [paired audit](AUDIT.json) preserve the evidence.
The paired audit checks all completed groups, operation/verification counts,
zero errors, CPU 3 affinity and frozen binary hashes. Final engine hashes
match the tested retained source. Nothing was pushed, no PR was opened,
and no subagents were spawned.
