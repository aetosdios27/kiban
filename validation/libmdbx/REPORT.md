# Kiban vs tuned libmdbx, with LMDB control

The warm-read page-fault gate accepted 19 of 20 scheduled rounds. Bulk/512 MiB round 1 reached the ten-attempt cap. All accepted commands recorded zero major faults. Kiban's agent-memory p50 was lower than both controls for both operations in all three groups with five qualifying rounds. The paired Kiban-to-libmdbx ratios were **0.49–0.62× for recall** and **0.74–0.75× for latest-ten**.

Durable remember median p50 was **0.541 ms** for indexed Kiban, **1.147 ms** for libmdbx, and **1.245 ms** for LMDB. Fast remember p50 was **0.656 µs**, **9.728 µs**, and **6.784 µs**, respectively. Kiban fast can acknowledge while its WAL is still buffered in userspace. [Protocol](PROTOCOL.md) · [Full trial ranges](MEASURE.md)

## Workload and timing boundary

The readpath driver uses seed 20261016, 64 agents, 200 ASCII text bytes per record, and the same composite key and JSON value in every engine. The ID determines the agent and immutable creation sequence. The key sorts the newest records first within each agent, so a forward prefix cursor returns latest-ten without a second index. Latest-ten revisits 64 slices. The 1M population increases the point-read working set.

Each read command runs 100k calls of each operation. Kiban bulk uses the existing bulk loader and final flush. Kiban normal uses individual ID-order puts with ordinary flush and compaction policy. The Kiban index-on row uses the `KibanOptions::agent_memory(18)` settings and prepares the prefix scan view at open. The index-off row disables both indexes and uses the sorted-range fallback of the same prefix API. LMDB and libmdbx each load in ID order in one transaction. Both B-tree controls use that single fixture across Kiban's bulk and normal groups. [Fixture layouts and effective options](fixture-layouts.json)

Each write command starts from a fresh copy of a 10k fixture. It runs 1,000 calls each of remember, recall-by-id, recent-for-agent, update, and mixed. Mixed has 900 point reads and 100 updates, for 2,100 total mutations across the five stages. Kiban's writes fit in the in-memory part of the engine before the final flush.

The per-call timer includes key construction, owned result materialization, JSON decode or encode, and any required transaction or persistence. The timer stops before result destruction, oracle checks, and write readback. Every timed result or write readback passes the full deterministic payload check. Each command verifies every record and all agents' latest-ten results before and after timing. Write commands also close, reopen, and verify the final model. The benchmark checks read layout stability. Call throughput divides count by the sum of call durations. Verified throughput divides count by the stage wall time, which includes verification and write readback. Both exclude final persistence and full validation. Histogram percentiles are upper bucket bounds.

## Versions and options

All rows use the same locked readpath release binary, including `mimalloc 0.1.48` and `serde_json 1.0.143`. Kiban's engine tree is unchanged from the study base. libmdbx uses direct `mdbx-sys 14.4.0` calls with libmdbx C 0.14.4. The existing LMDB control uses `heed3 0.22.0` and `lmdb-master3-sys 0.2.5`. The LMDB C header identifies the bundled master3 fork as 0.9.90. [Adapter change and package checksums](PROTOCOL.md#deviation-1-adapter-fairness-2026-11-22-utc) · [Frozen source and binary hashes](raw/metadata.json)

| Option | Kiban | libmdbx | LMDB |
|---|---|---|---|
| Read storage | LSM with block cache | Mapped B-tree | Mapped B-tree |
| Cache configuration | Host default or 512 MiB | OS page cache | OS page cache |
| Map geometry | Default engine settings | 1 MiB lower bound, 10 GiB upper bound, no automatic shrink | Fixed 10 GiB map |
| Read transaction | Point get or prefix-values call | One short transaction per query | One short transaction per query |
| Latest-ten | Prepared scan view or sorted-range fallback | One prefix seek and forward cursor | One prefix iterator |
| Write transaction | One put per call | One committed transaction per call | One committed transaction per call |
| Reader limit | Default engine settings | 200 requested | 200 requested |

