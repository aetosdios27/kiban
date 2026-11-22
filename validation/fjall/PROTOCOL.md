# Kiban vs Fjall protocol

Frozen before screening. Compare Kiban's warm, single-worker keyed JSON memory API against a screened Fjall configuration, with raw LMDB as a same-round control. The outcomes are API latency, throughput, footprint, and acknowledgment guarantees. The experiment makes no prediction about which engine wins.

## Workload and oracle

Use seed 20261016, 64 agents, 200-byte ASCII text, immutable creation sequence, and the shared JSON value codec. Every engine receives the same composite keys, `agent/NNNN/memory/<u64::MAX-id padded to 20 digits>`. Ascending byte order within the agent prefix is descending creation order. Fjall's forward bounded prefix iterator therefore implements latest-ten using the shared on-disk keys. Decode only the value from each Fjall guard. The workload repeats 64 distinct latest-ten slices.

Read fixtures contain 1M records. Each command runs 100k calls per operation, recall-by-id and latest-ten. Kiban bulk uses batch preloading and a final flush. Fjall bulk writes sorted records via `Keyspace::start_ingestion`. Normal loads issue individual ID-order puts. Kiban normal flushes at its default write-buffer threshold and runs the normal compaction picker. Fjall normal syncs the journal, rotates the remaining memtable and waits for flush and active compaction. Keep the resulting natural table layout. LMDB uses the same ID-order, single-transaction loader in both layout groups.

Write commands start from a fresh private copy of the 10k bulk fixture. Run 1k calls each of remember, recall-by-id, latest-ten, update and mixed. Mixed contains 900 reads and 100 updates. Each command performs exactly 2,100 mutations. The short stages expose call and persistence costs. They do not target sustained compaction throughput.

Run the full deterministic point and all-agent latest-ten oracle on reopen after fixture preparation, before and after each measured command, and again after closing and reopening every write result. Check every timed read result and write readback outside the call timer. Any failed check or failed command invalidates the run. Preserve incomplete evidence and do not selectively rerun trials. Read fixture table identities and sizes must remain fixed. Completed receipts may be reused when their binary, source, configuration and evidence hashes match.

## Complete candidates

