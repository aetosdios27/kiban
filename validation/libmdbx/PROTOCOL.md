# Kiban vs libmdbx protocol

Frozen before screening on 2026-11-22 UTC. Does Kiban's agent-memory profile change warm single-worker JSON memory API latency and resource use relative to a screened libmdbx configuration? Raw LMDB is the same-round control. No winner is assumed.

## Versions and fixed configuration

Use one locked Rust release binary with the existing readpath `remix` feature. Kiban index on uses the `KibanOptions::agent_memory(18)` settings and prepares the scan view at open. Index off disables both the point index and prefix scan extension, using the same borrowed-value prefix API with its sorted range fallback. Other settings remain at defaults apart from the cache override. Raw LMDB uses the existing heed3 0.22.0 adapter with lmdb-master3-sys 0.2.5, max readers and databases 200, and 10 GiB map.

Choose [libmdbx 0.9.0](https://crates.io/crates/libmdbx/0.9.0) and [mdbx-sys 14.4.0](https://crates.io/crates/mdbx-sys/14.4.0), both from [libmdbx-rs](https://github.com/vorot93/libmdbx-rs) at `aba108e11505c0a6c6cc890b65c63db78a51adc0`. The wrapper has a current crates.io release and typed sync modes, geometry, WRITEMAP and borrowed transaction/cursor decoding. Reth maintains a related binding in its workspace without a crates.io package under `reth-libmdbx`. The [Signet fork](https://crates.io/crates/signet-libmdbx) offered a published safe wrapper with older release activity at selection. The pinned native C library identifies itself as libmdbx 0.14.4, tag `v0.14.4`, commit `716ce9d5ae5458fa1de44097dc6e8f80ed1e4809`. The registry checksums are:

| Crate | SHA256 |
|---|---|
| libmdbx 0.9.0 | `3022c15786202637cef2c24fcd296f8455d4b201db65f2bc94823b00d9d1f400` |
| mdbx-sys 14.4.0 | `1f7c4502adb435d9aee303b3675aafa3f2f70465fff32c64c37859ccf340fd48` |

Use the default lexicographic B-tree, one short read transaction per query, a fresh prefix cursor for each latest-ten call, one committed write transaction per mutation, and no secondary index. The safe wrapper opens a default-table handle within the transaction and mediates writer begin/commit through its internal thread, which inherits CPU 10. Reuse the same code in the screen and primary. The lower/upper geometry are 1 MiB/10 GiB, automatic shrink disabled, max readers 200, default dirty-page and sync thresholds. Verify effective mode, flags, growth and page size at open. Reading uses mmap with or without WRITEMAP. WRITEMAP controls how the engine writes. libmdbx and LMDB have no configurable engine block-cache budget, and mapping size is a virtual address-space limit. COALESCE is absent from the vendored 0.14.4 public header. Screen LIFORECLAIM as the available page-recycling policy.

| Candidate | Page KiB | Growth MiB | WRITEMAP | NORDAHEAD | LIFORECLAIM |
|---|---:|---:|---|---|---|
| baseline | 4 | 16 | off | off | off |
| growth-1m | 4 | 1 | off | off | off |
| lifo | 4 | 16 | off | off | on |
| no-readahead | 4 | 16 | off | on | off |
| page-16k | 16 | 16 | off | off | off |
| writemap | 4 | 16 | on | off | off |
| writemap-16k | 16 | 16 | on | off | off |

## Workloads and selection

Seed 20261016, 64 agents, 200 ASCII text bytes, shared JSON serialization and bytewise keys `agent/NNNN/memory/<u64::MAX-id padded to 20 digits>`. Immutable creation order means a forward prefix scan yields latest-first. Read fixtures have 1M records. Each command runs 100k lookups and 100k latest-ten calls. Kiban bulk uses its existing bulk loader and final flush. Normal uses individual ID-order puts with the default write-buffer threshold and ordinary compaction picker. libmdbx and LMDB each load in ID order in one committed transaction. Each libmdbx candidate is prepared in Durable mode. The B-trees use one layout and have no LSM ingest/compaction distinction. Keep read fixtures fixed and reuse them in each applicable round. Use the Kiban default cache and 512 MiB for the two Kiban read layouts. Every primary read group repeats both mmap engines on their same fixture as contemporaneous controls.

Write commands use a fresh copy of a 10k bulk fixture and 1k calls each of remember, recall, recent, update and exactly 900 gets plus 100 updates in mixed. There are 2,100 mutation acknowledgments per command. Writes use Kiban's default cache. Each timed read and write readback is checked against the deterministic oracle outside its call timer. Every command verifies the full population and all-agent latest results before and after measurement. Writes also close, reopen and verify the complete final model. Any correctness failure invalidates the run. Preserve failed attempts and all completed rows. Resume only from validated receipts without selectively rerunning timed trials.

Minimize the equal-workload geometric mean of five-trial medians of mean call time. Read scoring has two cells, both read operations on the single libmdbx layout, since the Kiban cache budgets and LSM topology do not apply to libmdbx. Write scoring has six cells, remember/update/mixed in both durability modes. Select one complete candidate per family and break ties by candidate label. Five rounds rotate candidate/control order in each screen scenario. The independent primary matrix rotates Kiban index off/on, libmdbx winner and LMDB over five rounds in each scenario. Preserve all screens and commit the selections before primary measurements.

## Acknowledgment and checks

| Engine | Durable acknowledgment | Fast acknowledgment |
|---|---|---|
| Kiban | `put` followed by `sync` on every call | Buffered WAL may still be in userspace on return |
| libmdbx | `SyncMode::Durable`, one full committed transaction per call | `SyncMode::SafeNoSync`, one committed transaction per call. Retains the previous steady snapshot and permits rollback to it after system failure |
| LMDB | Default synchronous committed transaction per call | One commit with NO_SYNC and NO_META_SYNC, without WRITEMAP |

SafeNoSync uses mapped writes or OS file I/O by acknowledgment and supports recovery from process death with the kernel still running. This is stronger than Kiban fast's userspace buffering. Neither promises every fast acknowledgment survives system failure. SafeNoSync retains a crash-consistent steady state, at the cost of deferring page reuse. Do not measure UtterlyNoSync. Final finish/reopen verification applies to completed commands, after acknowledgment timing.

After the primary matrix, trace 32 durable mutations (16 insertions and 16 updates) of the write winner and both baseline native write paths. Inspect each acknowledgment interval for successful synchronous data file I/O (`fsync`, `fdatasync`, `msync(MS_SYNC)`, or verified write-through I/O). Kill the writer via SIGKILL after the last acknowledged commit and reopen to verify the full payload model and all-agent latest results. The check covers process death with OS caches intact.

## Host and reporting

Pin fixture preparation, benchmark children and diagnostics to CPU 10 with taskset. Leave sibling 11 unused by the campaign. Record its ticks and host load per command. Use CPUs 12–21 at nice +10 with two Cargo jobs for builds/tests and the coordinator. No builds/tests or diagnostics overlap timing in this campaign. Private target/data paths are `target/libmdbx-h2h` and `target/libmdbx-data`. No global page-cache drops. Full verification warms every fixture. Shared-host activity can affect results.

Report p50 and p99 as histogram upper bucket bounds, mean call time and its throughput, verified throughput, five-trial ranges, paired ratios against libmdbx and LMDB, logical/allocated bytes, and whole-process peak RSS. Retain per-trial values, fixture layouts, effective settings, command receipts and SHA256 provenance. Timers include key/JSON conversion and transaction work, and end before oracle checks, readback or destruction of the returned memory. Verified throughput counts stage wall time including checks. Final sync and reopen run outside the timers. Include tradeoffs and limits in the narrative.

## Deviation 1: adapter fairness (2026-11-22 UTC)

The original libmdbx 0.9.0 wrapper sent each write begin and commit through a synchronous handoff to an internal thread pinned alongside the caller on CPU 10. It also reopened the default-table handle inside every transaction. The revised adapter calls the existing pinned `mdbx-sys 14.4.0` API directly, begins and commits writes on the calling thread, and opens the default-table handle once per environment. Native libmdbx 0.14.4, environment flags including NOSTICKYTHREADS, geometry, sync modes, transaction boundaries, and loader order carry forward. The existing LMDB adapter retains its table handle as before.

Rerun the full read and write screens and five-round primary with the same seven candidates, selection rule, workloads, oracles, and affinity. Reprepare all fixtures because the first campaign removed its generated copies after validation. Repeat the three original durable checks and include the new write winner if it differs. Capture writer task IDs at readiness and after the last acknowledgment, trace thread creation and data-file sync calls, then kill and verify each writer as in the original protocol. [Original evidence](superseded-binding-handoff/README.md)

## Deviation 2: read primary rerun with a page-fault gate (2026-11-22 UTC)

During the adapter-fairness read primary, libmdbx recorded 2,177,499 major page faults across 12 of 20 commands, versus 78 in LMDB and 7 in indexed Kiban. One libmdbx command recorded 427,443 faults on a 688 MiB file (about 8.7 GB of faults per the review estimate). A director spot check of libmdbx alone on a quieter host recorded zero major faults and 1.632 µs recall p50 in bulk/default cache, versus 2.368 µs in the campaign. Four studies and builds ran concurrently. Host memory pressure evicted libmdbx file-backed pages, while Kiban's process-resident block cache was largely unaffected. The previous reads do not qualify as a warm comparison.

Keep the frozen screen winners, original fixtures and loaders, and the same four read scenarios, workloads, oracles, and CPU rules. Rerun only the read primary with five rotated rounds per scenario. GNU time must record zero major page faults over each of the four engine commands for the round to qualify. Run every engine in the scheduled order, preserve every receipt, and discard the entire round if any command faults. Repeat that round in the same order, up to 10 attempts per round, and report each discarded attempt. A correctness failure invalidates the run. Record `/proc/pressure/memory` and `MemAvailable` immediately before and after each command, alongside the existing CPU and host samples. Store qualifying results under `raw/measure-reads-quiet/` and keep discarded receipts under `raw/measure-reads-quiet/discarded/`. Archive the earlier read primary under `superseded-read-faults/`. The existing write primary remains the source for write results.
