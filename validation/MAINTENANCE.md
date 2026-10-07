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
