# First scale-validation report

**Historical evidence from before the correctness repairs.** The recorded defects
are repaired in the current code; see [CORRECTNESS.md](CORRECTNESS.md). Reproduce
this report’s failures on revision `6c29f8f`. The frozen-WAL probe is now an
ordinary passing test, so its historical `--ignored` command does not run it on
current code. Performance figures below remain tied to their recorded binaries.

Kiban fails correctness validation at small scale. These failures were exposed
by the new harness without changing engine policy, durability, or assertions.
Performance measurements therefore describe behavior, not a production-readiness
claim. Source changes add maintenance timing, trace sequences, explicit regression probes,
and two repairs to the existing simulated-device integration (bulk reads and
orphan enumeration). Engine scheduling and durability policy remain unchanged.

## Confirmed correctness cliffs

The most serious finding is a failed durability barrier, independently reproduced
with the existing power-loss model:

- **`sync()` after an unsynced memtable freeze does not cover the old WAL.** Put
  `acknowledged=must-survive`, invoke the same freeze operation used by automatic
  `SharedKiban` handoff, successfully sync, then simulate power loss before the
  frozen SST is built. Recovery returns `None`. Freeze drops the old WAL handle;
  sync persists only the new active WAL. The older generation is MANIFEST-live
  but its buffered/device-overlay records are not made durable by that sync.
  The existing flush sweep syncs *before* freeze, missing this ordering.
  Reproduce with:

  ```sh
  cargo test --lib sync_after_freeze_must_durably_cover_the_older_wal -- --ignored --nocapture
  ```

  This command **fails**. The new assertion demanding preservation is retained
  in `src/scale_validation_tests.rs`. The explicit reproducer is ignored in the
  ordinary suite; its separate failing result is recorded in
  `results/checks/freeze-sync-probe.log`. Regular test success does not resolve it.

Two additional failures occur on ordinary real files:

1. **Direct `Kiban` snapshots lose unflushed captured data across a flush.** Put
   `key=before`, sync, capture snapshot, put `key=after`, sync, flush. Snapshot
   `get_at(key)` returns `None`, rather than `before`. The snapshot pins only the
   old Version; it does not capture the memtable, and the newly flushed SST belongs
   to a later Version. This reproduction needs no crash, simulated device, or fault.
2. **SST point lookup can return an older version than a scan.** Put values 1..40
   under one key, sync, flush, reopen. `get(key)` returns 8; scan returns 40.
   `VerifiedBlock::get` chooses the last restart whose key is `<= target`, which
   skips newer versions when equal-key versions cross restart points. The current
   memtable retains these versions and flush emits them, even without a snapshot.
   A tombstone can similarly produce disagreement; the recovery campaign detected
   both missing/older point results relative to scans. This is a lookup defect,
   not evidence by itself that crash recovery lost the newest bytes.

The new `torture --mode probes` fails on both. Campaign outcomes and exact
commands are preserved below. No fix is included: this
phase finds the cliff, and does not optimize or rewrite the engine.

## Host and measurement boundary

Intel Core Ultra 9 185H, 22 exposed logical CPUs (16 cores, heterogeneous), about
31GiB physical RAM; WD SN740 NVMe. Storage measurements use the encrypted Btrfs
workspace filesystem, not `/tmp` (tmpfs). The machine is not otherwise isolated,
CPU placement/frequency is not controlled, and each listed comparison is one trial.
32/64 threads are oversubscribed. Do not treat small differences as repeatable wins.

Raw configurations, CSVs, logs and binary hashes live under `results/`. Early tmpfs
smoke runs and briefly overlapping campaigns are retained but explicitly excluded
in `results/exclusions.json`. `analyze.py` respects that file by default. All
reported comparisons below use serial, uncontaminated runs.

The performance workload uses deterministic fixed values, so it cannot distinguish
stale versions of overwrites. Versioned values/deletes in torture provide that
correctness check. Unless explicitly stated, performance runs use end-only sync;
throughput is not durable-write throughput. Write amplification below is **SST
output / application key+value bytes**, excluding WAL/MANIFEST/device overhead.

## Concurrency measurement

