# Kiban vs tuned RocksDB, with LMDB control

Kiban has a clear warm-read latency advantage over the screened RocksDB configuration for this single-worker JSON memory API. With the opt-in point index, same-round median point p50 ratios imply **2.24–2.69× faster** calls across the four read groups. Latest-ten calls are **1.75–2.41× faster**. RocksDB wins footprint and has the stronger WAL-on fast acknowledgment. Durable mutation latency has no broad winner; RocksDB has slightly higher call throughput in these trials.

This is a controlled **shared-host** comparison. CPU 11 was not exclusively idle. These results do not establish exclusive-core performance or a globally optimal RocksDB configuration. [PROTOCOL.md](PROTOCOL.md) fixes the screening and measurement rules.

## Workload and timing boundary

The experiment changes the readpath harness and evidence only. Engine `src/` remains at `06a457db`, with Kiban's opt-in perfect-hash point index disabled and enabled as separate rows.

The shared memory model has 64 agents, 200 ASCII text bytes, identity, immutable creation sequence and revision, with seed **20261016**. All three engines receive exactly the same composite key and JSON value bytes. IDs deterministically imply agent and creation sequence. This is an application model assumption, not a secondary lookup offered for arbitrary independent identifiers. `recall-by-id` fetches one value; `recent-for-agent` returns the latest ten records by descending creation sequence. With 64 agents, that workload revisits 64 distinct ten-record slices. The 1M population expands the point-read working set, not the number of distinct recent-read positions. These are short, hot latest-K queries, not long random history scans.

Read fixtures contain 1M records. Each command performs 100k calls of each read workload. Bulk means one sorted bulk-loaded SST for each LSM; normal means individual ID-order writes with natural compaction. RocksDB normal is flushed and waits for background jobs before verification/measurement, without a manual full compaction. Kiban normal uses its existing default write-buffer flush threshold and normal compaction picker. Actual files, levels, entries, pending compaction bytes and jobs are recorded rather than assuming equivalent topologies. Kiban normal has 1 L1, 6 L2, 77 L3 files with the index off, and 1 L1, 7 L2, 91 L3 files with it on. These are complete configuration comparisons, not an index-only test with identical normal topologies. Uncompressed RocksDB normal has three L0 and four L6 files. LZ4 normal has 3 L0 files, 3 L6 files. All normal fixtures are settled before reads and remain fixed across trials. LMDB uses its existing ID-order, single-transaction loader in both comparison groups.

Write commands start from fresh copies of the 10k fixtures, then run 1k remember, 1k recall, 1k recent, 1k update and 1k mixed calls. Mixed is exactly 900 point reads and 100 updates, totaling **2100 mutations** across the command. Updates preserve creation order. These are short memtable-resident writes, not steady-state compaction throughput. Every write is one API call, one put or one LMDB committed transaction; no group commit/batching is added to any timed backend.

Each timer covers one complete backend call: key construction, write serialization, owned result construction, read JSON decoding and requested commit durability. Initial Memory construction for writes is outside the timer for every backend, as in the existing runner. Every timed return is checked against its complete deterministic payload outside the timer. Every write is read back separately. Full point-population and all-agent latest-ten verification runs before and after commands; writes close, reopen and verify the complete final revision model again. Failure aborts the command before a successful receipt is accepted.

Call ops/s is call count divided by accumulated per-call duration. Verified ops/s includes oracle construction, checks, histogram and write readback within a stage. Neither is continuous application throughput or includes final persistence/full verification. Completion duration is retained separately. Returned Memory destruction is outside the timer; temporary backend buffers die inside the call. RocksDB `get_pinned_opt` with reusable default point ReadOptions and raw iterators decode from borrowed native buffers, LMDB decodes from mapped bytes, and Kiban decodes its API's owned byte vector. No extra native copy is inserted to equalize a Kiban API limitation. Histogram percentiles are upper bucket bounds, with <3.125% relative bucket error above 32 ns.

## Versions and RocksDB options

