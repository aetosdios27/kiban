# Kiban vs tuned Fjall, with LMDB control

Indexed Kiban had **1.77–2.31× lower recall-by-ID p50** and **2.67–3.33× lower latest-ten p50** than tuned Fjall across four warm 1M-record scenarios. These factors use median same-round latency ratios. Fjall had lower median durable remember and update latency, with overlapping trial ranges. Fjall LZ4 used the least disk space for both read layouts.

| Default-cache workload | Operation | Kiban off | Kiban on | Fjall tuned | Fjall LZ4 | LMDB |
| --- | --- | --- | --- | --- | --- | --- |
| 1M normal | Recall-by-ID | 1.632 | 0.944 | 1.760 | 1.824 | 1.248 |
| 1M normal | Latest-ten | 3.008 | 1.728 | 5.760 | 6.912 | 2.112 |
| 10k fast | Remember | 0.576 | 0.576 | 0.672 | 0.672 | 5.888 |
| 10k durable | Remember | 573.440 | 557.056 | 507.904 | 516.096 | 1,114.112 |
| 10k durable | Update | 573.440 | 557.056 | 507.904 | 540.672 | 1,212.416 |

Cells give median p50 in microseconds across five trials. The full results include all stages, p99, call and verified throughput, paired ratios, and ranges. [Primary results](RESULTS.md) · [Per-trial CSV](TRIALS.csv)

## Workload and timing boundary

The fixture contains 64 agents. Each value encodes the identity, creation sequence, revision and 200 ASCII text bytes as JSON, with seed 20261016. An ID determines its agent and creation sequence. The composite key orders each agent's values by newest first. Fjall uses an ascending prefix iterator and decodes the value from each guard. The latest-ten workload revisits 64 distinct slices, even with a 1M-record fixture.

Each read command performs 100k recall-by-ID calls and 100k latest-ten calls. Kiban bulk-preloads records in batches and flushes them. Fjall bulk uses sorted ingestion. Normal fixtures use individual ID-order puts and retain the engines' natural compaction layouts. LMDB uses its same single-transaction, ID-order loader in both layout groups. Each write command copies a 10k bulk fixture and runs 1k calls in each of five stages. Mixed has 900 reads and 100 updates. In total, each write command performs 2,100 mutations.

The timer includes key construction, JSON encoding or decoding, owned results and any requested durability. The deterministic input and returned Memory destruction are outside the timer. Each timed read and write readback is checked against the oracle. Full point and all-agent verification runs before and after measurement, and every write command reopens and checks the final revision model. Call throughput divides by accumulated call time. Verified throughput uses stage wall time, including oracle and write readback costs. Both exclude open, full-model verification and final persistence. Histogram percentiles are upper bucket bounds.

## Versions and options

| Engine | Release and options |
|---|---|
| Kiban | 0.2.0, engine source at `a9906ad1`. The index-on row uses `KibanOptions::agent_memory(18)`. The index-off row disables the point and prefix scan indexes. Both call `prefix_values`. Neither sets mmap or binary-fuse filtering. |
| Fjall | 3.1.13 with lsm-tree 3.1.10. One `memories` keyspace, one background worker and default leveled compaction. The same configuration won for reads and writes. It uses 4 KiB data blocks, a 10-bit Bloom policy where filters are built, a 64 MiB memtable, hash ratio 0.75, all-level index and filter pinning, the point-hit hint, and data restart interval 8. The LZ4 companion adds lz4_flex 0.13.1 data-block compression to that configuration. Fjall has no vendored C engine or compression library. |
| LMDB | heed3 0.22.0, lmdb-master3-sys 0.2.5. The bundled master3 header reports 0.9.90, dated May 1, 2017. The environment retains a 10 GiB virtual map, max_dbs=200 and max_readers=200. |

The two LSMs receive the same 536,870,912-byte or 448,987,008-byte block-cache capacity. The latter is Kiban's default on this host and is also used for writes. Index/filter residency and OS page cache are separate from the configured capacity. LMDB relies on OS-managed mmap pages. All engines use the same release process configuration with Rust MiMalloc 0.1.48 and serde_json 1.0.143. The binary also links the existing static RocksDB 11.8.1 archive for harness compatibility. No measured row opens RocksDB. Exact source commits and crate checksums are in the protocol. The compiler, archive digest and host configuration are in the host record. [Protocol](PROTOCOL.md) · [Host](host.json) · [Binary and source hashes](raw/screen-supplement/metadata.json)

## Durability boundary