1M operations, 50% requested reads, seed 42, 100k populated keys, 100k-key working
set, 256-byte values, default engine/governor, end-only sync. Exactly 499,755 reads
and 500,245 writes in every run. Use `concurrency/mixed-{1,2,4,8}t` and
`concurrency-clean/mixed-{16,32,64}t` under results.

| Threads | Total ops/s | Read p99 µs | Write p99 µs | Write p99.9 ms |
|---:|---:|---:|---:|---:|
| 1 | 86,635 | 49.2 | 21.5 | 0.058 |
| 2 | 113,292 | 51.2 | 36.9 | 0.123 |
| 4 | 118,067 | 62.5 | 217 | 0.639 |
| 8 | 101,028 | 71.7 | 885 | 3.80 |
| 16 | 91,120 | 75.8 | 2,621 | 17.3 |
| 32 | 84,437 | 81.9 | 6,160 | 28.8 |
| 64 | 83,468 | 79.9 | 15,466 | 75.5 |

**Measured observation:** throughput stops materially scaling beyond 2–4 threads
in this mix; write tails rise much faster than read tails. Sampled L0 max was 4,
final deep-level debt zero, SST write amp about 3.815x across the matrix. The 64-thread
run recorded 505 stalled calls and 100.5 seconds of summed caller backpressure wait
in 12.0 seconds of foreground wall time (concurrent waits overlap).

**Confirmed mechanism:** foreground writers do wait for maintenance progress;
the engine serializes mutations through a shared write gate. **Hypotheses:** gate
contention, freeze/commit durability work, and maintenance capacity contribute to
the scaling ceiling. No lock-wait profiler was collected, so the fraction due to
ordinary lock contention and the CPU-vs-storage bottleneck are not established.

## Sustained 10M-write measurement

10M random writes, 1M keys/working set, 128-byte values, four threads, seed 42,
no preload, default governor, end-only sync. `results/sustained-10m/`.

- 165.07s foreground wall time, **60,580 ops/s**, write p99 **258µs**.
- Final SST write amp **7.189x**; application bytes 1.36GB; completed compaction
  input **9.589GB**, output **8.242GB**; 283 compactions and 401 flushes.
- **767 stalled calls**, **311.3s** summed backpressure wait. Successful flush and
  compaction time covered about **97%** of foreground wall time before final drain.
- Sampled L0 max **4**, deep-debt peak **70.0MB**, final debt **0** after drain.
  Peak RSS **239.6MB**, peak process FDs **132** (includes telemetry/WAL/job FDs,
  not solely the configured 128-entry SST descriptor cache); final DB **187.9MB**.
- First/last thirds of operation-bearing sampling windows: **69.2k / 65.1k ops/s**.
  The first individual window was faster; startup/population effects must not be
  mistaken for a stationary result. No governor trace sequence gaps were observed.
- Linux process-attributed writes **12.01GB**, storage reads **0**: data and inputs
  fit comfortably in OS cache. This is **not** a cold-disk compaction experiment.

**Confirmed:** the maintenance pipeline imposes substantial foreground
backpressure under this write workload. **Not confirmed:** which worker resource
limits progress, or whether the scheduler is overcompacting relative to a matched
alternative; no scheduler A/B or resource profiler was run.

## 100M reads and cache attribution

`results/read-100m/`: 100M reads, one thread, 100k keys, 256-byte values, seed 42,
reusing the populated one-thread concurrency database after an explicit warm pass.
**215.06s**, **464,992 reads/s**, read p99 **3.52µs**. All 100M reads hit Kiban's
block cache; zero misses, zero process storage-read bytes, RSS about 33.5MB.
This proves count/harness capacity and a cache-resident path, not disk scalability.

Matched 1M-read runs on the same 28.25MB database:

| Cache preparation | Reads/s | Read p99 µs | Process storage-read bytes |
|---|---:|---:|---:|
| Warm pass | 466,890 | 3.46 | 0 |
| File-scoped advisory eviction + reopen | 385,599 | 6.02 | 27,676,672 |

The reduced run's first/last sampling-window rates were about 257k/461k reads/s:
it warmed quickly. The measured distinction works, but a 28MB dataset still fits
RAM. Neither this comparison nor the write runs establish larger-than-RAM behavior.
See `results/read-{warm,reduced}/` for latency distributions and cache counters.

