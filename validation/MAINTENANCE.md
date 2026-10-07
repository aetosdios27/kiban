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
