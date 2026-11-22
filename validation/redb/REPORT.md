# Kiban vs tuned redb, with LMDB control

Kiban with the agent-memory profile had the lowest median p50 and p99 in all eight 1M warm-read comparisons. Across the two layouts and two cache budgets, redb's paired point p50 was **2.00–2.17×** Kiban's, and its latest-ten p50 was **1.49–1.62×** Kiban's. LMDB's paired p50 was **1.27–1.62×** Kiban's for point reads and **1.14–1.25×** for latest ten. Turning off Kiban's point and scan indexes changed the result. redb had lower latest-ten p50 and p99 in both normal-layout groups, and LMDB beat index-off Kiban on both read operations across all four groups.

Kiban had lower observed durable mutation p50s and higher call rates in these five rounds. The indexed Kiban durable remember and update p50s were both 557 µs, versus 672 and 721 µs for redb. The tails varied widely. In fast mode, Kiban's buffered writes returned sooner than redb's non-durable individual transactions. Neither fast mode promises per-acknowledgment persistence.

Maximum compaction with quick repair off won both redb screens. The resulting bulk read fixture occupied 361.7 MiB on disk, close to indexed Kiban's 351.9 MiB. redb's organic normal fixture used 658.9 MiB allocated. These are shared-host results with five rounds per cell.

## Workload and timing boundary

The shared model uses 64 agents, seed 20261016, and 200 bytes of ASCII text per record. The JSON payload contains the record ID, agent, immutable creation sequence, revision, and text. All three engines store the same composite key and identical JSON bytes. Because the model derives the agent and creation sequence from the ID, recall uses one primary-key lookup. Latest-ten queries revisit 64 distinct ten-record slices.

Each read command starts with a full check of a 1M-record fixture, then runs 100k recalls and 100k latest-ten calls. Each write command starts with a fresh copy of a 10k-record bulk fixture. It performs 1k each of remember, recall, latest ten, update, and mixed calls. Mixed consists of 900 point reads and 100 updates. The three mutation stages acknowledge 2,100 individual writes in total.

The per-call timer covers key construction, encoding, database work, decoding, and construction of the owned return value. Input payload generation and destruction of the returned result fall outside the timer. The oracle compares every timed read and reads back every mutation outside the timer. All commands fully verify their expected population and each agent's latest ten at the start and end. Write commands also close, reopen, and verify the full revision model. Call throughput divides count by the sum of call durations. Verified throughput uses stage wall time and includes the oracle, histogram, and readbacks. Both exclude the full population checks and the final persistence step. Histogram percentiles are upper bucket bounds, with less than 3.125% relative bucket error above 32 ns.

## Versions and options

| Item | Setting |
|---|---|
| Kiban engine | Source tree at a9906ad1, with no engine edits in this study |
| Kiban indexed | KibanOptions::agent_memory(18) equivalent through the remix harness feature, point and scan indexes enabled |
| Kiban index off | Point index and its dependent scan extension disabled, with the same remaining options |
| redb | 5.0.0, source 0d53424a7e0a38b2665e4d41f40bb6372da953ad, pure Rust database |
| redb storage | One uncompressed table, 4 KiB pages, default exclusive-writer mode |
| redb reads | One fresh read transaction per call, decoding from the borrowed value |
| redb latest ten | Descending comparator over the same key bytes, bounded reverse traversal, and a ten-record limit |
| LMDB control | heed3 0.22.0 and lmdb-master3-sys 0.2.5, whose vendored LMDB header identifies version 0.9.90 |
| LMDB reads | Mapped values, one read transaction per call, 10 GiB virtual map, max_dbs=200 and max_readers=200 |
| Cache budgets | 536,870,912 bytes and 448,987,008 bytes for the two read groups, with the latter used for writes. Both Kiban and redb receive the exact input budgets. |
| Binary | Rust 1.98.1, MiMalloc 0.1.48, serde_json 1.0.143, release opt-level 2, one codegen unit, LTO, panic abort, and debug symbols |

The Kiban bulk loader writes batches before one flush. Normal Kiban loads records one by one, flushes at the default write-buffer threshold, and lets its compaction picker choose levels. LMDB uses the same ID-order single-transaction fixture for both read groups. redb bulk uses an ID-order single-transaction load. The normal redb fixture commits each write individually using None and checkpoints at the end. The two compacted bulk fixtures are copies of the same original.