## Workload and transition coverage

All nine serial 1M-operation profiles completed with no API errors. Configurations
and outputs are in `results/workloads-clean/commands.json` and per-workload files.
These use 100k keys/working set, 128-byte values, four threads except the initial
one-thread concurrency phase, seed 42, default engine and end-only sync. Sequential
writes explicitly wrap the 100k-key domain. Read/mixed/update/concurrency profiles
preload all keys; burst profiles start empty and report legitimate initial misses.

| Profile | Total ops/s | Read p99 µs | Write p99 µs |
|---|---:|---:|---:|
| Sequential write | 119,358 | — | 201 |
| Random write | 111,531 | — | 213 |
| Random read | 1,622,432 | 4.03 | — |
| Mixed | 198,072 | 53.2 | 197 |
| Update-heavy, full working set | 190,058 | 53.2 | 213 |
| Write burst → read-heavy | 192,795 | 31.7 | 193 |
| Sustained write | 108,720 | — | 205 |
| Steady → burst → recovery | 203,240 | 35.8 | 205 |
| One → four threads | 163,151 | 49.2 | 143 |

An additional hot-update profile (`results/update-hot/`), 1k-key working set,
90% requested writes, 100k preloaded keys, 128-byte values and four threads,
completed 1M operations at 192,450 ops/s, read/write p99 3.33/172µs and SST amp 1.130x.

Transition observations:

- Burst-read write p99 fell from **197µs** in write pressure to **62.5µs** in the
  read-heavy phase; the latter's read p99 was **31.7µs**.
- Burst-recovery write p99 was **217µs / 201µs / 60.4µs** across its three phases.
  The governor remained `ReadProtect` in recovery and final debt reached zero.
- The concurrency jump raised write p99 from **21.5µs** to **205µs**, while
  throughput rose from 142k to 192k ops/s in those two phases.
- No trace gaps were observed. Sampled mode/pressure and actual focus/candidate
  decisions are preserved; exact phase attribution is limited by 1s sampling.

These short transitions did not demonstrate mode oscillation or establish
reaction-time bounds. Over/undercompaction requires matched controls; high write
amp alone does not prove either. Idle mode persistence also does not by itself
mean harmful actuation when there are no jobs.

## Ten-minute sustained-write stability

`results/ten-minutes/sustained-600s/`: same wide-domain configuration as the 10M
write run, duration 600s, no operation cap. **33,324,436 writes** in **600.005s**,
**55,540 ops/s**, aggregate write p99 **254µs**. Final sync/flush/drain took 3.47s.

- First/last approximately one-minute windows: **69.2k / 54.8k ops/s**, a **20.8%**
  decline. Intermediate minute windows varied nonmonotonically, including roughly
  44.5k ops/s. The run degraded; the cause remains unprofiled.
- Cumulative SST write amp increased from about **6.46x** after the first minute
  to **7.66x** after final drain. Application bytes **4.532GB**, SST output **34.713GB**.
- **936 compactions**, **1,335 flushes**, compaction input/output **34.541/29.598GB**.
- **2,664 stalled calls**, **1,216.9s** summed caller backpressure waits. Successful
  compaction/flush elapsed time was **496.0/92.8s** (includes gate/I/O waits).
- Sampled L0 max **4**, debt peak **96.4MB**, final debt **0**. DB apparent size
  fluctuated with job outputs; final DB **171.4MB**.
- Peak RSS **270.5MB**, final **195.1MB**; peak FDs **142**, final **54**.
  No monotonic RSS/FD leak was established over these ten minutes.
- Maximum interval write p99/p99.9 **500µs / 1.38ms**. Aggregate percentiles alone
  would hide interval behavior; the full time series is retained.
- Governor mode switches **18**, no decision-trace gaps. This count alone does
  not establish oscillation or inappropriate scheduling.
- Process storage writes **42.28GB**, storage reads **0**: the population remains
  far smaller than RAM. The run stresses maintenance without cold input I/O.

