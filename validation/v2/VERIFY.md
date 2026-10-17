# Verification and reproduction

Work only in `/home/aetos/.t3/worktrees/kiban/t3-a7586988`. Do not spawn agents,
push, or open a PR. Timed children use CPU 8. Leave its sibling CPU 9 unused.
Builds and correctness tests use CPUs 2,3,4,5,6,7,10,11 at nice 10 and never
overlap timed runs. Leave the crash sweep on CPUs 12–21 untouched.

The final baseline is main `8bbb4934f8d102ed0285c813a3770831b21c63e5`.
Earlier attribution and rejected duplicate-value artifacts retain their original
`2a083fa` provenance. `baseline.json` and `build-provenance.json` identify the
final binaries. `host.json` records the toolchain and host state.

1. Extract `git archive 8bbb4934` into `target/v2-main-8bbb4934`. Run
   `taskset -c 2,3,4,5,6,7,10,11 nice -n 10 python3 tools/agentbench/readpath/v2_build.py`.
   The script builds the archived readpath crate and the final crate with their
   locked release profiles. It links the identical `range.rs` probe against
   each engine library, with opt-level 2, one codegen unit, LTO and mimalloc.
   It freezes executables under `target/v2-final-bin` and records their hashes.
2. Run `taskset -c 2,3,4,5,6,7,10,11 nice -n 10 python3 tools/agentbench/readpath/verify.py validation/v2/final-checks`.
   This runs all 11 gates and captures engine source hashes before and after.
   Both crash/model suites cover 20,000 seeds from zero. Seed parity selects
   the initial write format, and crash reopens flip the write policy.
   `--quick` is for iteration and omits the long suites.
3. After all builds and tests finish, run
   `taskset -c 2,3,4,5,6,7,10,11 nice -n 10 python3 tools/agentbench/readpath/v2_pipeline.py --before target/v2-final-bin/before --after target/v2-final-bin/after --test-binary target/release/deps/kiban-9b697395093ef19c`.
   Use the release library test executable named in the release check log if
   its toolchain-dependent suffix differs. The pipeline refuses stale engine
   source hashes and never compiles. It prepares fresh main/final datasets,
   runs five alternating trials with adjacent LMDB controls, then runs ranges,
   writes, and maintenance serially. Timed children pin themselves to CPU 8.
   The final matrix's test executable is identified by hash in
   `measurement-provenance.json`.
4. `v2.py` exposes individual `prepare`, `read`, `range`, `write` and `space`
   stages. It refuses existing datasets or trial output. Preserve prior
   artifacts and choose fresh DATA/OUT directories before reproducing a run.
   Both candidates generate the same JSON model and seed, with bulk or normal
   ingest. Final Kiban preparation enables `point_index`; main keeps format 3.
   A null `cache_mib` invokes each build's actual RAM-sized default. Default
   trials are independent of explicit 512 MiB trials.
5. `v2_report.py` rechecks the 120 read and 40 write commands and emits
   `READS.md`, `WRITES.md` and per-trial distributions. Run
   `v2_final_report.py --verdict VERDICT --reason REASON` after evaluating the
   scan evidence. It replaces the preliminary host-RAM cache estimate with measured cgroup-aware
   budgets. It checks source/binary hashes, zero errors, record counts,
   range receipts and maintenance records, then writes `REPORT.md` and
   `final-summary.json`. Its verdict is an explicit review decision.

Maintenance uses five alternating index-off/index-on pairs in the final
engine. Each pair flushes two 500k-record inputs, compacts them, and checks
all 1M values after timing. BUILD includes merging, construction, output
installation and validation. Manifest COMMIT is separate. The fixed 8-byte
keys and 296-byte values differ from the JSON matrix. The standard Rust test
allocator differs from the read/write executables' mimalloc.

Locator records retain full keys and metadata; values stay in sorted blocks.
Resident locator and backing bytes share `block_cache_bytes`. Scans reuse
verified backing views. Old snapshots and compaction inputs can deny output
admission until reopen, and denied tables use the sorted path. Other SST
metadata and allocator overhead remain outside the raw-byte capacity.
Whole-SST buffering and MPHF construction need temporary workspace.

The final format-4 encoding uses feature flag 2 and `KIBANPH2`. The earlier
unpublished duplicate-value encoding is rejected. Format-3 readers reject
format 4; rewrite with `point_index=false` before downgrading. The option
remains false by default, and the RAM-sized cache default matches main.

The original 80-command hash/THP ablation is documented in
`ablation/REPORT.md`. Its source datasets were private copies, and its
checksum receipt excludes LOCK files. It establishes attribution for that
isolated layout; the final integrated matrix determines the shipping verdict.