Matching cache inputs leaves different effective memory use. Kiban's metadata and redb's page-management state affect RSS, and LMDB relies on the OS page cache. A full-population check warms each process. Peak RSS covers the entire process including setup, checks, and reopening. Logical disk bytes sum regular file lengths. Allocated bytes use st_blocks × 512. The Btrfs figures reflect file block accounting on the shared filesystem, including sparse regions and compression, and do not measure unique physical extents or bytes written.

The crate archive checksums, vendored library identification, toolchain, filesystem, and binary hash are in [native-provenance.json](native-provenance.json), [host.json](host.json), and [raw/metadata.json](raw/metadata.json).

## Screening and fixture layouts

The selection rule minimizes the geometric mean of stage-level median mean call times. The read score includes both operations in all four scenarios. The write score includes remember, update, and mixed in fast and durable mode. The screen ran five rotated rounds for each scenario and retained all 150 commands, including LMDB controls. The primary runs started after committing the winners.

| redb configuration | Read score µs | Write score µs |
|---|---:|---:|
| As loaded | 2.980 | 57.792 |
| Balanced compaction | 3.044 | 57.385 |
| Maximum compaction | **2.751** | **54.601** |
| Balanced compaction and quick repair | 3.013 | 76.148 |

Maximum won by about 8% in the read score and 5% in the write score over the next candidate in each family. It packs partially filled pages as well as trimming the file. Quick repair saves allocator state and uses two-phase commit for durable transactions. Both balanced labels use identical bulk read fixture bytes and the same read path. Their small read-score difference reflects trial variation. All candidates share the as-loaded normal read fixture. [SCREEN.md](SCREEN.md) and [raw/selection.json](raw/selection.json) retain the stage and scoring evidence.

| 1M redb fixture | Logical MiB at preparation | Allocated MiB at preparation | Fragmented MiB in redb stats |
|---|---:|---:|---:|
| Bulk as loaded | 1042.56 | 658.84 | 750.34 |
| Bulk with Balanced | 658.88 | 658.84 | 320.90 |
| Bulk with Maximum | 361.82 | 361.80 | 28.70 |
| Normal as loaded | 1042.60 | 658.86 | 750.33 |

The normal load used about 40 KiB more logical and 20 KiB more allocated space than the uncompacted bulk load. Both reported approximately 750 MiB of fragmentation, including allocator free pages. Maximum reduced allocated bytes by about 45% against uncompacted bulk. Closing redb's read-write handle persists allocator state and can trim free tail pages even when the command only reads records. During screening, the Balanced and Maximum copies shrank by 128 KiB and 64 KiB respectively. The read oracle continued to pass. All recorded sizes are retained. [fixture-layouts.json](fixture-layouts.json) · [redb 5.0.0 close implementation](https://docs.rs/crate/redb/5.0.0/source/src/db.rs)

Each Kiban bulk fixture had one L0 SST. Its normal layout had 1 L1, 6 L2, and 77 L3 files without the point and scan indexes, and 1 L1, 6 L2, and 82 L3 files with them. Those normal groups measure the profile and its resulting layout together.

## redb key-order A/B

The descending comparator and reverse latest-ten range remain the study configuration. The opt-in candidate used redb's native byte-slice comparator and a forward range over the same key and value bytes. Across five paired rounds on CPU 8, every p50 and p99 trial range overlapped or touched. The largest median improvement among the 1M read cases was bulk recall at default cache, from 1.952 to 1.792 µs p50 and 2.880 to 2.496 µs p99. Normal recall at default cache moved in the other direction at p50, from 2.496 to 2.624 µs. The plain-key layout allocated 365.8 MiB versus 361.8 MiB after Maximum compaction and 753.9 MiB versus 658.9 MiB for the organic normal fixture.

The prespecified rule required a disjoint trial range and a median difference above one histogram bucket. The plain-key candidate did not meet the improvement criterion, so the original primary and durability checks stand. All 60 A/B commands passed the oracle, covering 8.1 million timed calls and 42,000 mutations. CPU 9 was busy for a median 2.0% of command time, with a 0–35.3% range. Host load ranged from 4.25 to 8.23. The A/B measures traversal direction, comparator, and the resulting B-tree layout together. [Key-order method](keyorder-ab/README.md) · [Per-operation latencies and ranges](keyorder-ab/RESULTS.md) · [Throughput and audit receipts](keyorder-ab/summary.json)