libmdbx decodes directly from mapped values while the read transaction is open. The adapter opens the default-table handle once per environment and calls native begin and commit on the benchmark thread. LMDB retains its named-table handle across transactions and also decodes from mapped values. The two mmap engines have no private block-cache budget. Map size measures reserved address space. Full verification faults in the mapped data and warms the Kiban cache.

## Acknowledgment guarantees

| Engine | Durable | Fast |
|---|---|---|
| Kiban | Each put returns after `Kiban::sync()` | Buffered WAL can retain acknowledged writes in userspace |
| libmdbx | `MDBX_SYNC_DURABLE` commits and syncs data and metadata before returning | `MDBX_SAFE_NOSYNC` commits without requiring per-call persistence and retains the last steady transaction |
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
```

The commands above reproduce the adapter-fairness campaign at commit `460f9829`. For the gated rerun, use its frozen selection and a byte-identical release binary in `target/libmdbx-h2h/release/kiban-readbench`. Run the following from a checkout with the frozen evidence, including `superseded-read-faults/` beside `$raw`, and a fresh `measure-reads-quiet` output directory under `$raw`.

```sh
raw="$PWD/validation/libmdbx/raw"
reads=tools/agentbench/readpath/libmdbx_reads.py
report=tools/agentbench/readpath/libmdbx_report.py
LC_ALL=C taskset -c 12-21 nice -n 10 python3 "$reads" prepare --raw "$raw"
LC_ALL=C taskset -c 12-21 nice -n 10 python3 "$reads" measure --raw "$raw" --resume
taskset -c 12-21 nice -n 10 python3 "$report" "$raw"
```

The read driver returns a nonzero status if a round exhausts its ten attempts. Run the audit after the remaining rounds finish.

The audit checks receipt hashes, rotation, counts, affinity, tuning, fixtures, verification, memory samples, and gate decisions. It regenerates the trial CSV, paired ratios, and result tables. [Protocol](PROTOCOL.md) · [Raw receipts](raw/metadata.json) · [Audit](audit.json) · [Decisions](decisions.tsv)

## Frozen screening choice

| Configuration | Read score (µs) | Write score (µs) | Selected |
|---|---|---|---|
| baseline | 2.454 | 209.996 |  |
| growth-1m | 2.332 | 403.527 |  |
| lifo | 2.399 | 223.345 |  |
| no-readahead | 2.322 | 154.275 | read, write |
| page-16k | 2.356 | 195.640 |  |
| writemap | 2.344 | 221.745 |  |
| writemap-16k | 2.400 | 340.273 |  |

## 1M reads

Cells show p50 µs / p99 µs / call kops/s. Bulk/512 MiB has descriptive medians from four qualifying rounds because round 1 reached the ten-attempt cap. The other rows use five qualifying rounds. [Accepted trials](TRIALS.csv) · [Discarded attempts](MEASURE.md#read-page-fault-gate)

| Scenario | Operation | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |
|---|---|---|---|---|---|
| Bulk, 512 MiB (4 rounds) | recall-by-id | 1.696 / 2.624 / 588.808 | 0.840 / 1.248 / 1198.239 | 1.712 / 2.752 / 570.935 | 1.440 / 2.016 / 688.249 |
| Bulk, 512 MiB (4 rounds) | recent-for-agent | 3.776 / 5.824 / 264.477 | 2.368 / 3.616 / 418.291 | 3.296 / 4.864 / 303.303 | 3.072 / 4.544 / 324.449 |
| Bulk, default | recall-by-id | 1.664 / 2.816 / 593.550 | 0.800 / 1.216 / 1227.769 | 1.696 / 2.944 / 581.554 | 1.376 / 2.368 / 717.658 |
| Bulk, default | recent-for-agent | 3.392 / 7.552 / 277.593 | 2.176 / 4.480 / 443.652 | 2.944 / 5.888 / 322.210 | 2.752 / 6.528 / 345.682 |
| Normal, 512 MiB | recall-by-id | 1.632 / 2.752 / 604.654 | 0.960 / 1.376 / 1018.781 | 1.568 / 2.688 / 630.992 | 1.280 / 1.856 / 780.931 |
| Normal, 512 MiB | recent-for-agent | 3.392 / 5.760 / 286.242 | 1.920 / 3.200 / 505.178 | 2.560 / 4.032 / 386.980 | 2.432 / 4.032 / 409.580 |
| Normal, default | recall-by-id | 1.632 / 2.624 / 605.256 | 0.976 / 1.312 / 1021.682 | 1.536 / 2.368 / 650.808 | 1.280 / 1.792 / 774.709 |
| Normal, default | recent-for-agent | 3.392 / 5.632 / 289.515 | 1.856 / 3.072 / 522.281 | 2.560 / 4.224 / 386.862 | 2.432 / 3.968 / 411.153 |

## 10k writes

Cells show p50 µs / p99 µs / call kops/s, each the median of five trials.

| Scenario | Operation | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |
|---|---|---|---|---|---|
| Fast | remember | 0.656 / 5.504 / 1089.889 | 0.656 / 5.504 / 1121.694 | 9.728 / 19.968 / 18.710 | 6.784 / 11.776 / 138.091 |
| Fast | recall-by-id | 0.848 / 1.472 / 1169.339 | 0.456 / 0.720 / 2113.624 | 0.992 / 1.664 / 1008.048 | 0.848 / 1.280 / 1187.371 |
| Fast | recent-for-agent | 3.136 / 4.992 / 308.534 | 2.496 / 3.392 / 401.775 | 2.496 / 3.456 / 394.617 | 2.688 / 3.648 / 367.399 |
| Fast | update | 0.768 / 5.888 / 832.545 | 0.800 / 6.272 / 934.023 | 13.312 / 23.040 / 5.083 | 7.552 / 11.264 / 129.732 |
| Fast | mixed-90-10 | 0.976 / 2.176 / 977.509 | 0.672 / 1.312 / 1408.246 | 1.216 / 17.920 / 387.219 | 0.928 / 8.192 / 624.766 |
| Durable | remember | 557.056 / 1671.168 / 1.723 | 540.672 / 1769.472 / 1.551 | 1146.880 / 2818.048 / 0.825 | 1245.184 / 2949.120 / 0.725 |
| Durable | recall-by-id | 0.976 / 2.304 / 972.781 | 0.704 / 1.440 / 1380.377 | 1.056 / 2.240 / 934.566 | 0.848 / 1.536 / 1196.781 |
| Durable | recent-for-agent | 3.136 / 5.248 / 307.096 | 2.240 / 3.584 / 437.771 | 2.624 / 4.032 / 374.975 | 2.496 / 3.328 / 400.102 |
| Durable | update | 557.056 / 1769.472 / 1.579 | 557.056 / 3670.016 / 1.394 | 1179.648 / 3211.264 / 0.806 | 1245.184 / 12058.624 / 0.439 |
| Durable | mixed-90-10 | 2.112 / 655.360 / 16.235 | 1.312 / 688.128 / 16.512 | 1.920 / 1343.488 / 8.204 | 1.472 / 1474.560 / 7.822 |

## Paired latency ratios

Each cell divides Kiban agent-memory latency by the same-round comparator and takes the median across five qualifying rounds. Values below 1 favor Kiban. Across the four qualifying bulk/512 MiB rounds, descriptive paired p50 ratios against libmdbx and LMDB were 0.490 and 0.584 for recall and 0.723 and 0.781 for latest-ten. Detailed tables include the index-off row, mean and throughput ratios, and per-metric ranges. [Full primary tables](MEASURE.md)

| Scenario | Operation | libmdbx p50 / p99 | LMDB p50 / p99 |
|---|---|---|---|
| Bulk, default | recall-by-id | 0.490 / 0.433 | 0.581 / 0.514 |
| Bulk, default | recent-for-agent | 0.739 / 0.763 | 0.810 / 0.745 |
| Normal, 512 MiB | recall-by-id | 0.622 / 0.581 | 0.750 / 0.768 |
| Normal, 512 MiB | recent-for-agent | 0.750 / 0.794 | 0.789 / 0.790 |
| Normal, default | recall-by-id | 0.625 / 0.554 | 0.762 / 0.732 |
| Normal, default | recent-for-agent | 0.738 / 0.727 | 0.784 / 0.774 |
| Fast | remember | 0.067 / 0.263 | 0.094 / 0.457 |
| Fast | update | 0.059 / 0.276 | 0.105 / 0.598 |
| Fast | mixed-90-10 | 0.553 / 0.072 | 0.712 / 0.155 |
| Durable | remember | 0.463 / 0.573 | 0.423 / 0.522 |
| Durable | update | 0.447 / 0.602 | 0.429 / 0.347 |
| Durable | mixed-90-10 | 0.641 / 0.427 | 0.880 / 0.390 |

## Disk and memory

Cells show logical MiB / allocated MiB / whole-process peak RSS MiB, medians across five trials. [Resource ranges](MEASURE.md)

| Scenario | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |
|---|---|---|---|---|
| Bulk, default | 314.6 / 314.6 / 458.5 | 351.9 / 351.9 / 408.1 | 688.0 / 688.0 / 722.3 | 799.3 / 799.3 / 839.4 |
| Normal, 512 MiB | 314.6 / 314.7 / 460.6 | 352.4 / 352.7 / 450.8 | 688.0 / 688.0 / 722.2 | 799.3 / 799.3 / 839.4 |
| Normal, default | 314.6 / 314.7 / 460.6 | 352.4 / 352.7 / 450.8 | 688.0 / 688.0 / 722.3 | 799.3 / 799.3 / 839.4 |
| Fast | 3.7 / 3.8 / 24.9 | 4.4 / 4.4 / 31.0 | 16.0 / 16.0 / 30.5 | 8.6 / 8.6 / 23.3 |
| Durable | 3.7 / 3.8 / 24.8 | 4.4 / 4.4 / 30.7 | 16.0 / 16.0 / 22.1 | 8.6 / 8.6 / 23.4 |


## What the screen selected

`no-readahead` won both screens with 4 KiB pages, 16 MiB growth, file I/O, and NORDAHEAD. Its read score was 2.322 µs, 0.010 µs ahead of `growth-1m`. Its write score was 154.275 µs, ahead of `page-16k` at 195.640 µs. The scores combine workloads on a multiplicative scale. [Screen timings and resources](SCREEN.md) · [Frozen selection](raw/selection.json)

Round five of the NORDAHEAD read screen reached a 94.208 µs recall p50 and 500,729 whole-command major faults. Its first four rounds recorded zero major faults. The frozen rule includes all five rounds and uses their median mean-call times. [Round five timing and faults](raw/screen/read-1000000-single-mmap/r5-no-readahead.time.txt)

Libmdbx remember p50 fell from 18.944 to 9.728 µs in fast mode (49%) and from 2.097 to 1.147 ms in durable mode (45%). Fast update p50 fell from 23.040 to 13.312 µs (42%). With `no-readahead` fixed in both screens, fast remember p50 fell from 17.920 to 9.728 µs. The campaigns ran at different times on the shared host and selected different read settings. [Original report](superseded-binding-handoff/REPORT.md) · [Original trial summary](superseded-binding-handoff/summary.json)

Fast mutation call throughput remained sensitive to infrequent stalls. Median libmdbx remember throughput fell from 20.8 to 18.7 kcalls/s, and fast update fell from 5.7 to 5.1 kcalls/s. The slowest observed fast update in the rerun took 3.66 s. [Fast write trial ranges](MEASURE.md#write-10000-fast) · [Round five raw data](raw/measure/write-10000-fast/r5-libmdbx-tuned.jsonl)

## Warm-read page-fault gate

The adapter-fairness primary recorded **2,177,499** libmdbx major faults across 12 of 20 read commands, compared with 78 for LMDB and 7 for indexed Kiban. Host memory pressure evicted mapped libmdbx pages. The director’s quieter spot check recorded zero major faults and 1.632 µs recall p50. Across the five bulk/default rounds, libmdbx recall p50 moved from 2.368 to 1.696 µs, p99 from 167.936 to 2.944 µs, and call throughput from 62.625 to 581.554 kops/s after the gated rerun. GNU time measured faults over the whole command, including verification and setup. The gate discarded **11** whole rounds and kept every receipt. [Deviation 2](PROTOCOL.md#deviation-2-read-primary-rerun-with-a-page-fault-gate-2026-11-22-utc) · [Superseded reads](superseded-read-faults/) · [Discarded attempts](MEASURE.md#read-page-fault-gate)

## Durability audit and completion

Each diagnostic performed 16 durable inserts and 16 durable updates on a fresh fixture copy. The supervisor released one mutation at a time and sampled `/proc/<pid>/task` at readiness and after the last acknowledgment. Both samples showed one thread. The syscall trace recorded one TID and no thread creation, with that TID performing both synchronous I/O and the acknowledgment writes. The supervisor sent SIGKILL after the last acknowledgment. Each reopen verified the full 10,016-record payload model and all 64 agents’ latest-ten results.

| Diagnostic | Writer threads | Acknowledgments with synchronous I/O | Mechanisms and acknowledgment counts |
|---|---:|---:|---|
| baseline | 1 | 32 of 32 | `fdatasync` 32, `write-through` 32 |
| writemap | 1 | 32 of 32 | `msync(MS_SYNC)` 32 |
| no-readahead (write winner) | 1 | 32 of 32 | `fdatasync` 32, `write-through` 32 |

Write-through I/O here means a successful write to an `O_DSYNC` data-file descriptor. The original file-I/O traces showed a separate TID performing `fdatasync`. Process death leaves the OS cache intact. Physical power-loss behavior depends on the storage stack honoring its synchronous I/O contract. [Baseline task count](checks/durable-baseline.json) · [Baseline trace](checks/durable-baseline.strace) · [WRITEMAP trace](checks/durable-writemap.strace) · [Original file-I/O trace](superseded-binding-handoff/checks/durable-baseline.strace)

Median durable remember and update rates were 1,551 and 1,394 calls/s for indexed Kiban, 825 and 806 for libmdbx, and 725 and 439 for LMDB. Mixed p50 mostly reflects reads. Mixed throughput includes all 100 durable writes. [Write trial ranges](MEASURE.md#write-10000-durable)

## What the results cover

The ID-order loaders and growth geometry influence the B-tree sizes. [Resource ranges](MEASURE.md) · [Fixture layouts](fixture-layouts.json)

Across the three complete 1M groups, the same-round libmdbx-to-LMDB median p50 ratios were 1.20–1.23× for point recall and 1.05–1.11× for latest-ten. Durable update p99 ranged from 1.25–4.46 ms for indexed Kiban, 2.36–6.42 ms for libmdbx, and 2.23–12.85 ms for LMDB. The campaign covers a single client in warm, verified fixtures and small fresh write workloads. It leaves open performance for cold reads, long scans, concurrent clients, batched commits, and sustained writes. [Primary trial ranges](MEASURE.md)

## Host activity and verification

Benchmark children ran on CPU 10. During accepted read commands, CPU 11 background activity had a median of 0.34% and ranged from 0.00% to 4.10%. One-minute host load ranged from 1.25 to 2.50. Host MemAvailable at the command boundaries ranged from 22.13 to 22.81 GiB. Each read receipt includes both MemAvailable and `/proc/pressure/memory` samples before and after the command. The host used Btrfs with zstd compression and turbo was enabled. [Read host snapshot](raw/measure-reads-quiet/host.json) · [Read receipts](raw/measure-reads-quiet/) · [Timing audit](audit.json)

The audit verified six rebuilt 1M-record fixtures and all 76 accepted read commands, covering 15,200,000 timed calls with zero verification errors. It also verified every discarded attempt, all 120 frozen screen commands, and all 40 original write commands. The Kiban default cache was 448,987,008 bytes (428.19 MiB), and the engine source revision was `a9906ad1`. The rerun used the original release binary. All 15 readpath tests, formatting, and both Clippy gates passed before preparation. [Tests](checks/quiet-test.log) · [Build receipt](checks/receipt.json) · [Trial data](TRIALS.csv) · [Metric summaries](summary.json)
