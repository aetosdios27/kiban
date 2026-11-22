# Kiban vs tuned libmdbx, with LMDB control

Kiban's agent-memory profile had lower warm-read p50 and p99 than both libmdbx and LMDB in all four 1M-record groups. The same-round median p50 ratios were **0.47–0.60× libmdbx for recall by ID** and **0.69–0.71× for latest-ten**. The index-off row lost latest-ten to both B-trees in every group. Raw LMDB had lower read p50 than libmdbx in every group.

Kiban also had lower median durable remember and update latency on this host. Indexed Kiban's durable remember p50 was **1.016 ms**, versus **2.097 ms** for each B-tree. In the durable write suite, LMDB beat indexed Kiban on recall and latest-ten p99. Kiban's fast mutation lead comes with a weaker process-crash acknowledgment. [Protocol](PROTOCOL.md) · [Full trial ranges](MEASURE.md)

## Workload and timing boundary

The readpath driver uses seed 20261016, 64 agents, 200 ASCII text bytes per record, and the same composite key and JSON value in every engine. The ID determines the agent and immutable creation sequence. The key sorts the newest records first within each agent, so a forward prefix cursor returns latest-ten without a second index. Latest-ten revisits 64 slices. The 1M population increases the point-read working set.

Each read command runs 100k calls of each operation. Kiban bulk uses the existing bulk loader and final flush. Kiban normal uses individual ID-order puts with ordinary flush and compaction policy. The Kiban index-on row uses the `KibanOptions::agent_memory(18)` settings and prepares the prefix scan view at open. The index-off row disables both indexes and uses the sorted-range fallback of the same prefix API. LMDB and libmdbx each load in ID order in one transaction. Both B-tree controls use that single fixture across Kiban's bulk and normal groups. [Fixture layouts and effective options](fixture-layouts.json)

Each write command starts from a fresh copy of a 10k fixture. It runs 1,000 calls each of remember, recall-by-id, recent-for-agent, update, and mixed. Mixed has 900 point reads and 100 updates, for 2,100 total mutations across the five stages. Kiban's writes fit in the in-memory part of the engine before the final flush.

The per-call timer includes key construction, owned result materialization, JSON decode or encode, and any required transaction or persistence. The timer stops before result destruction, oracle checks, and write readback. Every timed result or write readback passes the full deterministic payload check. Each command verifies every record and all agents' latest-ten results before and after timing. Write commands also close, reopen, and verify the final model. The benchmark checks read layout stability. Call throughput divides count by the sum of call durations. Verified throughput divides count by the stage wall time, which includes verification and write readback. Both exclude final persistence and full validation. Histogram percentiles are upper bucket bounds.

## Versions and options

All rows use the same locked readpath release binary, including `mimalloc 0.1.48` and `serde_json 1.0.143`. Kiban's engine tree is unchanged from the study base. libmdbx uses the published `libmdbx 0.9.0` wrapper and `mdbx-sys 14.4.0` with libmdbx C 0.14.4. The existing LMDB control uses `heed3 0.22.0` and `lmdb-master3-sys 0.2.5`. The LMDB C header identifies the bundled master3 fork as 0.9.90. [Binding selection and package checksums](PROTOCOL.md) · [Frozen source and binary hashes](raw/metadata.json)

| Option | Kiban | libmdbx | LMDB |
|---|---|---|---|
| Read storage | LSM with block cache | Mapped B-tree | Mapped B-tree |
| Cache configuration | Host default or 512 MiB | OS page cache | OS page cache |
| Map geometry | Default engine settings | 1 MiB lower bound, 10 GiB upper bound, no automatic shrink | Fixed 10 GiB map |
| Read transaction | Point get or prefix-values call | One short transaction per query | One short transaction per query |
| Latest-ten | Prepared scan view or sorted-range fallback | One prefix seek and forward cursor | One prefix iterator |
| Write transaction | One put per call | One committed transaction per call | One committed transaction per call |
| Reader limit | Default engine settings | 200 requested | 200 requested |

libmdbx decodes directly from mapped values while the read transaction is open. The safe wrapper opens the default-table handle in each transaction and delegates write begin and commit to an internal thread, pinned to CPU 10 with the client. The measurements include that coordination cost. LMDB retains its named-table handle across transactions and also decodes from mapped values. The two mmap engines have no private block-cache budget. Map size measures reserved address space. Full verification faults in the mapped data and warms the Kiban cache.

