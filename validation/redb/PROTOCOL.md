# Kiban and redb head-to-head protocol

Compare the warm, single-worker agent-memory API on Kiban, tuned redb, and raw LMDB. Look for differences in point reads, short latest-ten reads, individual transactions, and resource use. Use the same payload and operation sequence for all engines.

## Workload and verification

Use seed 20261016, 64 agents, 200 bytes of ASCII text, and JSON values containing identity, agent, immutable creation sequence, revision, and text. The primary key has the form agent/NNNN/memory/MAX-minus-ID, with the last component padded to 20 digits. Creation order determines the lookup key without a second index. redb stores identical key bytes with a descending byte comparator and uses a bounded reverse range for latest-ten. Its internal separators retain the shortest prefix that preserves the ordering between adjacent keys. LMDB uses a forward prefix iterator. All engines return owned, deserialized records.

Each 1M-record read fixture runs 100k recall-by-ID and 100k latest-ten calls. redb bulk uses one write transaction in ID order. redb normal uses one non-durable committed write transaction per record in ID order, then a durable checkpoint. The normal fixture keeps its organic layout without manual compaction. Record page fragmentation, logical file length, and allocated bytes. Kiban uses the existing bulk and normal loaders. LMDB uses the same single-transaction layout for both comparisons because its control uses the original ID-order bulk loader.

Each write command starts from a fresh copy of a 10k bulk fixture. Run 1k calls in each stage in order: remember, recall, recent, update, and mixed. Mixed contains 900 point reads and 100 updates. Each of the 2,100 mutations gets its own commit. Compute the mutation's payload outside the timer. Include key construction, encoding, database calls, decoding, and returned result construction in the per-call timer. Drop the returned payload and check every result outside that timer. Verify the full expected population and latest-ten slices before and after each command. Check every write by immediate readback, then close, reopen, and verify the final model again. Any correctness failure invalidates the campaign.

## Engine settings and durability

Use one Rust release binary, with the readpath remix feature for Kiban. The indexed row is equivalent to KibanOptions::agent_memory(18). Turn off both the point index and its dependent scan extension in the second Kiban row. Use the profile's default filter and mmap settings. Set redb's builder cache budget to the exact Kiban default bytes or 512 MiB for the two read groups. Keep the default budget for write groups. redb retains the default exclusive-writer concurrency mode. LMDB maps a 10 GiB virtual address range and has no engine cache budget to match.

| Engine | Durable call | Fast call | End of write command |
|---|---|---|---|
| Kiban | Put, then WAL sync before acknowledgment | Buffered WAL. A process kill can lose acknowledged writes. | Sync and flush |
| redb 5.0.0 | Durability::Immediate before returning the committed transaction. Candidate quick repair adds a second phase and saves allocator state. | Durability::None. The only faster level leaves acknowledged mutations without crash-safe persistence. Reopening after a crash can recover an earlier durable state. | An Immediate commit checkpoints any outstanding non-durable transactions. |
| Raw LMDB | One default synchronous write transaction | One committed transaction with MDB_NOSYNC and MDB_NOMETASYNC. No per-commit sync. | Force sync |

Kiban fast buffers WAL writes and may send them to the OS before the end of the command. redb None has no promise that an acknowledged transaction will persist until a subsequent Immediate commit. Neither fast configuration guarantees acknowledged writes survive process death. Use strace -f -c on separate write-suite copies and compare sync counts for all three engines. For redb, SIGKILL a durable writer after 32 insert acknowledgments and verify the entire updated population in a new process. Power interruption remains outside the test.

redb is pinned to =5.0.0, source 0d53424a7e0a38b2665e4d41f40bb6372da953ad, crate SHA-256 11175cc688ba1b9d3cec4fe582fd82cb97e2a82e28ccc8101b7b8030a8003280. redb vendors no C database library. The control uses heed3 = 0.22.0 and lmdb-master3-sys = 0.2.5, with vendored LMDB header version 0.9.90. [redb release and source](https://docs.rs/crate/redb/5.0.0) · [Durability contract](https://docs.rs/redb/5.0.0/redb/enum.Durability.html) · [Compaction modes](https://docs.rs/redb/5.0.0/redb/enum.CompactionMode.html) · [Write transaction options](https://docs.rs/redb/5.0.0/redb/struct.WriteTransaction.html)

## Screen and primary trials

Screen four redb configurations. Each uses the same cache-budget schedule and fast and durable mapping above. Balanced compaction moves pages and truncates free tail space. Maximum also packs partially filled pages. Screen quick repair as a durable-write option with the balanced bulk fixture. It changes commit work and recovery cost, so the read commands for both balanced labels use the same prepared bytes. The normal read fixture is shared by all four labels to preserve the as-is layout.

| Candidate | Optional bulk post-load compaction | Quick repair on durable mutations |
|---|---|---|
| as-is | None | Off |
| balanced | Balanced | Off |
| maximum | Maximum | Off |
| balanced-quick-repair | Balanced | On |

Use five rotated trials per scenario and keep the LMDB control in every round. Choose a read winner and a write winner separately, before primary measurement. For each configuration, compute the median of the five mean call times in each scored stage, then take the geometric mean with equal weight per stage. Score both read operations across the two layouts and both cache budgets. Score remember, update, and mixed across both write modes. Minimize the resulting score and break exact ties by label order. Keep every screen result. Re-measure the frozen winners in independent five-round primary trials with Kiban index on, Kiban index off, and LMDB in every round.

## Host and reporting

Pin preparation, measured commands, and diagnostic processes to CPU 8. Assign no study work to SMT sibling CPU 9 and record its tick deltas and host load. Build, test, and coordinate on CPUs 12–21 at nice +10, with two Cargo jobs. Finish builds and tests before timing. Keep diagnostics separate from primary measurement. Use target/redb-h2h for Cargo artifacts and target/redb-data for disposable fixtures.

Report per-call p50 and p99 upper histogram bucket bounds, mean-call throughput, verified stage throughput, five-trial median and range, and same-round ratios against redb and LMDB. Report logical bytes, allocated bytes using st_blocks × 512, and whole-process peak RSS including oracle checks and reopening. Cache budgets match input byte limits. Actual RSS, file layouts, and the OS page cache vary. The complete pre-timing verification warms each process. Record host sharing and sibling activity alongside the results.
