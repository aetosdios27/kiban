# Reproduce the small-cache campaign

Use an isolated worktree at main `06a457dbfe24aabe1e33a3763e1ac0e1a8d73e05`.
All timed children use CPU 6, including four-reader probes. Do not schedule
this task on its sibling CPU 7, or CPUs 3, 4, 8, or 9. Builds, tests, and
coordinators use CPUs 12 through 21 at nice 10. No subagents, push, or PR.

The commands below assume a fresh private `target/smallcache` directory.
Drivers refuse to overwrite trial configs or prepared databases. Freeze
`before` before applying the engine changes. Build the two readbench binaries
with the same locked Cargo dependencies and release profile.

    taskset -c 12-21 nice -n 10 cargo build --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
    mkdir -p target/smallcache/bin
    cp tools/agentbench/readpath/target/release/kiban-readbench target/smallcache/bin/before
    taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/smallcache.py prepare

`prepare` uses the frozen baseline for all fixtures, including format-4
fixtures, because main already supports writing that format. Each final
before/after pair reads the identical fixture for its index setting.
Preparation verifies every record after reopen. The raw space and topology
receipts are in `prepare/space.json` and the preparation logs.

After applying the final engine changes, run all gates and build the final
readbench. `READBENCH_BUILD_CPUS` accepts a comma-separated CPU list.

    READBENCH_BUILD_CPUS=12,13,14,15,16,17,18,19,20,21 taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/verify.py validation/smallcache/final-checks
    taskset -c 12-21 nice -n 10 cargo build --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
    taskset -c 12-21 nice -n 10 cc -shared -fPIC -O2 -Wall -Wextra -Werror tools/agentbench/readpath/syscall_probe.c -ldl -o target/smallcache/syscall_probe.so
    taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/smallcache_pipeline.py

The pipeline requires all 11 gates to pass and checks every engine source
hash. It freezes `after`, records both binary hashes, then runs inspection,
the complete warm matrix, the complete cold matrix, writes, a mapped-option
comparison at 32 MiB with the index on, and diagnostic profiles. In the
mapped comparison, both logical sides use the final binary: before selects
positional reads, and after selects the existing mapping option. It never
compiles or changes affinity outside the configured sets. The Linux perf
binary must be installed at `target/read-path-tools/root/usr/bin/perf`, as
expected by the existing profile driver. This run uses an eight-page perf
buffer because the host rejected 128 pages. Sampling-loss limitations remain
in the raw logs and reports.

Warm trials have 100,000 point and 100,000 latest-ten calls each, with complete
1M-record verification before and after. Cold trials have 10,000 point calls,
with an oracle check for every timed result. `cold.rs` drops this process's
dataset PTEs and file pages after open, using `madvise(DONTNEED)` followed by
file-specific `posix_fadvise(DONTNEED)`. It never drops global caches. Each
side has an adjacent LMDB control and five trials, reversing the side order
on alternate trials. Four readers share CPU 6; this is not four-core scaling.

Cold file-page eviction cannot discard heap data already retained by a full
point index. Default-cache/index-on rows may therefore perform no storage
reads. Report those as retained-resident results, rather than claiming a
cold-storage speedup. The cold receipts include storage bytes and major and
minor faults so that interpretation can be checked.

Write trials start from fresh copies of the 10k fixtures. They use the
existing five-stage harness with 10,000 operations per stage, in fast and
durable modes. They preserve the harness's default write cache options.
Final flush/sync/verification time is outside per-call timers and is retained
in the completion row. Index off and on are measured separately.

`smallcache_report.py FOLDER` checks command affinity, exact operation counts,
complete verification receipts, errors, all five pairs, and warm process
affinity. It writes medians, full trial ranges, and paired Kiban/LMDB ratios.
Cold summaries retain p999, storage bytes, and faults. Never use the syscall
preload for latency trials: its clocks and interposition add overhead.

The engine source hashes in `final-checks/source-hashes.json` must match the
final commit. The initial error-classification failure and its fix are recorded
in `decisions.tsv`; they are development evidence rather than final failures.