## 1M warm reads

Each cell gives p50 µs / p99 µs / call kops/s. Values are five-trial medians.

| Read fixture | Operation | Kiban index off | Kiban indexed | redb | LMDB |
| --- | --- | ---: | ---: | ---: | ---: |
| bulk, 512 MiB | recall-by-id | 1.888 / 2.752 / 501.3 | 0.960 / 1.248 / 1010.1 | 1.920 / 2.624 / 498.9 | 1.504 / 2.112 / 605.0 |
| bulk, 512 MiB | recent-for-agent | 3.264 / 4.992 / 292.0 | 2.240 / 3.776 / 416.9 | 3.328 / 4.992 / 290.1 | 2.624 / 4.352 / 364.4 |
| bulk, default | recall-by-id | 1.888 / 2.688 / 494.7 | 0.928 / 1.184 / 1070.8 | 2.016 / 2.752 / 472.6 | 1.504 / 2.016 / 635.4 |
| bulk, default | recent-for-agent | 3.200 / 5.120 / 299.8 | 2.112 / 3.328 / 457.1 | 3.328 / 5.120 / 288.7 | 2.624 / 4.032 / 367.4 |
| normal, 512 MiB | recall-by-id | 2.112 / 3.520 / 450.6 | 1.120 / 1.600 / 857.8 | 2.240 / 5.120 / 380.7 | 1.440 / 2.304 / 657.4 |
| normal, 512 MiB | recent-for-agent | 3.584 / 7.552 / 252.4 | 2.048 / 4.352 / 456.1 | 3.264 / 6.912 / 280.9 | 2.432 / 5.632 / 380.6 |
| normal, default | recall-by-id | 1.664 / 2.560 / 602.1 | 0.992 / 1.280 / 1023.0 | 2.048 / 4.480 / 416.1 | 1.248 / 1.792 / 796.6 |
| normal, default | recent-for-agent | 3.072 / 4.736 / 321.6 | 1.696 / 2.432 / 581.3 | 2.752 / 4.096 / 355.4 | 2.048 / 3.200 / 478.4 |

## 10k write suites

Each cell gives p50 µs / p99 µs / call kops/s. Values are five-trial medians.

| Mode | Operation | Kiban index off | Kiban indexed | redb | LMDB |
| --- | --- | ---: | ---: | ---: | ---: |
| fast | remember | 0.592 / 4.992 / 1240.4 | 0.592 / 4.864 / 1241.2 | 7.680 / 10.496 / 113.2 | 6.016 / 9.472 / 159.7 |
| fast | recall-by-id | 0.656 / 0.992 / 1554.9 | 0.392 / 0.544 / 2528.5 | 0.864 / 1.216 / 1146.0 | 0.544 / 0.832 / 1773.6 |
| fast | recent-for-agent | 2.624 / 3.648 / 372.7 | 2.048 / 2.560 / 486.8 | 2.624 / 3.968 / 368.7 | 2.176 / 2.432 / 468.4 |
| fast | update | 0.672 / 4.992 / 957.6 | 0.672 / 4.992 / 1134.6 | 7.680 / 10.752 / 123.1 | 5.760 / 7.040 / 173.9 |
| fast | mixed-90-10 | 0.720 / 1.216 / 1383.0 | 0.432 / 0.912 / 2022.2 | 0.880 / 8.448 / 635.9 | 0.592 / 6.272 / 888.4 |
| durable | remember | 573.440 / 1507.328 / 1.7 | 557.056 / 1507.328 / 1.6 | 671.744 / 2621.440 / 1.0 | 1146.880 / 2490.368 / 0.8 |
| durable | recall-by-id | 0.976 / 2.112 / 971.1 | 0.672 / 1.312 / 1448.9 | 1.248 / 2.240 / 803.7 | 0.816 / 1.504 / 1235.7 |
| durable | recent-for-agent | 3.072 / 5.120 / 315.1 | 2.112 / 3.456 / 459.0 | 2.816 / 4.352 / 334.1 | 2.176 / 3.264 / 442.5 |
| durable | update | 557.056 / 1507.328 / 1.7 | 557.056 / 1310.720 / 1.6 | 720.896 / 1736.704 / 1.3 | 1114.112 / 2359.296 / 0.8 |
| durable | mixed-90-10 | 1.888 / 589.824 / 17.2 | 1.152 / 655.360 / 16.6 | 1.792 / 851.968 / 12.9 | 1.312 / 1376.256 / 8.0 |