## Acknowledgment guarantees

| Engine | Durable | Fast |
|---|---|---|
| Kiban | Each put returns after `Kiban::sync()` | Buffered WAL can retain acknowledged writes in userspace |
| libmdbx | `SyncMode::Durable` commits and syncs data and metadata before returning | `SyncMode::SafeNoSync` commits without requiring per-call persistence and retains the last steady transaction |
| LMDB | Default synchronous transaction commit | `NO_SYNC` and `NO_META_SYNC` defer persistence |

libmdbx SafeNoSync publishes writes through the OS's mapped pages or file I/O before acknowledgment. Process death can lose Kiban fast writes still in its userspace buffer. After system failure, SafeNoSync can fall back to a consistent steady transaction, with loss of more recent acknowledgments. Deferred page reuse can grow the file while unsynced versions accumulate. Fast-mode speed ratios compare these different contracts. The durable results compare per-mutation persistence. [Upstream environment flags](https://libmdbx.dqdkfa.ru/group__mdbx__env.html)

Disk figures use post-command logical bytes and allocated blocks times 512. Btrfs sharing and compression can change allocated bytes. The write figures include final sync and flush effects. Peak RSS comes from GNU time and covers open, warmup, verification, and write reopen. RSS includes mapped pages and process allocations.

## Reproduction

Use a fresh private `target/libmdbx-data` and output directory. The campaign used an existing static RocksDB 11.8.1 archive for the unrelated RocksDB dependency. A clean build can compile that dependency from the locked crate. The recorded Clang 15 paths select the resource directory on this Fedora host. Set those paths to your installation. [Build and test receipts](checks/receipt.json)

```sh
export CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$PWD/target/libmdbx-h2h"
export LIBCLANG_PATH=/usr/lib64/llvm15/lib
export BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/usr/lib64/llvm15/lib/clang/15.0.7
manifest=tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 nice -n 10 cargo fmt --check --manifest-path "$manifest"
taskset -c 12-21 nice -n 10 cargo test --locked --release --features remix --manifest-path "$manifest" -- --test-threads=2
taskset -c 12-21 nice -n 10 cargo clippy --locked --release --features remix --manifest-path "$manifest" --all-targets -- -D warnings
taskset -c 12-21 nice -n 10 cargo build --locked --release --features remix --bin kiban-readbench --manifest-path "$manifest"

raw="$PWD/target/libmdbx-reproduction/raw"
bench=tools/agentbench/readpath/libmdbx_bench.py
report=tools/agentbench/readpath/libmdbx_report.py
taskset -c 12-21 nice -n 10 python3 "$bench" prepare --out "$raw"
taskset -c 12-21 nice -n 10 python3 "$bench" screen --out "$raw"
taskset -c 12-21 nice -n 10 python3 "$bench" select --out "$raw"
taskset -c 12-21 nice -n 10 python3 "$report" --screen-only "$raw"
taskset -c 12-21 nice -n 10 python3 "$bench" measure --out "$raw"
taskset -c 12-21 nice -n 10 python3 "$bench" diagnostics --out "$raw" --strace /usr/bin/strace
taskset -c 12-21 nice -n 10 python3 "$report" "$raw"
```

Archive the selection before running `measure`. The audit checks receipt hashes, rotation, counts, affinity, tuning, fixtures, and verification results. It regenerates the trial CSV, paired ratios, and result tables. [Protocol](PROTOCOL.md) · [Raw receipts](raw/metadata.json) · [Audit](audit.json) · [Decisions](decisions.tsv)

## Frozen screening choice

| Configuration | Read score (µs) | Write score (µs) | Selected |
|---|---|---|---|
| baseline | 2.657 | 173.561 |  |
| growth-1m | 2.639 | 348.468 |  |
| lifo | 2.479 | 227.041 | read |
| no-readahead | 2.579 | 169.230 | write |
| page-16k | 2.705 | 213.702 |  |
| writemap | 2.531 | 206.256 |  |
| writemap-16k | 2.713 | 273.670 |  |

## 1M reads

