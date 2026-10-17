# Verification and reproduction

No subagents, pushes or PRs. This worktree starts from main 2a083fa. Timed
commands use CPU 8. All compilation and tests use CPUs 6,7,10,11 with nice 10;
no timed command overlaps this worktree's compilation/tests. Foreign workloads
on 2–5 and 12–21 are untouched. Shared LLC, power and memory contention remains.

1. Attribution: build `tools/agentbench/phash` in release mode separately, then
   `taskset -c 6,7,10,11 nice -n 10 python3 tools/agentbench/phash/ablation.py`.
   Run `sample_ablation.py` alongside it on the same driver cores. Then run
   `ablation_report.py`. Each timed child pins itself to 8 and asserts PR_GET
   THP state. `ablation/input-checksums.json` compares the private Kiban copies
   after all 80 commands with the supplied source copies, excluding LOCK.
2. Baseline: `git archive 2a083fa` into target/baseline-source; build its original
   readpath crate and copy the binary to target/v2-bin/before. See baseline.json
   and target/v2-bin/before-build.log. After: build the current readpath crate
   and freeze target/v2-bin/after. Both use the locked release profile.
3. Gates: `taskset -c 6,7,10,11 nice -n 10 python3
   tools/agentbench/readpath/verify.py validation/v2/final-checks`.
   `--quick` is for development and skips the two 20k-seed suites. The full
   command records all statuses and SHA256 of engine sources. Seeded suites
   start in both formats according to seed parity and flip write policy at
   crash reopens, checking acknowledged-prefix recovery and point/scan agreement.
4. Engine matrix: `taskset -c 6,7,10,11 nice -n 10 python3
   tools/agentbench/readpath/v2.py ACTION --before target/v2-bin/before
   --after target/v2-bin/after`, with ACTION prepare, read, write, space.
   The driver calls the existing run.py/harness and never compiles. Before
   data are private reflink/copy datasets; after Kiban data are freshly prepared
   with point_index=true using the same model/seed and bulk/normal write policy.
   Each candidate is followed by LMDB, reversing before/after order each trial.
   Final analysis: `python3 tools/agentbench/readpath/v2_report.py`.
5. Maintenance: run the already-built root release library test executable
   under `taskset -c 8`, selecting `point_index_maintenance_measurement
   --ignored --nocapture --test-threads=1`. It defaults to 1M records, five
   alternating pairs with the option off/on. It isolates direct flush time
   (including WAL barrier and publication), compaction BUILD (including output
   installation and section validation), and manifest COMMIT. Every resulting
   key/value is checked after timing. Its fixed-width binary keys/296-byte
   values are a maintenance workload, separate from the JSON read matrix.

The new point arena duplicates keys/values on disk; this is measured, not hidden
inside cache metadata. Admission is table-granular within block_cache_bytes.
Old snapshots/compaction inputs can prevent new output admission until reopen;
the sorted path remains correct. Format 3 cannot read format 4, so downgrade
requires rewriting with point_index=false. No default format/cache change.
