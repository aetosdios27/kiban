# Kiban vs tuned Fjall, with LMDB control

Indexed Kiban had **2.10–2.41× lower recall-by-ID p50** and **2.91–3.50× lower latest-ten p50** than tuned Fjall across four warm 1M-record scenarios. These factors use the median same-round latency ratios. Indexed Kiban also led LMDB on both read medians. Fjall had lower median durable insert and update latency. Fjall LZ4 used the least disk space on both 1M layouts.

| Default-cache workload | Kiban index off p50 µs | Kiban index on p50 µs | Fjall tuned p50 µs | Fjall LZ4 p50 µs | LMDB p50 µs |
|---|---:|---:|---:|---:|---:|
| 1M normal recall-by-ID | 1.600 | 1.056 | 2.048 | 2.016 | 1.280 |
| 1M normal latest-ten | 3.136 | 1.760 | 6.144 | 6.016 | 2.176 |
| 10k fast remember | 0.624 | 0.592 | 0.832 | 0.736 | 6.272 |
| 10k durable remember | 557.056 | 557.056 | 524.288 | 516.096 | 1,277.952 |
| 10k durable update | 557.056 | 573.440 | 524.288 | 540.672 | 1,277.952 |

Cells are medians of five trials. The full results include every stage, p99, call and verified throughput, same-round ratios, and minimum and maximum values. [Primary results](RESULTS.md) · [Per-trial CSV](TRIALS.csv)

## Workload and timing boundary

The fixture contains 64 agents. Each value encodes the identity, creation sequence, revision and 200 ASCII text bytes as JSON, with seed 20261016. An ID determines its agent and creation sequence. The composite key orders each agent's values by newest first. Fjall uses an ascending prefix iterator and decodes the value from each guard. The latest-ten workload revisits 64 distinct slices, even with a 1M-record fixture.

Each read command performs 100k recall-by-ID calls and 100k latest-ten calls. Kiban bulk-preloads records in batches and flushes them. Fjall bulk uses sorted ingestion. Normal fixtures use individual ID-order puts and retain the engines' natural compaction layouts. LMDB uses its same single-transaction, ID-order loader in both layout groups. Each write command copies a 10k bulk fixture and runs 1k calls in each of five stages. Mixed has 900 reads and 100 updates. In total, each write command performs 2,100 mutations.

The timer includes key construction, JSON encoding or decoding, owned results and any requested durability. The deterministic input and returned Memory destruction are outside the timer. Each timed read and write readback is checked against the oracle. Full point and all-agent verification runs before and after measurement, and every write command reopens and checks the final revision model. Call throughput divides by accumulated call time. Verified throughput uses stage wall time, including oracle and write readback costs. Both exclude open, full-model verification and final persistence. Histogram percentiles are upper bucket bounds.

## Versions and options

| Engine | Release and options |
|---|---|
| Kiban | 0.2.0, engine source at `a9906ad1`. The index-on row uses `KibanOptions::agent_memory(18)`. The index-off row disables the point and prefix scan indexes. Both call `prefix_values`. Neither sets mmap or binary-fuse filtering. |
| Fjall | 3.1.13 with lsm-tree 3.1.10. One `memories` keyspace, one background worker and default leveled compaction. Data block size, whole-key Bloom bits and memtable size are screened. Index/filter placement uses crate defaults. LZ4 rows use lz4_flex 0.13.1 on data blocks only. Fjall has no vendored C engine or compression library. |
| LMDB | heed3 0.22.0, lmdb-master3-sys 0.2.5. The bundled master3 header reports 0.9.90, dated May 1, 2017. The environment retains a 10 GiB virtual map, max_dbs=200 and max_readers=200. |

The two LSMs receive the same 536,870,912-byte or 448,987,008-byte block-cache capacity. The latter is Kiban's default on this host and is also used for writes. Index/filter residency and OS page cache are separate from the configured capacity. LMDB relies on OS-managed mmap pages. All engines use the same release process configuration with Rust MiMalloc 0.1.48 and serde_json 1.0.143. The binary also links the existing static RocksDB 11.8.1 archive for harness compatibility. No measured row opens RocksDB. Exact source commits and crate checksums are in the protocol. The compiler, archive digest and host configuration are in the host record. [Protocol](PROTOCOL.md) · [Host](host.json) · [Binary and source hashes](raw/metadata.json)

## Durability boundary