Cells show p50 µs / p99 µs / call kops/s, each the median of five trials.

| Scenario | Operation | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |
|---|---|---|---|---|---|
| Bulk, 512 MiB | recall-by-id | 1.824 / 2.688 / 549.338 | 0.832 / 1.216 / 1191.038 | 1.760 / 2.496 / 563.993 | 1.408 / 1.920 / 714.544 |
| Bulk, 512 MiB | recent-for-agent | 3.328 / 6.528 / 282.591 | 2.176 / 3.392 / 456.918 | 3.136 / 4.736 / 315.098 | 2.688 / 4.032 / 368.712 |
| Bulk, default | recall-by-id | 2.016 / 3.008 / 492.900 | 0.944 / 1.280 / 1081.110 | 2.016 / 3.072 / 489.865 | 1.536 / 2.688 / 639.086 |
| Bulk, default | recent-for-agent | 3.584 / 7.168 / 267.976 | 2.304 / 4.736 / 420.222 | 3.264 / 6.656 / 296.025 | 2.816 / 5.760 / 344.537 |
| Normal, 512 MiB | recall-by-id | 2.112 / 3.136 / 476.282 | 1.184 / 1.568 / 858.922 | 1.984 / 2.688 / 503.409 | 1.536 / 2.016 / 652.190 |
| Normal, 512 MiB | recent-for-agent | 4.032 / 6.400 / 246.036 | 2.240 / 3.520 / 447.684 | 3.264 / 4.736 / 307.707 | 2.816 / 4.352 / 350.355 |
| Normal, default | recall-by-id | 2.048 / 3.200 / 483.266 | 1.184 / 1.568 / 850.408 | 2.016 / 2.752 / 493.380 | 1.536 / 2.048 / 650.293 |
| Normal, default | recent-for-agent | 3.840 / 5.888 / 255.626 | 2.176 / 3.328 / 447.697 | 3.200 / 4.736 / 311.122 | 2.688 / 4.032 / 365.837 |

## 10k writes

Cells show p50 µs / p99 µs / call kops/s, each the median of five trials.

| Scenario | Operation | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |
|---|---|---|---|---|---|
| Fast | remember | 0.784 / 7.808 / 888.623 | 0.784 / 6.784 / 909.652 | 18.944 / 51.200 / 20.785 | 8.448 / 16.896 / 113.202 |
| Fast | recall-by-id | 1.088 / 2.304 / 871.425 | 0.656 / 1.120 / 1514.108 | 1.280 / 2.048 / 779.188 | 0.960 / 1.536 / 1040.282 |
| Fast | recent-for-agent | 3.584 / 7.296 / 263.147 | 2.816 / 5.760 / 332.299 | 3.456 / 5.120 / 285.203 | 3.008 / 6.016 / 331.401 |
| Fast | update | 0.896 / 7.424 / 722.125 | 0.912 / 8.448 / 783.266 | 23.040 / 67.584 / 5.709 | 8.448 / 15.616 / 116.706 |
| Fast | mixed-90-10 | 1.248 / 2.752 / 766.210 | 0.928 / 2.048 / 1009.854 | 1.504 / 53.248 / 161.528 | 1.024 / 9.984 / 557.378 |
| Durable | remember | 1081.344 / 6422.528 / 0.772 | 1015.808 / 2555.904 / 0.971 | 2097.152 / 9175.040 / 0.435 | 2097.152 / 9437.184 / 0.397 |
| Durable | recall-by-id | 1.120 / 2.496 / 859.578 | 0.768 / 1.952 / 1206.947 | 1.216 / 2.240 / 835.051 | 0.896 / 1.632 / 1116.716 |
| Durable | recent-for-agent | 3.072 / 6.016 / 307.700 | 2.496 / 5.504 / 381.550 | 3.072 / 6.016 / 326.536 | 2.560 / 3.904 / 380.843 |
| Durable | update | 1032.192 / 3997.696 / 0.905 | 1032.192 / 6029.312 / 0.838 | 2162.688 / 10747.904 / 0.389 | 2228.224 / 13369.344 / 0.360 |
| Durable | mixed-90-10 | 2.560 / 1146.880 / 9.333 | 1.664 / 1081.344 / 11.255 | 2.176 / 2686.976 / 4.094 | 1.856 / 4718.592 / 2.969 |