## Whole-process memory and disk

Each cell gives peak RSS MiB / logical MiB / allocated MiB. Values are five-trial medians.

| Scenario | Kiban index off | Kiban indexed | redb | LMDB |
| --- | ---: | ---: | ---: | ---: |
| bulk, 512 MiB | 458.5 / 314.6 / 314.6 | 407.9 / 351.9 / 351.9 | 530.0 / 361.8 / 361.7 | 839.3 / 799.3 / 799.3 |
| bulk, default | 458.4 / 314.6 / 314.6 | 407.9 / 351.9 / 351.9 | 531.0 / 361.8 / 361.7 | 839.3 / 799.3 / 799.3 |
| normal, 512 MiB | 460.7 / 314.6 / 314.7 | 450.9 / 352.4 / 352.7 | 744.1 / 1042.6 / 658.9 | 839.2 / 799.3 / 799.3 |
| normal, default | 460.5 / 314.6 / 314.7 | 450.9 / 352.4 / 352.7 | 632.7 / 1042.6 / 658.9 | 839.4 / 799.3 / 799.3 |
| fast | 24.8 / 3.7 / 3.8 | 30.9 / 4.4 / 4.4 | 28.9 / 6.8 / 6.8 | 23.3 / 8.6 / 8.6 |
| durable | 24.5 / 3.7 / 3.8 | 30.7 / 4.4 / 4.4 | 22.7 / 6.8 / 4.2 | 23.1 / 8.6 / 8.6 |

Full ranges, verified throughput, and same-round p50, p99, call and verified rate ratios are in [MEASURE.md](MEASURE.md). Each paired ratio is computed within a round before taking the median and range. A ratio above one favors the listed engine.

On the 1M bulk fixture at default cache, indexed Kiban verified 716k point calls per second versus 381k for redb and 483k for LMDB. Its latest-ten call rate was 457k/s, with a verified rate of 152k/s versus redb's 127k/s and LMDB's 138k/s. Including the oracle narrows the observed rates, particularly for latest-ten, which checks ten decoded payloads per call.

For indexed Kiban versus redb, the same-round median call-rate advantage was 1.35× for durable remember and 1.30× for durable update. All five paired call-rate ratios favored indexed Kiban for those stages. Indexed Kiban's durable remember p99 ranged from 1.25 to 3.41 ms, and redb's ranged from 1.54 to 9.96 ms. The mixed p50 mostly reflects reads because nine in ten mixed calls are recalls. The Kiban index-off row had a higher median durable mixed call rate and lower p99 than indexed Kiban. LMDB also had slightly lower p99 than indexed Kiban in the durable suite's latest-ten stage. The tables retain the full tails and both Kiban rows.

## Durability and recovery

| Engine | Durable acknowledgment | Fast acknowledgment |
|---|---|---|
| Kiban | Put, then WAL sync before returning | Buffered WAL. A process kill can lose acknowledged writes still in userspace. |
| redb | One Immediate commit before returning. Quick repair off in the selected configuration. | One None commit. The new state has no crash-safe persistence guarantee until an Immediate commit checkpoints it. |
| LMDB | One synchronous commit with default flags | One commit under NO_SYNC and NO_META_SYNC. No per-commit sync. |