| Engine | Durable acknowledgment | Fast acknowledgment |
|---|---|---|
| Kiban | Put followed by `Kiban::sync()`, which flushes and fdatasyncs the WAL | Buffered WAL and memtable put |
| Fjall | Insert followed by `Database::persist(SyncData)`, which flushes and fdatasyncs the journal | Insert into the memtable and manually persisted journal, with no explicit persist at acknowledgment |
| LMDB | One synchronous committed transaction per mutation | One transaction per mutation with `NO_SYNC` and `NO_META_SYNC` |

Both LSM fast modes can acknowledge data still in userspace. Their buffering and background flush policies can differ. LMDB fast also relaxes per-write durability. The final sync sits outside the operation timer. The sync probe checks successful calls over the full command, including setup and finalization. The SIGKILL check tests survival on a running OS. Machine power-loss behavior remains outside this experiment.

| Engine | Fast fsync / fdatasync | Durable fsync / fdatasync |
|---|---:|---:|
| Kiban index on | 9 / 2 | 9 / 2,102 |
| Fjall tuned | 12 / 1 | 12 / 2,101 |
| LMDB | 0 / 1 | 0 / 2,101 |

Every durable probe added exactly 2,100 successful fdatasync calls over its fast counterpart. The Fedora host lacked strace, so the diagnostics used the existing libc preload probe. Fjall's source calls `File::sync_data` on the journal before returning from `SyncData`. In a separate trial, the parent received 25 durable acknowledgments, sent SIGKILL and reopened the database in a new process. The complete 10,025-record model passed verification. [Sync counts](checks/sync.json) · [SIGKILL receipt](checks/crash.json)

## Frozen screen

The same 4 KiB, 10-bit Bloom, 64 MiB memtable configuration won both selections. The write score was within 0.6% of the 16 MiB memtable candidate. That margin selects a configuration under the protocol. It gives little evidence for a write benefit from the larger memtable. The read score was 5.4% below the next lowest score. Each score is a geometric mean over workloads and conditions. [All candidates and control trials](SCREEN.md) · [Frozen selection](raw/selection.json)

| Candidate | Data block KiB | Whole-key Bloom bits/key | Memtable MiB | Read score µs | Write score µs |
|---|---:|---:|---:|---:|---:|
| block16 | 16 | 10 | 64 | 4.355 | 27.692 |
| block4 | 4 | 10 | 64 | 4.091 | 26.156 |
| bloom6 | 4 | 6 | 64 | 4.426 | 27.758 |
| mem16 | 4 | 10 | 16 | 4.325 | 26.312 |

One-worker normal preparation initially stalled in Fjall's bounded worker queue. A backtrace showed its rotation worker and the caller both waiting to enqueue a flush. The normal loader now pauses at the configured memtable limit minus 1 KiB to rotate and flush. It retains individual ID-order puts and the normal compaction picker. This change, the rebuild and fresh verification all preceded screening. The short timed write stages fit below the memtable limit and use the same put path throughout. [Dated protocol deviation](PROTOCOL.md#2026-11-22-preparation-deviation) · [Archived stall evidence](checks/preparation-stall/backtraces.log)

## 1M warm reads

Cells give **p50 µs / p99 µs / call kops/s**. Each value is the median of five trials.

| Layout and cache | Operation | Kiban index off | Kiban index on | Fjall tuned | Fjall LZ4 | LMDB |
|---|---|---:|---:|---:|---:|---:|
| Bulk, 512 MiB | Recall-by-ID | 1.600 / 2.752 / 618.2 | 0.800 / 1.152 / 1,225.8 | 1.920 / 3.136 / 512.1 | 1.952 / 2.816 / 515.6 | 1.280 / 1.760 / 777.8 |
| Bulk, 512 MiB | Latest-ten | 2.624 / 3.904 / 376.3 | 1.824 / 2.688 / 544.0 | 5.248 / 7.040 / 193.3 | 5.120 / 6.784 / 194.5 | 2.176 / 3.072 / 455.9 |
| Bulk, default | Recall-by-ID | 1.824 / 2.816 / 539.3 | 0.784 / 1.152 / 1,235.2 | 1.888 / 3.008 / 522.1 | 2.240 / 3.136 / 445.5 | 1.312 / 1.792 / 765.5 |
| Bulk, default | Latest-ten | 2.816 / 4.864 / 348.8 | 1.824 / 3.840 / 532.4 | 5.120 / 7.936 / 191.1 | 5.376 / 8.704 / 187.3 | 2.176 / 3.648 / 454.8 |
| Normal, 512 MiB | Recall-by-ID | 1.632 / 2.560 / 603.5 | 0.976 / 1.344 / 1,018.7 | 2.176 / 3.904 / 449.1 | 2.048 / 3.200 / 483.2 | 1.280 / 1.760 / 777.6 |
| Normal, 512 MiB | Latest-ten | 3.136 / 5.120 / 313.2 | 1.792 / 3.008 / 541.4 | 6.272 / 10.752 / 152.0 | 6.144 / 9.216 / 162.5 | 2.112 / 3.520 / 464.2 |
| Normal, default | Recall-by-ID | 1.600 / 2.880 / 615.9 | 1.056 / 1.408 / 971.2 | 2.048 / 3.456 / 476.6 | 2.016 / 3.136 / 496.3 | 1.280 / 1.824 / 780.3 |
| Normal, default | Latest-ten | 3.136 / 4.992 / 314.0 | 1.760 / 2.944 / 556.1 | 6.144 / 9.216 / 160.4 | 6.016 / 9.728 / 163.1 | 2.176 / 3.520 / 453.3 |