## Paired latency ratios

Each cell divides Kiban agent-memory latency by the same-round comparator and takes the median across five rounds. Values below 1 favor Kiban. Detailed tables include the index-off row, mean and throughput ratios, and per-metric ranges. [Full primary tables](MEASURE.md)

| Scenario | Operation | libmdbx p50 / p99 | LMDB p50 / p99 |
|---|---|---|---|
| Bulk, 512 MiB | recall-by-id | 0.473 / 0.475 | 0.591 / 0.617 |
| Bulk, 512 MiB | recent-for-agent | 0.688 / 0.703 | 0.810 / 0.823 |
| Bulk, default | recall-by-id | 0.468 / 0.427 | 0.615 / 0.488 |
| Bulk, default | recent-for-agent | 0.708 / 0.712 | 0.783 / 0.740 |
| Normal, 512 MiB | recall-by-id | 0.597 / 0.583 | 0.771 / 0.773 |
| Normal, 512 MiB | recent-for-agent | 0.686 / 0.743 | 0.795 / 0.809 |
| Normal, default | recall-by-id | 0.597 / 0.571 | 0.771 / 0.758 |
| Normal, default | recent-for-agent | 0.694 / 0.703 | 0.810 / 0.803 |
| Fast | remember | 0.043 / 0.133 | 0.093 / 0.402 |
| Fast | update | 0.040 / 0.144 | 0.112 / 0.463 |
| Fast | mixed-90-10 | 0.617 / 0.040 | 0.879 / 0.172 |
| Durable | remember | 0.492 / 0.264 | 0.475 / 0.392 |
| Durable | update | 0.485 / 0.523 | 0.441 / 0.500 |
| Durable | mixed-90-10 | 0.711 / 0.402 | 0.885 / 0.306 |

## Disk and memory

Cells show logical MiB / allocated MiB / whole-process peak RSS MiB, medians across five trials. [Resource ranges](MEASURE.md)

| Scenario | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |
|---|---|---|---|---|
| Bulk, 512 MiB | 314.6 / 314.6 / 458.5 | 351.9 / 351.9 / 408.0 | 688.0 / 688.0 / 724.3 | 799.3 / 799.3 / 839.6 |
| Bulk, default | 314.6 / 314.6 / 457.8 | 351.9 / 351.9 / 407.9 | 688.0 / 688.0 / 724.4 | 799.3 / 799.3 / 839.3 |
| Normal, 512 MiB | 314.6 / 314.7 / 460.7 | 352.4 / 352.7 / 450.9 | 688.0 / 688.0 / 724.4 | 799.3 / 799.3 / 839.5 |
| Normal, default | 314.6 / 314.7 / 460.7 | 352.4 / 352.7 / 451.0 | 688.0 / 688.0 / 724.4 | 799.3 / 799.3 / 839.5 |
| Fast | 3.7 / 3.8 / 25.0 | 4.4 / 4.4 / 31.0 | 16.0 / 16.0 / 32.8 | 8.6 / 8.6 / 23.2 |
| Durable | 3.7 / 3.8 / 24.8 | 4.4 / 4.4 / 30.6 | 16.0 / 16.0 / 24.4 | 8.6 / 8.6 / 23.3 |

## What the screen selected

The read winner uses 4 KiB pages, 16 MiB growth, LIFORECLAIM, and OS readahead. The write winner also uses 4 KiB pages and 16 MiB growth, with NORDAHEAD enabled and LIFORECLAIM disabled. Both use file I/O for writes. The read score was 2.479 µs versus 2.531 µs for the next candidate. The write score was 169.230 µs versus 173.561 µs for the baseline. These scores aggregate workloads on a multiplicative scale. The narrow margins and overlapping trial ranges leave the effect of individual flags uncertain.

The NORDAHEAD read screen included a 60.416 µs recall-by-ID p50 and 341,167 whole-command major faults in round three. Its round-two command had zero major faults. The screen retained both. The 16 KiB WRITEMAP fixture used 650 MiB allocated at 1M records, versus 688 MiB for the read winner. Its read and write scores were higher. [Screen timings and resources](SCREEN.md) · [Frozen selection](raw/selection.json) · [NORDAHEAD round three](raw/screen/read-1000000-single-mmap/r3-no-readahead.time.txt)