Use `fjall = 3.1.13` with `lsm-tree = 3.1.10`. Both crates are Rust implementations. Fjall's LZ4 support uses Rust `lz4_flex 0.13.1`. Fjall's source commit is `cfcdce3ef8425c1877e45e9b3ab88ed667132d92` and its crate checksum is `6e98f7212d748b723dcfbd43d5f166958945ad2c05ad4537fb99bcda42ad8a96`. The lsm-tree source commit is `9812163ae144213b6bb576a147fda066e7ab4ab1` and its crate checksum is `5498808f4d41cf785079d3ef3f28716f1b4364af1b512f3097bfb651bded1b3e`. [Fjall release](https://docs.rs/crate/fjall/3.1.13) · [lsm-tree release](https://docs.rs/crate/lsm-tree/3.1.10)

| Candidate | Data block KiB | Whole-key Bloom bits/key | Memtable MiB |
|---|---:|---:|---:|
| block16 | 16 | 10 | 64 |
| block4 | 4 | 10 | 64 |
| bloom6 | 4 | 6 | 64 |
| mem16 | 4 | 10 | 16 |

Use one Fjall keyspace, one background worker, and default leveled compaction for all candidates. Its public API does not expose size-tiered compaction or a prefix extractor or prefix Bloom filter. FIFO can evict keys and is incompatible with complete-model verification. Screen with compression disabled for data blocks, index blocks and the journal. Keep other defaults, including filter/index placement. Separate LZ4 rows enable data-block compression at every level and keep the corresponding winner's other settings. LZ4 does not enter selection.

Use `KibanOptions::agent_memory(18)` for the indexed Kiban row and disable both point and scan indexes for the index-off row. Both Kiban rows use `prefix_values` through the readpath harness's `remix` feature. Keep the normal Kiban cache, mmap and filter settings, except where the explicit cache group changes cache size. Match the two LSMs' block-cache capacity in bytes. Read groups use both 512 MiB and the same runtime Kiban default. Write groups use the default budget, as in the RocksDB study. LMDB uses its existing 10 GiB virtual map and OS-managed mapped pages.

## Acknowledgments

| Engine | Durable | Fast |
|---|---|---|
| Kiban | Put, flush buffered WAL and sync data before return | Put into the buffered WAL and memtable |
| Fjall | Insert, then `Database::persist(PersistMode::SyncData)` before return | Insert with `manual_journal_persist(true)` and leave the journal buffered |
| LMDB | One synchronous committed transaction per mutation | One committed transaction per mutation with `NO_SYNC` and `NO_META_SYNC` |

Fjall fast and Kiban fast both allow acknowledged writes to remain in userspace. Neither promises survival after a process crash. Their flush triggers and recovery boundaries may differ. Final sync and verification are outside call timers. Compare the per-command fdatasync counts from the existing preload probe for durable and fast writes after primary measurements. The durable command must add 2,100 calls and all syncs must succeed. In a separate private copy, kill Fjall after 25 acknowledged durable inserts and verify the entire 10,025-record model after reopen. This tests process death on a running OS.

## Selection, scheduling and reporting

For each family, minimize the equal-workload geometric mean of five-trial medians of mean call time. Read scores include both operations, both layouts and both budgets. Write scores include remember, update and mixed in both modes. Select one whole candidate per family. Break exact ties by label. Archive all losing measurements and commit the winners before the first primary command.

Screen with five rotated rounds per scenario and an LMDB command in every round. Primary rounds each contain Kiban index off, Kiban index on, Fjall tuned, Fjall LZ4 and LMDB. Rotate the five commands so each has each position once. Pair ratios by round before taking medians. The per-call timer includes key construction, serialization/deserialization, owned result construction and requested write durability. A returned Memory drops after the timer. Verified throughput includes the per-stage oracle and write readbacks. Open, pre/post verification and final persistence are outside both stage metrics. Histogram percentiles are upper bucket bounds.

Pin fixture preparation, benchmark processes and diagnostics to CPU 6. Assign no task work to its SMT sibling CPU 7. Record CPU 6 and 7 tick deltas and host load. This is a shared host without exclusive affinity reservations. Pin builds, tests and coordinators to CPUs 12–21 at nice +10. Set `CARGO_BUILD_JOBS=2`. Finish builds and tests before timing. Use a single release binary from `target/fjall-h2h` and store fixtures in `target/fjall-data`. Keep diagnostics separate from primary timings.

Report median and range for p50, p99, mean-call throughput, verified throughput, logical and allocated bytes, and whole-process peak RSS. Report same-round Kiban ratios against both Fjall and LMDB. Record fixture table counts, runtime settings, command receipts, exact versions, assumptions and screening losses. Full verification warms the process and host caches. These results cover the measured warm API, the given hardware and the tested candidate set.

## 2026-11-22 preparation deviation

Before screening, normal 4 KiB Fjall preparation stalled: the sole worker and the caller both blocked sending a flush on the full bounded worker queue. The backtraces and first preparation attempt are retained. To keep one worker, the normal loader now pauses at the configured memtable limit minus 1 KiB of record headroom and rotates the memtable synchronously. The ordinary compaction picker still runs. Timed puts and sorted ingestion retain their original paths. Rebuild, rerun the gates, and prepare every fixture from scratch. No candidate had been screened when the loader changed. [Backtraces](checks/preparation-stall/backtraces.log)

## 2026-11-22 Deviation 1: supplemental read-knob screen

The initial Fjall screen omitted data-block hash indexing and index/filter pinning, although the RocksDB study screened equivalent features. Fjall's latest-ten p50 was about 6.1 µs against LMDB's 2.2 µs. The supplemental screen tests whether the omitted settings improve Fjall on this workload.

Use the frozen `block4` baseline (4 KiB data blocks, 10-bit Bloom, 64 MiB memtable, leveled compaction, and compression off). Each row changes the listed settings together.

| Candidate | Hash ratio 0.75 at all levels | Pin filter and index blocks at all levels | Expect point read hits | Data restart interval 8 at all levels |
|---|---|---|---|---|
| a-hash | Yes | | | |
| b-pin | | Yes | | |
| c-hit | | | Yes | |
| d-restart | | | | Yes |
| e-hash-pin | Yes | Yes | | |
| f-hash-pin-hit | Yes | Yes | Yes | |
| g-hash-pin-hit-restart | Yes | Yes | Yes | Yes |

Every timed recall is a hit. Keep the default index restart interval and partitioning policies. Rebuild fixtures from the same seed and loader for each candidate, including `block4`. Run `block4` and the seven new candidates in label order with LMDB last before rotation. Keep the original five rotated rounds per scenario, fixtures, oracles, CPU placement, and the frozen selection rule and scoring. Compare the new candidates against the contemporaneous `block4` scores. If either winner changes, archive the original primary and rerun all five primary rounds and both durability checks. Use the selected read configuration for the LZ4 companion in all scenarios. The uncompressed row still uses the respective read and write winners.

After timing, trace one durable write command per engine for Kiban with indexes on and Fjall's selected write configuration. Record the write, positioned write, and sync calls and their bytes. Attribute journal traffic to the 2,100 verified mutations and distinguish it from setup, flush, and output.