**Measured:** throughput and write-amp degradation, bounded sampled L0/debt bands,
and substantial maintenance backpressure. **Hypotheses:** evolving overlap/LSM
shape, host throttling, contention or another resource limit explain the drift.
No hour/overnight boundedness claim follows from ten minutes.

## Repeated recovery outcomes

Final campaigns after simulator integration repairs:

| Campaign | Verified recoveries | Outcome |
|---|---:|---|
| WAL-only simulated power loss, snapshots explicitly off | 1,000 | Passed exact durable-state equality; 10,508 completed mutations, 63 injected faults fired |
| Simulated flush/compaction + snapshots off | 115 | Failed at cycle 115: point lookup returned an older value than scan |
| Standard simulated campaign + snapshots on | 0 | Failed in first recovery on point/scan disagreement, with no injected fault |
| Concurrent shared-engine process crashes + snapshots on | 1 | Next child reported shared-snapshot get/scan disagreement; campaign failed |
| Two real-file minimal probes | — | Both failed |
| Explicit freeze → sync → power-loss probe | — | Failed: acknowledged write missing |

See `results/{wal-recovery-complete,compaction-recovery-complete}/` and
`results/torture-complete/`; each has config, status, cycles/probes and error logs.
The first 115 verified cycles in the compaction campaign completed 47,022
mutations and triggered 36 syscall faults. Attempt bands are used only after
ambiguous failed flushes, exact equality otherwise. Known unacknowledged process
tails are bounded by the parent's recorded receipt protocol.

The simulator's bulk reader originally bypassed its device namespace, and orphan
sweeps originally enumerated only real files. Those framework gaps were repaired
using the existing `sys::File` backend and simulated namespace. The real bulk
reader retains its owned descriptor, positional read-ahead, and cache bypass;
normal orphan enumeration is unchanged. New tests verify byte equivalence, cache
isolation, orphan removal, and preservation of live/nested namespaces.

Torture now records **every operation error** in `operation-errors.csv` and rejects
errors without a fired injected fault. Earlier exploratory simulated runs could
treat an unexpected compaction error as an ambiguous crash. They are retained for
audit, superseded by the final campaigns, and excluded from recovery conclusions.

## Reproduction and unexecuted scope

See README.md for exact campaign commands and metric definitions. Machine-readable
commands for scripted runs are in their campaign's `commands.json`; additional
executed commands are in `results/manual-commands.txt`. Reproduce with fresh output
and database directories, preserving every resolved parameter.

Prepared, **not yet established by measurements**: 100M/1B writes, a populated
working set above RAM, one-hour and overnight stability. The larger-memory example
populates 50M keys x 1024 bytes (51.6GB application footprint); this first phase does
not justify a larger-than-memory performance claim from small cache-fit runs.

Additional code-level gaps: MANIFEST encoding has structural validation but no
checksum, despite the README's blanket CRC statement; the governor retains an
unbounded vector of chosen batch lengths and recomputes its average during stats
sampling. Neither was changed, and neither is presented as a measured cause of
latency or memory degradation.

## Verification

Formatting, normal/loom Clippy, library tests, integration tests, example tests,
and loom models were run. Final ordinary checks: **247 library tests passed** (3 ignored), **3 integration
and 3 example tests passed**, **3 loom models passed**, both Clippy configurations,
formatting and the dedicated governor syscall sweep passed. The full library suite
includes existing single/pairwise faults and power-loss tests. One newly ignored
regression is the explicit, separately executed **failing frozen-WAL durability
probe**; the other two ignores predate this phase. Integration success includes
oracle failure-detection tests and does not mean the observed storage defects pass.

An extra `cargo test --all-targets` diagnostic failed in the **existing** `basic`
benchmark: concurrent memtable writers use independent sequence ranges against
overlapping keys, violating the memtable's ascending-sequence assertion. The
assertion and benchmark are unchanged. This failure is distinct from the normal
unit/integration/loom results and from the newly discovered storage defects.

The performance CSVs were collected with the original bulk-read implementation,
before the simulator adapters were added. They remain valid evidence for that
recorded binary/revision, not a claim of a measured performance win from the
adapters. Binary hashes and source/dirty-state records are preserved.
