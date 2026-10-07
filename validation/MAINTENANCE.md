# Maintenance pressure validation

This pass measures why foreground writes wait, then repeats the same workloads
after a targeted maintenance change. It preserves one worker, one immutable
memtable, WAL durability, MANIFEST publication, compaction drop rules and format.

The baseline adds phase/cause counters only. Ordinary unstalled writes do not
acquire extra clocks or metrics locks. Clocks occur at freeze, maintenance job
boundaries and waits that already happened. Counts classify each blocked call's
first reason; durations classify each individual wait. The three duration classes
sum to the existing aggregate wait time. Concurrent waits overlap and are not
wall time. Successful job timers separate queue, BUILD, COMMIT gate acquisition
and COMMIT execution. Flush queue maximum is since engine open (not subtractable).

`maintenance-plan.json` copies seven affected configurations from the comparison
plans without changing workload parameters: 10M random writes, 10M hot updates,
1M warm reads, 1M burst/recovery and 1M mixed at 4/16/64 threads. Each has three
repetitions with fixed workload/order seed 20261008. All named engine options
remain explicit. 64 threads oversubscribes this host's 22 logical CPUs.

This uses the existing `scale` executable, including its original publication
batch of 256 and stage timing. Both before/after phases use the same harness;
the old comparison harness used 4096, so these fresh measurements must not be
silently pooled with historical RocksDB/Kiban comparison rates. The new verifier
runs outside measurement, reopens each database, recomputes the exact expected
key set, scans all keys/values/order and cross-checks selected point reads.

Build and retain immutable binaries at each committed engine revision:

```sh
python3 validation/maintenance.py --prepare-binaries \
  --binary-dir target/maintenance-binaries/before
python3 validation/maintenance.py \
  --binary-dir target/maintenance-binaries/before \
  --output-root validation/maintenance-results/before \
  --data-root target/maintenance-data/before

# Repeat after committing the candidate, with new binary/output/data directories.
python3 validation/maintenance.py --prepare-binaries \
  --binary-dir target/maintenance-binaries/after
python3 validation/maintenance.py \
  --binary-dir target/maintenance-binaries/after \
  --output-root validation/maintenance-results/after \
  --data-root target/maintenance-data/after
```

The runner records source snapshots, exact commands, revisions, binary/source
hashes, compiler/OS/CPU/RAM/filesystem and per-trial status. It reuses the existing
comparison supervisor for timeout and suspend/wall-clock-gap detection. Every
trial gets fresh directories; errors stop the campaign without retry or smaller
parameters. Generated evidence is excluded from crate packaging.

## Diagnosed delay and candidate

The instrumented baseline is commit `06d87f8`. All 21 runs (75M foreground
operations) completed and passed full reopened-state verification. In the first
10M random-write run, all 787 blocked calls were attributed to immutable capacity:
359 cumulative seconds of caller waiting, maximum flush queue delay 1.72 s,
111 s of compaction BUILD, and only 0.7 ms of compaction COMMIT gate waiting.
This confirms flushes queued behind long compaction BUILD jobs in that workload.
It does not prove the cause of the separate high-concurrency mixed plateau.

The candidate services at most one pending flush after processing
`min(target_file_size, 4 MiB)` logical compaction input bytes. Dropped versions
and tombstones count toward progress, so merges producing little output still
reach checkpoints. It resumes the same merge with its captured input files and
unchanged drop/split rules. Trivial moves retain their metadata-only path.
Cooperative flushes do not publish beyond L0's existing hard ceiling. No extra
worker, immutable slot, WAL generation rule or format is introduced.

Flush and compaction COMMIT remain serialized by the existing engine gate and
apply deltas against the current version. A nested flush uses the normal SST
fsync/MANIFEST/WAL-retirement ordering. Compaction phase timers exclude nested
flush work to avoid double counting. The checkpoint is a byte budget, not a hard
wall-time guarantee; slow I/O can still pause writes.

The new progress regression proves a newer SST installs before the original
compaction completes, preserves old/new snapshots and tombstones, permits further
writes, and verifies reopened point/scan agreement with a one-entry descriptor
cache. It fails with the old non-cooperative worker restored. A separate test
checks durable overwrites/deletes across 24 injected-fault positions during the
cooperative flush path. Existing correctness tests remain unchanged except for
additional cause-counter assertions.

## Correctness checks

The candidate engine is commit `fd4c4e6`. Formatting, both all-target Clippy
configurations (ordinary and Loom), default tests, example tests, all three Loom
models, both ignored release stress tests, package verification and package listing
passed. The default suite passed 262 library and three integration tests; the two
ignored stress tests were explicitly run in release mode. Five example tests
passed. Exact commands and output are in `maintenance-results/checks/after/`.
The expected old-worker regression failure is retained separately.