## Durability audit and completion

After primary measurement, each diagnostic performed 16 durable inserts and 16 durable updates on a fresh fixture copy. The supervisor released one mutation at a time. The trace places synchronous data-file I/O before each acknowledgment. After the final acknowledgment, the supervisor sent SIGKILL before close or final sync. Reopen verified 10,016 complete records and every agent's latest-ten results in each case.

| Diagnostic | Acknowledgments with synchronous I/O | Observed mechanisms |
|---|---:|---|
| Write winner, NORDAHEAD | 32 of 32 | `fdatasync` and write-through I/O |
| Baseline | 32 of 32 | `fdatasync` and write-through I/O |
| WRITEMAP | 32 of 32 | `msync(MS_SYNC)` |

Write-through I/O here means a successful write to an `O_DSYNC` data-file descriptor. The trace and the per-ack audit cover both native paths. Process death leaves the OS cache intact. Physical power-loss behavior depends on the storage stack honoring the synchronous I/O contract. [Write-winner trace](checks/durable-no-readahead.strace) · [Baseline trace](checks/durable-baseline.strace) · [WRITEMAP trace](checks/durable-writemap.strace)

Indexed Kiban handled 971 durable remember calls/s and 838 durable updates/s at the median. libmdbx handled 435 and 389, and LMDB handled 397 and 360. Durable p99 varied substantially. Indexed Kiban update p99 spanned 2.10–9.44 ms versus 4.13–12.58 ms for libmdbx. Within Kiban, the index-off row had a lower median durable update p99, 4.00 ms versus 6.03 ms. Mixed p50 mostly reflects reads. Mixed throughput also includes all 100 durable writes.

The final sync, flush where applicable, and full verification together took a median of 27.6 ms for indexed Kiban fast and 40.3 ms for libmdbx fast. Their durable equivalents took 25.5 and 15.5 ms. The close and reopen check follows that interval. [Raw write commands](raw/measure/write-10000-durable/commands.jsonl)

## What the results cover

The Kiban profile combines point and prefix scan acceleration. Bulk had one L0 table per Kiban configuration. Normal had one L1 and six L2 tables in both configurations, with 77 L3 tables off and 82 on. The results compare complete configurations with their resulting layouts. On 1M records, the profile adds roughly 37–38 MiB to Kiban's logical disk size. Both Kiban configurations used less disk and peak RSS than either B-tree in the primary 1M groups. At 10k records, LMDB used the least peak RSS in both write modes. libmdbx occupied less disk than LMDB at 1M records and more at 10k. The ID-order loaders and the selected growth geometry affect these sizes.

LMDB's same-round p50 advantage over libmdbx in the 1M groups was about 1.28–1.30× for point reads and 1.14–1.17× for latest-ten, expressed as libmdbx latency divided by LMDB latency. libmdbx's safe binding includes transaction-scoped table opens and writer thread coordination. This campaign measures the complete Rust adapters. It does not isolate native engine costs or cover custom FFI, sorted B-tree loading, batched transactions, cold reads, long scans, concurrent clients, or sustained writes.

## Host activity and verification

All benchmark processes were pinned to CPU 10. CPU 11 had background activity during primary command windows, with median busy time **4.67%** and a range of **0–22.22%**. The one-minute host load ranged from **3.28 to 5.50**. These windows include setup and verification. Short windows have coarse tick resolution. The host used Btrfs with zstd compression, turbo was enabled, and other work shared memory and storage. All four engines rotated each round, with the first order repeated in round five. Rotation and same-round controls limit order effects under these shared-host conditions. [Host details](host.json) · [Timing audit](audit.json)

The audit accepted 120 screen commands and 120 primary commands, covering 8.40 million and 16.20 million timed calls with zero verification errors. The host default Kiban cache was 448,987,008 bytes (428.19 MiB). The engine source matches the study base `2dee66cb`, with engine revision `a9906ad1`. The release build, 15 readpath tests, both Clippy gates, and formatting passed before preparation. [Check receipts](checks/receipt.json) · [Trial data](TRIALS.csv) · [Metric summaries](summary.json)