| Engine | Durable acknowledgment | Fast acknowledgment |
|---|---|---|
| Kiban | Put followed by `Kiban::sync()`, which flushes and fdatasyncs the WAL | Buffered WAL and memtable put |
| Fjall | Insert followed by `Database::persist(SyncData)`, which flushes and fdatasyncs the journal | Insert into the memtable and manually persisted journal, with no explicit persist at acknowledgment |
| LMDB | One synchronous committed transaction per mutation | One transaction per mutation with `NO_SYNC` and `NO_META_SYNC` |

Both LSM fast modes can acknowledge data still in userspace. Their buffering and background flush policies can differ. LMDB fast also relaxes per-write durability. The final sync sits outside the operation timer. The sync probe checks successful calls over the full command, including setup and finalization. The SIGKILL check tests survival on a running OS. Machine power-loss behavior remains outside this experiment.

| Engine | Fast fsync / fdatasync | Durable fsync / fdatasync |
| --- | --- | --- |
| Kiban index on | 9 / 2 | 9 / 2,102 |
| Fjall tuned | 12 / 1 | 12 / 2,101 |
| LMDB | 0 / 1 | 0 / 2,101 |

Each durable command added exactly 2,100 successful fdatasync calls over its fast counterpart. The libc preload probe counted syncs in the harness and its native libraries. Fjall's journal calls `File::sync_data` before returning from `SyncData`. After 25 durable acknowledgments, SIGKILL terminated Fjall. A fresh process reopened and verified all 10,025 records. [Sync counts](checks/sync.json) · [SIGKILL receipt](checks/crash.json)

## Supplemental screen

`g-hash-pin-hit-restart` won both screens. Its read score was 11.24% below the contemporaneous `block4` score, and its write score was 1.67% lower. For each workload and scenario, take the median of five trial mean call times. The score is the geometric mean of those medians. Lower is better.

| Candidate | Read score µs | Write score µs |
| --- | --- | --- |
| a-hash | 3.483 | 16.932 |
| b-pin | 3.378 | 16.723 |
| block4 | 3.522 | 16.137 |
| c-hit | 3.259 | 16.642 |
| d-restart | 3.580 | 17.039 |
| e-hash-pin | 3.560 | 15.896 |
| f-hash-pin-hit | 3.376 | 16.966 |
| g-hash-pin-hit-restart | 3.126 | 15.867 |

The new candidate lowered normal-layout latest-ten p50 from 7.040 to 5.632 µs at 512 MiB and from 6.912 to 5.760 µs at the default budget. The five-trial ranges were disjoint in both normal-layout scenarios. Bulk latest-ten p50 matched the baseline at 512 MiB. The write score was within 0.18% of `e-hash-pin`, and durable remember ranges overlapped. That margin determines the configuration without establishing a durable write advantage.