Against LMDB, indexed Kiban's same-round p50 was 1.21–1.60× lower for recall and 1.18–1.26× lower for latest-ten. With the indexes off, Kiban trailed LMDB on both read medians in all four scenarios. In the bulk/default group, even indexed Kiban had a slightly higher paired latest-ten p99 than LMDB, with a ratio of 1.016. In the same group, index-off recall was within 4% of Fjall by displayed p50. The preparation logs confirm that the indexed Kiban profile has direct latest-ten extents for all 64 prefixes. [Paired ratios and ranges](RESULTS.md#same-round-kiban-ratios) · [Fixture receipts](raw/prepare/commands.jsonl)

## 10k writes

Cells use the same p50, p99 and throughput convention. The full results include recall and latest-ten stages from the write suite.

| Mode | Operation | Kiban index off | Kiban index on | Fjall tuned | Fjall LZ4 | LMDB |
|---|---|---:|---:|---:|---:|---:|
| Fast | Remember | 0.624 / 5.760 / 1,117.0 | 0.592 / 5.248 / 1,215.1 | 0.832 / 6.400 / 901.4 | 0.736 / 5.632 / 1,027.8 | 6.272 / 18.432 / 135.8 |
| Fast | Update | 0.704 / 6.144 / 836.0 | 0.672 / 6.144 / 1,092.3 | 0.880 / 5.760 / 876.0 | 0.816 / 5.632 / 943.2 | 6.272 / 9.728 / 159.6 |
| Fast | Mixed | 0.864 / 1.920 / 1,085.1 | 0.464 / 0.976 / 1,905.5 | 1.632 / 2.816 / 641.1 | 1.504 / 2.432 / 685.7 | 0.752 / 6.912 / 761.6 |
| Durable | Remember | 557.056 / 1,638.400 / 1.620 | 557.056 / 1,540.096 / 1.649 | 524.288 / 1,441.792 / 1.747 | 516.096 / 2,031.616 / 1.518 | 1,277.952 / 2,949.120 / 0.722 |
| Durable | Update | 557.056 / 3,538.944 / 1.497 | 573.440 / 2,686.976 / 1.553 | 524.288 / 1,441.792 / 1.776 | 540.672 / 1,802.240 / 1.678 | 1,277.952 / 6,422.528 / 0.614 |
| Durable | Mixed | 2.432 / 622.592 / 16.226 | 1.504 / 704.512 / 15.930 | 2.944 / 638.976 / 16.708 | 3.072 / 655.360 / 16.493 | 1.568 / 1,507.328 / 7.242 |

Fjall's median durable remember and update p50 beat both Kiban rows. The paired indexed Kiban/Fjall p50 ratios were 1.049 and 1.059. Durable update p50 ranges overlap at 524–557 µs. The tuned Fjall update p99 ranged from 0.999 to 7.864 ms, versus 1.311 to 4.325 ms for indexed Kiban. Fjall had better median tails and higher median call throughput for durable inserts and updates. The paired throughput ratios for indexed Kiban were 0.942 and 0.988. The tail ranges and paired update rate leave the size of the advantage uncertain.

In mixed durable, indexed Kiban had a lower p50. That percentile mostly reflects reads. Fjall had higher median call and verified throughput. In fast mode, indexed Kiban led Fjall on remember and update p50 and on mixed throughput. Fjall's displayed median fast-update p99 was lower, while the paired indexed Kiban/Fjall median p99 ratio was slightly below one. In the primary fast commands, final sync, flush and full post-write verification took a median 23.1 ms for indexed Kiban and 32.0 ms for tuned Fjall across primary trials. These costs are outside stage throughput. [Write ranges](RESULTS.md#write-durable) · [Completion times](TRIALS.csv)