Kiban fast can replay a WAL prefix that reached the OS before a process kill. redb None leaves the acknowledged transaction without a durable commit point until a later Immediate commit. It gives no stronger persistence guarantee than Kiban fast. The final persistence operation and reopened-model check in every write command occur outside the call timers. [redb Durability contract](https://docs.rs/redb/5.0.0/redb/enum.Durability.html)

| Engine | Fast fdatasync / fsync | Durable fdatasync / fsync | Durable minus fast total syncs |
|---|---:|---:|---:|
| Kiban | 2 / 9 | 2102 / 9 | 2100 |
| redb | 13 / 0 | 2111 / 0 | 2098 |
| LMDB | 1 / 0 | 2101 / 0 | 2100 |

The full-process strace summaries recorded zero sync errors. A second trace with stage boundaries resolved redb's whole-process difference. Durable mode added 1,000 fdatasync calls by the remember boundary, 1,000 in update, and 100 in mixed, exactly matching the mutation counts. The two read-only intervals added none. The final fast-mode checkpoint added two fdatasyncs after mixed. These traces ran after primary timing. In the separate SIGKILL check, all 32 acknowledged durable inserts survived recovery and the reopened oracle verified all 10,032 records, including latest-ten results for every agent. The experiment covers process death and syscall behavior on this host. [Commit windows](checks/redb-commit-windows.json) · [Crash receipt](checks/crash-receipt.json)

## Host and limitations

The audit accepted 150 screen commands and 120 primary commands with zero oracle errors. Together they cover 36,450,000 timed calls and 189,000 mutations, excluding diagnostics. Every measured child reported CPU 8 affinity. Builds, tests, and coordinators ran on CPUs 12–21 at nice +10, with two Cargo jobs. Both feature sets passed 15 tests, formatting, and Clippy with warnings denied. [audit.json](audit.json) · [Build and test receipts](checks/receipt.json)

No study work used the SMT sibling. Other host activity kept CPU 9 busy for a median 0.97% of primary command time, with a 0–20.45% range. The one-minute load average ranged from 2.83 to 5.50 during primary commands. The shared NVMe, CPU package, LLC, and OS page cache can affect both latency and allocation. The host used the intel_pstate powersave governor with turbo enabled. All rounds were retained, and same-round pairing limits the effect of drift between periods. The study had no protocol amendments.

The results apply to fully warmed, single-worker operations over this 64-agent key distribution, on two read layouts and short mutation stages. JSON materialization is part of each call. The trace shows the extra persistence work in durable mode. The study did not isolate the read-path components, run cold-cache tests, exercise long scans or concurrent writers, or measure steady-state write amplification and power-loss recovery.

## Reproduce the campaign

Use a clean fixture area and a new output directory. The study driver creates fixtures directly under target/redb-data. Keep CPU 9 free of study work and keep builds and diagnostic commands outside the measurement window. On this host, bindgen needed the Clang resource paths recorded in host.json. The existing RocksDB 11.8.1 static archive sped up linking the unused RocksDB backend. The normal source build works for reproduction with the corresponding compiler and headers.

~~~sh
export CARGO_TARGET_DIR="$PWD/target/redb-h2h"
export CARGO_BUILD_JOBS=2
export PYTHONDONTWRITEBYTECODE=1
crate=tools/agentbench/readpath/Cargo.toml
driver=tools/agentbench/readpath/redb_bench.py
report=tools/agentbench/readpath/redb_report.py
out="$PWD/target/redb-data/reproduction/raw"

taskset -c 12-21 nice -n 10 cargo fmt --manifest-path "$crate" --check
taskset -c 12-21 nice -n 10 cargo test --manifest-path "$crate" --release --locked --features remix
taskset -c 12-21 nice -n 10 cargo clippy --manifest-path "$crate" --release --locked --features remix --all-targets -- -D warnings
taskset -c 12-21 nice -n 10 cargo build --manifest-path "$crate" --release --locked --features remix --bin kiban-readbench
taskset -c 12-21 nice -n 10 python3 "$driver" prepare --out "$out"
taskset -c 12-21 nice -n 10 python3 "$driver" screen --out "$out"
taskset -c 12-21 nice -n 10 python3 "$driver" select --out "$out"
taskset -c 12-21 nice -n 10 python3 "$driver" measure --out "$out"
taskset -c 12-21 nice -n 10 python3 "$driver" probe --out "$out"
taskset -c 12-21 nice -n 10 python3 "$report" "$out"
~~~

The frozen study rules are in [PROTOCOL.md](PROTOCOL.md). Build and test receipts, syscall counts, and crash logs are in [checks/](checks/). [TRIALS.csv](TRIALS.csv) retains the values for each round, including read and write tails within the mixed stage. [summary.json](summary.json) stores medians, ranges, per-round values, and paired ratios. [decisions.tsv](decisions.tsv) records the choices made during the study.