The existing torture harness also passed probes, 1,000 simulated recovery cycles
(414,831 completed mutations, 341 triggered faults), and 1,000 process-kill/reopen
cycles (532,576 acknowledged mutations). These use seed 20261008, four threads,
snapshots, and sync every 17 operations. Raw configuration, cycle and status CSVs
are retained in `maintenance-results/torture-*`; complete commands are in
`maintenance-results/checks/after/checks.json`. Process validation checks durable
acknowledgements plus the permitted unacknowledged tail, not exact equality with
the acknowledgement ledger. SIGKILL testing is not machine power-loss testing.

Aggregate both complete benchmark campaigns with:

```sh
python3 validation/maintenance_analyze.py \
  validation/maintenance-results/before validation/maintenance-results/after \
  --output validation/maintenance-results/summary
```

The analyzer rejects incomplete campaigns, mismatched plans, wrong operation
counts and inconsistent pressure-counter partitions. It includes every verified
trial and reports medians and ranges; per-trial percentiles are not pooled.

## Measured results

All 42 trials completed, settled with zero sampled final debt, and passed full
reopened-state verification: 150M foreground operations. Three repetitions per
case ran on the same 22-logical-CPU Core Ultra 9 185H host, about 30.8 GiB RAM,
Fedora 44, rustc 1.98.1, release profile, encrypted Btrfs/NVMe. Exact host and
configuration receipts are in each campaign’s metadata. These are buffered writes
with the existing final sync, not per-write durable acknowledgements.

| Workload | Before ops/s | After ops/s | Change |
| --- | ---: | ---: | ---: |
| write-10m | 90,118 | 124,804 | +38.5% |
| updates-10m | 362,134 | 336,039 | -7.2% |
| read-1m | 3,420,539 | 3,423,715 | +0.1% |
| burst-recovery-1m | 441,365 | 466,304 | +5.7% |
| mixed-4t | 528,178 | 499,139 | -5.5% |
| mixed-16t | 426,246 | 349,799 | -17.9% |
| mixed-64t | 355,019 | 365,184 | +2.9% |

Values are medians of complete trials. Full ranges, latency percentiles, phase
times, RSS, descriptors and SST write amplification are in
[`maintenance-results/summary/summary.json`](maintenance-results/summary/summary.json).
The throughput ranges overlap for the long-write trials: 77.6–115.6k/s before
and 82.3–130.9k/s after. Three serial repetitions on a shared desktop do not
establish a universal speedup or statistical significance.

For 10M random writes, median maximum flush queue delay fell from 1,531.6 ms
to 75.6 ms (95.1%). Cumulative writer waits fell from 294.3 s to 168.1 s (42.9%).
The remaining waits are almost entirely L0 pressure: median 167.6 s, versus
0.4 s immutable-only waits. The sampled L0 count never exceeded eight.
Median per-trial maximum PUT fell from 1,511 ms to 1,292 ms, but the worst
candidate trial reached 1,802 ms versus 1,729 ms before. PUT p99 stayed close
(57.3 → 56.3 µs), while p99.9 increased (188.4 → 208.9 µs). Long stalls persist.

SST-only write amplification fell from median 9.59x to 7.08x; the after range
was 6.80–10.22x. This metric excludes WAL, MANIFEST and filesystem/device writes,
so it must not be compared directly with historical process-write amplification.
Median sampled peak RSS increased from 249.0 to 257.1 MiB, and descriptors from
162 to 169. Neither resource issue is solved by this change.

Burst/recovery median maximum PUT fell from 80.8 to 14.5 ms. Mixed at 16 threads
regressed from 426k to 350k ops/s, with PUT p99 increasing from 294.9 to 344.1 µs
and p99.9 from 6.95 to 8.65 ms. Hot-update throughput fell 7.2%; mixed at four
threads fell 5.5%. Warm reads were essentially unchanged. This is a targeted
maintenance improvement with measured tradeoffs, not an across-the-board win.

## Observations, hypotheses and remaining work

The baseline phase counters and deterministic regression confirm that a pending
flush waited behind compaction BUILD. Cooperative service removes most of that
queue delay. Afterward, the sustained-write capacity constraint is L0 draining;
the engine still blocks writes at the existing ceiling. COMMIT gate acquisition
was negligible in the diagnosed baseline write run. These measurements do not
establish which part of compaction BUILD is expensive (CPU, reads or writes).

The mixed/update regressions are measured; their cause is unresolved. Changed
flush timing can change file overlap, read work and foreground gate scheduling,
and checkpoint polling adds some worker work. Those are hypotheses requiring
profiles and an independently repeated campaign, not confirmed bottlenecks.
No additional optimization was made to hide the regressions.

The candidate remains on `perf/maintenance-progress`; the release tag and main
are unchanged. It should be evaluated against the mixed-workload regression
before adoption. This campaign does not replace the 100M+, larger-than-memory
or overnight validation. One-second resource samples may miss brief peaks,
checkpoint byte budgets do not bound slow I/O latency, and SIGKILL/fault tests
do not model every power-loss or device failure. No storage format, durability
rule, snapshot retention rule or compaction version-drop rule changed.