Static **RocksDB 11.8.1**, `rocksdb = 0.25.0`, `librocksdb-sys = 0.19.0+11.8.1`, native build source `abeebd9630f11bd08c28b7bd43c7bdfc62050654`. The bundled `build_version.cc` identifies tag v11.8.1. LZ4 is bundled through `lz4-sys 1.11.1+lz4-1.10.0`. The existing LMDB control is `heed3 0.22.0` with `lmdb-master3-sys 0.2.5`; its vendored master3 header reports 0.9.90 with date May 1, 2017. This identifies the bundled fork, not an upstream release claim. It retains max_dbs=200, max_readers=200 and the 10 GiB map. Rust dependencies and checksums are fixed in the harness Cargo.lock. All engines run from the same release binary, with MiMalloc for Rust and RocksDB's normal native allocator on glibc 2.43. The Rust allocator crates are `mimalloc 0.1.48` and `libmimalloc-sys 0.1.44`; JSON uses `serde_json 1.0.143`. The shared readpath release profile uses opt-level 2, one codegen unit, LTO, panic abort, and debug symbols. Native RocksDB uses the crate's release C++20 build with NDEBUG; compiler flag environment is recorded. Exact host/build information is in [host.json](host.json), binary/source SHA256 in [raw/metadata.json](raw/metadata.json).

Every RocksDB process uses a block-based table and explicitly sets:

| Option | Setting |
|---|---|
| block cache | LRU; exact Kiban byte budget (512 MiB or the host's Kiban default) |
| cache_index_and_filter_blocks | true |
| pin_l0_filter_and_index_blocks_in_cache | true |
| data_block_index_type | binary-and-hash |
| block_size | 4096 or 16384, screened |
| filter policy | full Bloom 10 bits/key or Ribbon at Bloom-equivalent 10 bits/key, screened |
| whole_key_filtering | true; extractor also enables prefix filtering |
| prefix_extractor | fixed 11 bytes, `agent/NNNN/` |
| memtable_prefix_bloom_size_ratio | 0.1 |
| compression | none; separate LZ4 row |
| allow_mmap_reads | false |
| row_cache | absent |
| max_background_jobs | 2; all threads inherit CPU 10 affinity |
| use_fsync | false (Linux fdatasync path) |
| create_if_missing | true |
| ReadOptions iterate_upper_bound | `agent/NNNN/memory0`, exclusive |
| ReadOptions prefix_same_as_start | true for prefix candidate, false for total order |
| ReadOptions total_order_seek | false for prefix candidate, true for total order |
| WriteOptions sync | true for durable, false for fast |
| WriteOptions disableWAL | false except explicit no-WAL row |
| Other ReadOptions / WriteOptions | crate defaults; no fixed snapshot; checksum verification and cache filling enabled |
| WaitForCompactOptions | flush=true, timeout=120000000 microseconds |
| IngestExternalFileOptions | defaults; one external sorted SST, source file removed after ingestion |

Default automatic WAL flushing remains enabled (`manual_wal_flush=false`); no concurrent writers/group commit, direct I/O, statistics collection or manual read snapshots are added. Default bytewise comparator, skiplist memtable, checksum verification, level compaction and remaining settings are retained. Every effective native option, including untouched defaults, is preserved in preparation `*.OPTIONS.txt` files, with requested settings and runtime cache capacity checked in process logs. The default 4 KiB block size is screened against 16 KiB. Prefix and total-order paths share identical table files for each filter/block candidate, so their selection does not compare different fixture contents. The exclusive upper bound shares the extractor's agent prefix with the seek key.

The bulk loader writes records in composite-key order using `SstFileWriter`, producing one externally ingested SST. Normal loads individual puts with WAL enabled, then flushes and waits for scheduled compactions. It uses ordinary automatic leveling rather than converting the normal fixture into a fully compacted bulk layout. Default 64 MiB write buffers are deliberately retained for RocksDB; Kiban normal uses its own default 4 MiB budget. The two engines have different normal compaction algorithms and resulting layouts; this comparison does not impose an artificial identical level count.

[RocksDB's Ribbon documentation](https://rocksdb.org/blog/2021/12/29/ribbon-filter.html) describes its space/CPU tradeoff. "10" for Ribbon is an equivalent Bloom false-positive target, not a claim of literally 10 physical filter bits per key. Both policies are actually built and measured. Optional mmap and row-cache configurations were omitted; this is a tuned candidate search, not a claim to exhaust all RocksDB configurations.

## Fairness and acknowledgment guarantees

| Row | Per-write acknowledgment |
|---|---|
| Kiban durable | put followed by `Kiban::sync()`, including WAL buffer flush and fdatasync |
| RocksDB durable | WAL enabled, `WriteOptions::sync=true` for each mutation; returns after native WAL sync |
| LMDB durable | Default synchronous committed transaction per mutation |
| Kiban fast | Buffered WAL; acknowledged writes can remain in userspace and be lost on process death |
| RocksDB fast | WAL enabled, sync=false, default automatic WAL flush to OS; process-crash consistency, no per-write power-loss durability |
| RocksDB no-WAL | disableWAL=true, sync=false; no durability for acknowledged memtable writes, including process death |
| LMDB fast | NO_SYNC and NO_META_SYNC, one committed transaction per mutation; no per-commit durability promise, no WRITE_MAP |

RocksDB fast has a stronger acknowledgment than Kiban fast. A fast-mode latency ratio is not an equal-guarantee durability result. Final sync/flush and reopened verification establish finished-fixture correctness, not durability when each earlier acknowledgment was returned. Durable rows are the per-write persistence comparison. These labels follow the engine implementations and [RocksDB's WAL contract](https://github.com/facebook/rocksdb/wiki/Write-Ahead-Log-%28WAL%29) and [basic write options](https://github.com/facebook/rocksdb/wiki/Basic-Operations); this experiment is not a power-cut test. Separate libc sync-call counts audit the actual paths without contaminating reported timings.

The host default Kiban block-cache capacity is **449,030,592 bytes (428.229 MiB)**; the explicit row uses 536,870,912 bytes. RocksDB receives the exact same byte capacity in each group. RocksDB charges cached data, index and filter blocks to its block cache. Kiban metadata overhead is outside its byte budget. Raw block-cache capacity is matched. Total RSS is not: metadata, native allocations, pinned blocks and allocator overhead live outside or alongside the configured cache, and all engines share the OS page cache. LMDB's 10 GiB map is virtual reservation, not a 10 GiB resident cache. Both read cache budgets cover the base datasets. Full verification warms every process, so these results say nothing about cold starts, disk-resident random reads or small-cache pressure.

Logical size includes all regular database files after process exit, including WAL, LOG, MANIFEST and OPTIONS. Allocated bytes are st_blocks×512. Btrfs compression/reflinks make this different from unique physical extents. Write figures are after final flush/settle, not peak WAL size or write amplification. `/usr/bin/time -v` peak RSS covers the entire command including open, full verification and write reopen.

## Scheduling and reproduction

All preparation and benchmark processes are pinned to **CPU 10 only**. This task assigns no work to sibling CPU 11 or the other tasks' CPUs 3/4, 6/7, 8/9. Build/test commands use `taskset -c 12-21 nice -n 10`; coordinators use CPUs 12–21. Builds and tests finish before the timed campaign begins. CPU 10/11 tick deltas and host load are retained per command. The host uses `intel_pstate` with the powersave governor and turbo enabled; frequency and package power are not locked. Affinity is not exclusive reservation: other host processes share CPU scheduling, LLC, memory, storage and package power. Sibling activity is audited with the results.

Each scenario has five rotated trials and an LMDB control in every trial. All five-entry final read/durable-write scenarios put each engine in each position once. Fast-write scenarios have six entries over five rotations; screens have nine entries over five rotations. Those retain cyclic rotation without claiming perfect position balance. Ratios pair rows from the same round before taking the median. Dividing displayed medians can differ from the paired ratio.

Run from a fresh checkout/output/data directory. The driver stores fixtures at `target/rocksdb/data` independently of `--out`; move that directory aside before a fresh reproduction. Keep the recorded data for review. The Clang paths below resolve this host's missing default Clang resource directory; use your installed Clang paths elsewhere. Never overlap build/check commands with a measurement campaign.

```sh
export LIBCLANG_PATH=/usr/lib64/llvm15/lib
export BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/usr/lib64/llvm15/lib/clang/15.0.7
taskset -c 12-21 nice -n 10 cargo test --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 nice -n 10 cargo clippy --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml -- -D warnings
taskset -c 12-21 nice -n 10 cargo fmt --check --manifest-path tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 nice -n 10 cargo build --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
mkdir -p target/rocksdb
taskset -c 12-21 nice -n 10 cc -O2 -Wall -Wextra -Werror -shared -fPIC tools/agentbench/readpath/rocksdb_sync_probe.c -ldl -o target/rocksdb/sync-probe.so
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_bench.py prepare --out target/rocksdb/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_bench.py screen --out target/rocksdb/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_bench.py select --out target/rocksdb/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_bench.py prepare-lz4 --out target/rocksdb/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_bench.py measure --out target/rocksdb/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_bench.py probe --out target/rocksdb/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_report.py target/rocksdb/reproduction/raw
```

The driver freezes binary, harness, engine sources, candidate list and selection rule before preparation. `--resume` accepts only the same provenance, configurations and successful result hashes. Interrupted commands have no successful receipt; private write copies are recreated on resume. Losing screens stay intact. [decisions.tsv](decisions.tsv) records choices and evidence.

## Completed audit and tuning correction

The final campaign passes **425 benchmark commands and 56,725,000 timed calls with zero errors**: 270 screening commands and 155 independent primary commands. The primary matrix contains 20,275,000 timed calls. All benchmark affinity receipts show CPU 10. Full payload, final-population, latest-ten, and reopened-write verification pass. Native normal-layout receipts show zero running or pending flush/compaction jobs and zero estimated pending compaction bytes. The audit confirms read SST layouts stay fixed across all trials and both cache budgets.

Ten Rust release tests pass, including both filters, block sizes, seek modes, bulk/normal layouts, compression and write-policy cases, reopened updates, and wrong-revision rejection. Clippy with warnings denied, formatting, and the final build pass before fresh fixture preparation. [Check receipts](checks/receipt.json), [audit.json](audit.json), [fixture-layouts.json](fixture-layouts.json), [TRIALS.csv](TRIALS.csv), and [summary.json](summary.json) retain the evidence. The latter includes per-trial values, ranges, verified throughput, and paired p50/p99/throughput ratios.

An initial implementation used `DB::get_pinned`, which constructs default ReadOptions on every point call in rust-rocksdb 0.25.0. The final adapter reuses default point ReadOptions and calls `get_pinned_opt`, without a fixed snapshot. Each latest-K call still creates a fresh iterator and its owned ReadOptions. All screens and primary trials were restarted with fresh fixtures after this correction. The original 270 screens and 61 accepted primary commands, their source snapshots, and an explicit interruption notice remain in [archive-convenience-api](archive-convenience-api/NOTICE.md). Its separate audit passes 331 accepted commands and 48,650,000 timed calls, excluded from every final summary. No performance improvement is inferred by comparing the two campaigns across different host intervals.

Both final families select **Ribbon at Bloom-equivalent 10 bits/key, 16 KiB blocks, and total-order seek with the exclusive upper bound**. The prefix extractor and stored prefix filters remain configured. Total-order seek disables prefix-seek filtering for that iterator. Every candidate below uses one complete configuration, not a blend of point and scan winners.

The read score margin over Bloom/16 KiB/total is only about 1.4%. The write winner is effectively tied with Bloom/16 KiB/prefix, 39.4897 versus 39.4960 µs. These choices follow the predeclared rule, not a claim of universal Ribbon superiority. Prefix filtering has little opportunity to prune these populated-agent latest-ten queries; extra prefix checks are a plausible cost, not a measured causal decomposition. All losing timings, controls, RSS and disk ranges remain in [SCREEN.md](SCREEN.md) and [SCREEN-RESOURCES.md](SCREEN-RESOURCES.md).

## Frozen screening choice

| Complete configuration | Read score µs | Write score µs | Selected family |
|---|---:|---:|---|
| bloom-16384-prefix | 5.506 | 39.496 |  |
| bloom-16384-total | 5.252 | 40.257 |  |
| bloom-4096-prefix | 5.633 | 40.085 |  |
| bloom-4096-total | 5.253 | 40.903 |  |
| ribbon-16384-prefix | 5.587 | 41.067 |  |
| ribbon-16384-total | 5.179 | 39.490 | read, write |
| ribbon-4096-prefix | 5.708 | 41.680 |  |
| ribbon-4096-total | 5.405 | 44.200 |  |

Scores are geometric aggregates of mean call times, not public-operation latencies. All losing rows and ranges are in SCREEN.md.

## 1M reads

Cells are **p50 µs / p99 µs / call kops/s**, each the median of five trials.

| Layout/cache | Operation | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | LMDB |
|---|---|---:|---:|---:|---:|---:|
| bulk/512 | recall-by-id | 1.760 / 2.752 / 563.6 | 0.960 / 1.440 / 1038.9 | 2.304 / 4.352 / 423.2 | 2.752 / 4.992 / 361.2 | 1.472 / 2.368 / 674.9 |
| bulk/512 | recent-for-agent | 3.008 / 6.400 / 315.7 | 3.072 / 6.912 / 309.2 | 4.736 / 9.728 / 200.9 | 5.632 / 11.008 / 172.4 | 2.432 / 5.120 / 389.3 |
| bulk/default | recall-by-id | 1.824 / 3.136 / 545.3 | 0.992 / 1.408 / 1010.3 | 2.688 / 4.864 / 357.6 | 2.496 / 4.032 / 402.4 | 1.408 / 2.176 / 716.6 |
| bulk/default | recent-for-agent | 3.136 / 5.888 / 313.9 | 3.008 / 6.144 / 325.9 | 5.248 / 8.960 / 187.5 | 5.248 / 9.984 / 184.6 | 2.496 / 5.376 / 387.7 |
| normal/512 | recall-by-id | 1.888 / 3.200 / 518.6 | 1.184 / 1.664 / 837.7 | 2.752 / 4.736 / 363.7 | 2.816 / 4.864 / 354.2 | 1.376 / 2.016 / 715.6 |
| normal/512 | recent-for-agent | 3.392 / 5.504 / 288.7 | 3.136 / 5.248 / 311.3 | 7.552 / 11.008 / 130.9 | 7.680 / 12.800 / 128.4 | 2.240 / 3.648 / 436.1 |
| normal/default | recall-by-id | 1.696 / 2.560 / 587.5 | 1.088 / 1.440 / 921.8 | 2.496 / 4.032 / 402.9 | 2.496 / 4.096 / 401.1 | 1.280 / 1.696 / 776.6 |
| normal/default | recent-for-agent | 3.072 / 4.992 / 319.5 | 3.008 / 4.864 / 326.3 | 7.168 / 10.496 / 138.1 | 7.168 / 10.496 / 138.7 | 2.112 / 3.392 / 475.0 |

## 10k writes

Same cell convention. RocksDB fast keeps WAL enabled; no-WAL explicitly removes that acknowledgment guarantee.

| Mode | Operation | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | RocksDB no-WAL | LMDB |
|---|---|---:|---:|---:|---:|---:|---:|
| fast | remember | 0.608 / 4.992 / 1173.4 | 0.592 / 5.120 / 1211.3 | 2.368 / 5.376 / 324.3 | 2.304 / 5.248 / 372.1 | 0.832 / 2.496 / 954.0 | 6.016 / 12.288 / 153.5 |
| fast | update | 0.672 / 5.248 / 948.1 | 0.688 / 6.144 / 1065.9 | 2.432 / 7.168 / 339.9 | 2.368 / 5.376 / 379.4 | 0.896 / 2.240 / 974.2 | 6.144 / 9.984 / 156.7 |
| fast | mixed-90-10 | 0.784 / 1.600 / 1195.6 | 0.592 / 1.216 / 1613.0 | 1.824 / 4.480 / 532.3 | 1.824 / 6.016 / 485.7 | 1.696 / 2.880 / 600.6 | 0.704 / 6.528 / 823.5 |
| durable | remember | 557.056 / 1638.400 / 1.5 | 557.056 / 1703.936 / 1.5 | 540.672 / 1540.096 / 1.7 | 540.672 / 1638.400 / 1.6 | — | 1179.648 / 2818.048 / 0.8 |
| durable | update | 557.056 / 1933.312 / 1.6 | 540.672 / 1671.168 / 1.5 | 540.672 / 1540.096 / 1.6 | 524.288 / 1671.168 / 1.6 | — | 1179.648 / 2686.976 / 0.7 |
| durable | mixed-90-10 | 2.112 / 884.736 / 15.1 | 1.440 / 622.592 / 15.7 | 3.456 / 704.512 / 16.1 | 3.328 / 671.744 / 14.6 | — | 1.344 / 1998.848 / 7.3 |

## Paired Kiban latency ratios

Each cell is **p50 ratio / p99 ratio**, Kiban divided by its same-round comparator, then median across five rounds. Below 1 favors Kiban.

| Layout/cache | Operation | Index | / RocksDB tuned | / LMDB |
|---|---|---|---:|---:|
| bulk-512-fast | recall-by-id | 0 | 0.705 / 0.603 | 1.196 / 1.108 |
| bulk-512-fast | recall-by-id | 1 | 0.403 / 0.317 | 0.652 / 0.581 |
| bulk-512-fast | recent-for-agent | 0 | 0.554 / 0.556 | 1.237 / 1.111 |
| bulk-512-fast | recent-for-agent | 1 | 0.568 / 0.600 | 1.231 / 1.306 |
| bulk-None-fast | recall-by-id | 0 | 0.692 / 0.679 | 1.295 / 1.342 |
| bulk-None-fast | recall-by-id | 1 | 0.372 / 0.366 | 0.705 / 0.695 |
| bulk-None-fast | recent-for-agent | 0 | 0.561 / 0.621 | 1.282 / 1.095 |
| bulk-None-fast | recent-for-agent | 1 | 0.573 / 0.545 | 1.235 / 1.262 |
| normal-512-fast | recall-by-id | 0 | 0.686 / 0.686 | 1.372 / 1.514 |
| normal-512-fast | recall-by-id | 1 | 0.430 / 0.371 | 0.837 / 0.823 |
| normal-512-fast | recent-for-agent | 0 | 0.432 / 0.477 | 1.514 / 1.536 |
| normal-512-fast | recent-for-agent | 1 | 0.415 / 0.477 | 1.400 / 1.439 |
| normal-None-fast | recall-by-id | 0 | 0.697 / 0.635 | 1.325 / 1.509 |
| normal-None-fast | recall-by-id | 1 | 0.447 / 0.355 | 0.850 / 0.833 |
| normal-None-fast | recent-for-agent | 0 | 0.429 / 0.476 | 1.460 / 1.472 |
| normal-None-fast | recent-for-agent | 1 | 0.420 / 0.463 | 1.460 / 1.434 |
| bulk-None-durable | remember | 0 | 1.048 / 1.000 | 0.471 / 0.568 |
| bulk-None-durable | remember | 1 | 1.030 / 1.106 | 0.459 / 0.750 |
| bulk-None-durable | update | 0 | 1.000 / 1.128 | 0.450 / 0.646 |
| bulk-None-durable | update | 1 | 1.029 / 1.085 | 0.438 / 0.622 |
| bulk-None-durable | mixed-90-10 | 0 | 0.635 / 1.385 | 1.600 / 0.500 |
| bulk-None-durable | mixed-90-10 | 1 | 0.417 / 0.974 | 1.071 / 0.378 |
| bulk-None-fast | remember | 0 | 0.250 / 0.929 | 0.101 / 0.424 |
| bulk-None-fast | remember | 1 | 0.257 / 0.976 | 0.101 / 0.444 |
| bulk-None-fast | update | 0 | 0.276 / 0.889 | 0.109 / 0.554 |
| bulk-None-fast | update | 1 | 0.284 / 0.964 | 0.114 / 0.649 |
| bulk-None-fast | mixed-90-10 | 0 | 0.431 / 0.379 | 1.195 / 0.245 |
| bulk-None-fast | mixed-90-10 | 1 | 0.333 / 0.243 | 0.804 / 0.190 |

## 1M disk and memory

Cells are **logical disk MiB / whole-process peak RSS MiB**, median of five runs. Allocated bytes and ranges are in MEASURE-RESOURCES.md.

| Layout/cache | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | LMDB |
|---|---:|---:|---:|---:|---:|
| bulk/512 | 314.564 / 433.574 | 391.176 / 426.590 | 304.064 / 337.227 | 217.766 / 337.281 | 799.305 / 820.203 |
| bulk/default | 314.564 / 436.105 | 391.176 / 428.441 | 304.234 / 336.707 | 217.935 / 336.918 | 799.305 / 820.309 |
| normal/512 | 314.573 / 441.742 | 391.197 / 477.758 | 304.114 / 337.152 | 218.615 / 337.238 | 799.305 / 820.281 |
| normal/default | 314.573 / 441.664 | 391.197 / 477.781 | 304.285 / 337.156 | 218.786 / 336.797 | 799.305 / 820.355 |

## Durability audit and completion cost

The seven separate sync probes perform the same 2,100 mutations, final sync/flush and full reopened verification on private copies. Their timings are excluded from latency tables. Each engine's durable mode adds exactly **2,100 fdatasync calls** over its fast mode. No failed sync calls occur.

| Whole-process diagnostic | fsync | fdatasync | Failed calls |
|---|---:|---:|---:|
| kiban-durable | 9 | 2102 | 0 |
| kiban-fast | 9 | 2 | 0 |
| lmdb-durable | 0 | 2101 | 0 |
| lmdb-fast | 0 | 1 | 0 |
| rocksdb-durable | 6 | 2107 | 0 |
| rocksdb-fast | 6 | 7 | 0 |
| rocksdb-no-wal-fast | 6 | 7 | 0 |

These are whole-process libc call counts, including setup and finalization. They do not count every kernel persistence mechanism, such as implicitly synchronous writes. Equal fdatasync counts do not imply identical data/metadata work. No-WAL still performs final SST persistence; that does not make earlier acknowledgments durable.

RocksDB tuned durable remember throughput is 1,652 calls/s versus indexed Kiban's 1,533; update is 1,620 versus 1,533. Both have the same 540.672 µs median update p50. Update p50 ranges overlap: RocksDB 524.288–589.824 µs, indexed Kiban 516.096–589.824 µs. RocksDB update p99 spans 1.147–4.063 ms and indexed Kiban spans 1.638–2.425 ms. These runs support a modest observed throughput edge, not a general durable-latency winner. The LZ4 row's p50 and throughput also differ; its update p99 reaches 5.505 ms in one trial.

Mixed durable p50 describes mostly reads, while its mean includes 100 sync writes. Indexed Kiban mixed p50 is 1.440 µs versus RocksDB's 3.456 µs, yet RocksDB has slightly higher call throughput, 16,140 versus 15,713 calls/s. A low mixed p50 is not proof of higher durable-write throughput. Raw maxima and per-stage read/write distributions remain in JSONL and summary.json.

Final sync, SST compression/flush, settlement and full verification are outside per-call latency. [COMPLETION.md](COMPLETION.md) gives their separate duration. These short write stages fit in memtables; LZ4 SST compression occurs at preload or final flush, while WAL compression remains disabled. These rows do not measure steady-state compaction or compression throughput.

## CPU evidence and limits

Ten separate scoped counting loops perform one million full API calls each, bulk/512. FIFO control enables events only around the loop, excluding open, trace generation and full pre/post verification. Counters are user-mode cpu_core events, fully scheduled without multiplex scaling. These loops include a small result digest and returned Memory destruction, unlike the per-call latency boundary. They are diagnostic evidence, not extra latency trials.

| API row | User cycles/op | Instructions/op | Branch misses/op | LLC misses/op | dTLB misses/op |
|---|---:|---:|---:|---:|---:|
| kiban-index0-stage1 | 6988.69 | 8333.17 | 2.81 | 9.23 | 1.73 |
| kiban-index0-stage2 | 11582.25 | 49119.52 | 12.39 | 0.04 | 0.12 |
| kiban-index1-stage1 | 3717.90 | 5640.87 | 0.74 | 7.96 | 0.13 |
| kiban-index1-stage2 | 11314.66 | 48449.89 | 13.10 | 0.01 | 0.15 |
| lmdb-stage1 | 5481.40 | 7897.38 | 10.07 | 12.10 | 2.00 |
| lmdb-stage2 | 9365.54 | 42346.46 | 14.57 | 0.01 | 0.07 |
| rocksdb-lz4-stage1 | 9858.03 | 18624.05 | 16.51 | 8.85 | 4.53 |
| rocksdb-lz4-stage2 | 20578.31 | 69187.18 | 26.77 | 0.01 | 0.22 |
| rocksdb-stage1 | 10622.49 | 18619.63 | 16.65 | 10.79 | 4.65 |
| rocksdb-stage2 | 20775.93 | 69032.10 | 26.97 | 0.02 | 0.21 |

Indexed Kiban point calls use about 5,641 user instructions versus RocksDB's 18,620 and LMDB's 7,897. Latest-ten uses about 48,450 versus RocksDB's 69,032 and LMDB's 42,346. This supports CPU work in the warm API path as the read limiter, consistent with the latency ordering. It does not isolate a causal percentage for hashing, cache management, JSON, FFI, or iterator machinery. The shared decoder and owned-result API are included throughout.

Call-stack sampling was attempted separately after the complete latency matrix. The host rejected hardware and software sampling, including a user-only fallback: `Failure to open any events for recording` with perf_event_paranoid=2 and an access-control-policy diagnostic. [Raw failure](sampling-bulk/rocksdb-stage1.log) and [fallback attempts](sampling-bulk/fallback-permission.log) are retained. Host policy was not changed. No successful call-stack or normal-layout sampling result is claimed. Counting profiles succeeded and are in [profile](profile/metadata.json).

## Where RocksDB wins, and the verdict

RocksDB wins the measured footprint comparison. Its uncompressed SSTs occupy about **303.2 MiB**, versus Kiban's **314.6 MiB without the point index** and **391.2 MiB with it**. LZ4 reduces RocksDB SSTs to **217.6 MiB bulk** or **218.4 MiB normal**, about 28% below its uncompressed SST size. The point-index configuration adds about 76.6 MiB to Kiban's SSTs. These table-format and compression tradeoffs are visible without counting operational logs; [SST-FOOTPRINT.md](SST-FOOTPRINT.md) separates them from total database size. Native LOG retention explains small total-size growth across repeated opens.

Whole-process RocksDB peak RSS is roughly **337 MiB** in all four read groups. Kiban without the index uses 434–442 MiB and with the index 427–478 MiB. This is a measured process peak under full verification, not an allocation breakdown or total host page-cache budget. LMDB's roughly 799 MiB disk and 820 MiB RSS reflect the existing ID-order loader and mapped-page control. They do not establish the best footprint achievable with a packed LMDB loader.

RocksDB's WAL-on fast row also provides a stronger process-crash acknowledgment than Kiban fast. Kiban's fast insert/update latency advantage cannot be counted as an equal-guarantee durability win. The explicit no-WAL row shows the CPU cost of removing RocksDB WAL writes: remember p50 drops from 2.368 to 0.832 µs and update from 2.432 to 0.896 µs. Indexed Kiban is still 0.592 and 0.688 µs respectively, with different recovery floors and no per-write durability in either relaxed comparison. RocksDB no-WAL wins the corresponding p99 comparison: 2.496 and 2.240 µs versus indexed Kiban fast at 5.120 and 6.144 µs. Kiban has lower medians, while this no-durability RocksDB variant has lower mutation tails.

Kiban's supported advantage is low warm latency for this keyed, single-worker JSON API. Both Kiban rows beat the screened RocksDB block-cache row in read p50 here; the point index gives the larger point advantage at a disk-space cost. Indexed Kiban also beats the existing LMDB control in point p50 by paired factors of about 1.18–1.53. LMDB remains faster for latest-ten, with indexed Kiban/LMDB p50 ratios of 1.23–1.46. No engine wins every criterion.

The read advantages do not make RocksDB a poor general choice. If storage footprint, mature engine features, or stronger relaxed acknowledgments matter, these results give concrete reasons to choose it. Durable mutation differences are small relative to their overlap and tails. Optional row-cache and mmap configurations were not run, so this is the best screened block-cache configuration, not an exhaustive RocksDB optimum.

The experiment excludes cold starts, small-cache pressure, arbitrary IDs independent of agents, long history scans, deletes, vector search, multi-process contention, concurrent readers/writers, batched commits, and sustained 1M-record write/compaction throughput. Application impact depends on the fraction of request time spent in these storage calls.

## Host noise and final source verification

Every benchmark affinity receipt is CPU 10, but **strict sibling-idle isolation was not achieved**. CPU 11 busy time over the 155 primary command intervals has median **3.61%**, range **0.00–30.48%**. These intervals include open and verification, not only timed calls; short write intervals are coarse. One-minute load ranges **2.44–9.96**, median 3.71. Rotation and LMDB controls reduce drift bias but cannot create an exclusive core or exclusive NVMe device. The broad warm-read ratios and diagnostic instruction counts are useful shared-host evidence; small cache/filter and durable-tail differences need an isolated-host rerun before generalization.

The audit matches every frozen engine and harness source hash and the final binary hash. Engine `src/` remains unchanged at `06a457db`. No push or PR is made. All final primary results, losing screens, sync diagnostics, counting profiles, interrupted superseded evidence and decisions are retained in this worktree and committed.

The full tables regenerate with `rocksdb_report.py`; diagnostic summaries use `rocksdb_diagnostics.py`; the superseded campaign audit uses `rocksdb_archive_audit.py`. To reproduce optional counters, provide a compatible Linux perf executable at `target/rocksdb/perf`, then run:

```sh
taskset -c 12-21 python3 tools/agentbench/readpath/rocksdb_profile.py --raw target/rocksdb/reproduction/raw --out target/rocksdb/reproduction/profile
```

Call-stack sampling additionally requires a host policy that allows perf record. `rocksdb_sample.py --layout bulk` or `--layout normal` implements the scoped sampling workflow, but it did not succeed on this host.