## Footprint and layout

Cells give **logical disk MiB / allocated disk MiB / whole-process peak RSS MiB**, using default-cache medians. The resource tables cover both cache budgets, writes and ranges.

| 1M layout | Kiban index off | Kiban index on | Fjall tuned | Fjall LZ4 | LMDB |
|---|---:|---:|---:|---:|---:|
| Bulk | 314.564 / 314.574 / 458.594 | 351.854 / 351.863 / 407.875 | 304.308 / 304.340 / 382.848 | 231.316 / 231.344 / 366.883 | 799.305 / 799.297 / 839.574 |
| Normal | 314.573 / 314.699 / 460.602 | 352.359 / 352.664 / 450.867 | 364.437 / 364.469 / 464.520 | 292.282 / 292.312 / 386.949 | 799.305 / 799.297 / 839.430 |

Tuned Fjall bulk had five tables and no L0 tables after reopen. Normal also had five tables, including one L0 table. The remaining journal file accounted for about 57.7 MiB of normal's logical footprint. The LZ4 fixtures had four bulk tables and six normal tables. Both Kiban bulk fixtures had one SST. Kiban normal had 84 SSTs with the indexes off and 89 with them on. Enabling the Kiban profile added 37–38 MiB to logical disk usage here. Fjall LZ4 reduced total logical size by approximately 24% for bulk and 20% for normal relative to tuned Fjall.

Disk metrics cover all database files after process exit. Allocated bytes use `st_blocks × 512`. Btrfs compression and reflinks affect the relationship to unique physical extents. Peak RSS covers the whole process, including verification and reopen. LMDB uses the existing ID-order loader, so its footprint describes this control layout. [Full resource results](RESULTS.md#resources)

## Host noise and scope

All 150 screening and 150 primary commands passed. Each phase verified 20.25 million timed calls. The audit checks the trial order, expected calls and mutations, results and source hashes, binary digest, reopen receipts and read fixture table identities. The orchestration shell received SIGTERM between the last successful primary receipt and its commit. All 150 primary receipts and output hashes passed the audit, and no primary command was rerun. [Audit](AUDIT.json)

The Intel Core Ultra 9 185H host ran the release binary and all its threads on CPU 6. Build and coordinator work used CPUs 12–21 at nice +10. Benchmark processes inherited nice +10. CPU 7 received no work from this task, but it was active during primary commands. Busy time was 1.44% median and 0–12.87% across command intervals. The one-minute load was 3.19 median and 2.52–4.33. These intervals include full-model checks. Rotation balances trial positions. It cannot remove shared CPU package, memory or storage contention. The durable tail ranges and the near-tied write screening score reflect this uncertainty. [Host inventory](host.json) · [Per-command activity](TRIALS.csv)

These results cover warm point hits and short latest-ten scans for a single worker. Both block-cache budgets exceed the total table-file sizes of the LSM read fixtures. The study leaves cold starts, constrained-cache pressure, arbitrary ID-to-agent mappings, concurrent updates, group commit, sustained compaction throughput, long scans and power loss unmeasured. The Fjall screen covers three capacity and layout knobs on leveled compaction. In particular, it leaves alternative pinned metadata policies and hit-specific Bloom behavior at defaults.

## Reproduce

Use a fresh fixture directory and a fresh evidence directory. These commands assume compatible Clang/libclang discovery for the readpath harness. On this host, set `LIBCLANG_PATH=/usr/lib64/llvm15/lib` and `BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/usr/lib64/llvm15/lib/clang/15.0.7`. Cargo can build the linked RocksDB dependency directly from the lockfile. The original run reused the archive in the host record.

```sh
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/target/fjall-h2h"
mkdir -p target/fjall-data
taskset -c 12-21 nice -n 10 cargo build --locked --release --features remix \
  --manifest-path tools/agentbench/readpath/Cargo.toml --bin kiban-readbench
taskset -c 12-21 nice -n 10 cc -O2 -shared -fPIC \
  tools/agentbench/readpath/rocksdb_sync_probe.c -o target/fjall-h2h/sync-probe.so -ldl
for phase in prepare screen select prepare-lz4 measure probe; do
  taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/fjall_bench.py "$phase" \
    --out target/fjall-reproduction/raw || exit 1
done
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/fjall_report.py \
  --out target/fjall-reproduction
```