The hit hint drops Bloom filters for sorted ingestion and bottom-level compaction. All-level pinning keeps full unpartitioned index and filter blocks resident after reopen. Partitioned indexes retain their existing cache behavior. The evidence includes command counts, CPU 7 activity, and trial ranges for every candidate. [Protocol deviation](PROTOCOL.md#2026-11-22-deviation-1-supplemental-read-knob-screen) · [Supplemental screen](SCREEN-SUPPLEMENT.md) · [Audit](AUDIT-SUPPLEMENT.json)

## 1M warm reads

Cells give median p50 µs / p99 µs / call kops/s across five trials.

| Layout and cache | Operation | Kiban off | Kiban on | Fjall tuned | Fjall LZ4 | LMDB |
| --- | --- | --- | --- | --- | --- | --- |
| Bulk, 512 MiB | Recall-by-ID | 1.568 / 2.368 / 643.302 | 0.752 / 1.024 / 1,319.635 | 1.664 / 2.432 / 594.383 | 1.664 / 2.432 / 596.302 | 1.248 / 1.728 / 793.572 |
| Bulk, 512 MiB | Latest-ten | 2.688 / 4.864 / 360.717 | 1.824 / 3.328 / 529.222 | 4.864 / 7.808 / 206.124 | 4.864 / 7.168 / 206.834 | 2.240 / 4.032 / 434.933 |
| Bulk, default | Recall-by-ID | 1.504 / 2.368 / 662.050 | 0.720 / 1.056 / 1,415.475 | 1.664 / 2.496 / 594.411 | 1.664 / 2.624 / 591.090 | 1.280 / 1.984 / 763.481 |
| Bulk, default | Latest-ten | 2.688 / 4.992 / 359.425 | 1.824 / 3.456 / 534.256 | 4.864 / 7.296 / 206.340 | 4.736 / 7.552 / 206.711 | 2.240 / 4.224 / 430.993 |
| Normal, 512 MiB | Recall-by-ID | 1.664 / 2.624 / 602.393 | 0.976 / 1.344 / 1,016.691 | 1.760 / 2.816 / 557.861 | 1.824 / 3.072 / 534.233 | 1.280 / 1.824 / 783.126 |
| Normal, 512 MiB | Latest-ten | 3.072 / 5.504 / 316.220 | 1.760 / 3.584 / 549.073 | 5.760 / 8.192 / 171.196 | 7.168 / 11.264 / 135.991 | 2.176 / 3.584 / 449.286 |
| Normal, default | Recall-by-ID | 1.632 / 2.624 / 604.636 | 0.944 / 1.248 / 1,053.592 | 1.760 / 2.816 / 566.184 | 1.824 / 2.944 / 547.138 | 1.248 / 1.760 / 803.366 |
| Normal, default | Latest-ten | 3.008 / 4.736 / 327.299 | 1.728 / 2.752 / 567.547 | 5.760 / 8.064 / 171.086 | 6.912 / 9.472 / 144.491 | 2.112 / 3.392 / 463.368 |

Against LMDB, indexed Kiban's same-round p50 advantage was 1.29–1.78× for recall and 1.23–1.25× for latest-ten. In the new normal/default rounds, Fjall recall p50 moved from 2.048 to 1.760 µs and latest-ten from 6.144 to 5.760 µs. Those primary rounds used fresh fixtures and ran at different times. The supplemental screen provides the same-round comparison of the new settings with `block4`. [Original primary](superseded-initial-screen/REPORT.md) · [Paired ratios](RESULTS.md#same-round-kiban-ratios)

## 10k writes

Cells give median p50 µs / p99 µs / call kops/s across five trials. Mixed p50 mostly reflects reads because 900 of 1,000 calls are reads.

| Mode | Operation | Kiban off | Kiban on | Fjall tuned | Fjall LZ4 | LMDB |
| --- | --- | --- | --- | --- | --- | --- |
| Fast | Remember | 0.576 / 4.992 / 1,269.553 | 0.576 / 4.864 / 1,272.213 | 0.672 / 5.504 / 1,092.218 | 0.672 / 5.632 / 943.342 | 5.888 / 9.984 / 161.758 |
| Fast | Update | 0.656 / 5.376 / 938.236 | 0.656 / 6.144 / 1,112.183 | 0.736 / 5.504 / 1,028.298 | 0.736 / 5.504 / 1,022.397 | 5.760 / 9.216 / 171.034 |
| Fast | Mixed | 0.768 / 1.632 / 1,208.553 | 0.432 / 1.008 / 1,976.269 | 1.056 / 1.760 / 965.282 | 1.088 / 1.728 / 915.800 | 0.592 / 6.272 / 894.907 |
| Durable | Remember | 573.440 / 1,540.096 / 1.664 | 557.056 / 3,932.160 / 1.505 | 507.904 / 1,409.024 / 1.877 | 516.096 / 1,212.416 / 1.844 | 1,114.112 / 2,359.296 / 0.833 |
| Durable | Update | 573.440 / 1,671.168 / 1.635 | 557.056 / 3,735.552 / 1.404 | 507.904 / 1,277.952 / 1.846 | 540.672 / 1,376.256 / 1.786 | 1,212.416 / 2,621.440 / 0.781 |
| Durable | Mixed | 1.856 / 720.896 / 16.491 | 1.152 / 589.824 / 17.733 | 2.240 / 622.592 / 17.743 | 2.368 / 606.208 / 18.405 | 1.376 / 1,441.792 / 7.861 |

Fjall's durable remember p50 was 508 µs (508–573 µs) and indexed Kiban's was 557 µs (516–590 µs). Fjall also had lower median durable p99 for remember and update, but the p99 ranges overlap. Indexed Kiban led fast remember and update p50. Its fast mixed median call throughput was 1.98 million calls/s, versus 0.97 million for Fjall. [Write ranges](RESULTS.md#write-durable)

## Durable write syscalls

In the initial primary, durable remember p50 was 524 µs for Fjall and 557 µs for indexed Kiban. The five-trial ranges overlapped at 516–541 µs. Outside timing, `strace -f -C` captured syscall counts and successful return bytes for one complete durable command per engine. Both commands verified 2,100 mutations and the reopened 11,000-record model. The table shows journal calls per mutation. The write columns also give bytes per mutation.

| Engine | write calls / bytes | pwrite calls / bytes | fdatasync calls | fsync calls |
| --- | --- | --- | --- | --- |
| Kiban index on | 1 / 4,095.99 | 0 / 0 | 1 | 0 |
| Fjall tuned | 1 / 380.32 | 0 / 0 | 1 | 0 |

Both engines issued one `write` and one `fdatasync` per mutation. Neither used vectored or positioned journal writes. Kiban handed about 10.8× as many journal bytes to the kernel because its WAL pads to 4 KiB sync alignment. This measures bytes accepted by the syscall. Physical storage traffic and the share of latency attributable to padding remain unmeasured. [WAL sync implementation](../../src/wal.rs)

The whole-command totals include setup, flush, and output. Both traces contain an additional journal `fsync` on open. [Trace summaries and raw records](checks/durable-syscalls/summary.json)

## Footprint and layout

Cells give median logical disk MiB / allocated disk MiB / whole-process peak RSS MiB at the default cache budget.

| Layout | Kiban off | Kiban on | Fjall tuned | Fjall LZ4 | LMDB |
| --- | --- | --- | --- | --- | --- |
| Bulk | 314.564 / 314.574 / 457.156 | 351.854 / 351.863 / 407.688 | 306.177 / 306.211 / 463.742 | 231.714 / 231.742 / 439.891 | 799.305 / 799.297 / 839.297 |
| Normal | 314.573 / 314.699 / 460.676 | 352.359 / 352.664 / 450.836 | 366.106 / 366.133 / 467.023 | 292.716 / 292.750 / 466.836 | 799.305 / 799.297 / 839.457 |

Fjall LZ4 used 24% less logical disk space than uncompressed Fjall on bulk and 20% less on normal. At the default cache budget tuned Fjall peak RSS rose from 383 to 464 MiB on bulk relative to the initial primary. Normal peak RSS moved from 465 to 467 MiB. These are whole-configuration comparisons across runs.

Disk metrics cover all database files after process exit. Allocated bytes use `st_blocks × 512`. Btrfs compression and reflinks affect the relationship to unique physical extents. Peak RSS covers the whole process, including verification and reopen. [Resources by scenario](RESULTS.md#resources)

## Host noise and scope

All 270 supplemental screening and 150 new primary commands passed. They verified 36.45 million and 20.25 million timed calls, respectively. The audit checks result and source hashes, the trial order, per-stage call counts, reopen receipts, and read fixture table identities. [Primary audit](AUDIT.json) · [Screen audit](AUDIT-SUPPLEMENT.json)

Measured commands and diagnostics used CPU 6. Builds and coordinators used CPUs 13–21 at nice +10 to leave CPU 12 to other work. CPU 7 received no work from this task. During primary commands its busy percentage was 0.51 median and 0.00–22.22 across command intervals. One-minute host load was 2.18 median and 1.73–2.50. Shared package, memory and storage contention can still affect the results. [Host record](host.json) · [Per-command activity](TRIALS.csv)

The original one-worker preparation stall and the paced normal loader are documented in the protocol and archive. The measurements cover warm point hits and 64 distinct latest-ten slices. Cold reads, misses, constrained-cache pressure, long scans, group commit, power loss, and sustained compaction remain outside the workload. [Preparation deviation](PROTOCOL.md#2026-11-22-preparation-deviation) · [Archived stall evidence](superseded-initial-screen/checks/preparation-stall/backtraces.log)

## Reproduce

Use a fresh fixture directory and a fresh evidence directory. These commands assume compatible Clang/libclang discovery for the readpath harness. On this host, set `LIBCLANG_PATH=/usr/lib64/llvm15/lib` and `BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/usr/lib64/llvm15/lib/clang/15.0.7`. Cargo can build the linked RocksDB dependency directly from the lockfile. The recorded run reused the archive in the host record.

```sh
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/target/fjall-h2h"
mkdir -p target/fjall-data
taskset -c 13-21 nice -n 10 cargo build --locked --release --features remix \
  --manifest-path tools/agentbench/readpath/Cargo.toml --bin kiban-readbench
taskset -c 13-21 nice -n 10 cc -O2 -shared -fPIC \
  tools/agentbench/readpath/rocksdb_sync_probe.c -o target/fjall-h2h/sync-probe.so -ldl
for phase in prepare screen select prepare-lz4 measure probe; do
  taskset -c 13-21 nice -n 10 python3 tools/agentbench/readpath/fjall_bench.py "$phase" --supplement \
    --out target/fjall-reproduction/raw || exit 1
done
taskset -c 13-21 nice -n 10 python3 tools/agentbench/readpath/fjall_report.py --supplement \
  --out target/fjall-reproduction
taskset -c 13-21 nice -n 10 python3 tools/agentbench/readpath/fjall_syscalls.py \
  --out target/fjall-reproduction --strace "$(command -v strace)"
```
