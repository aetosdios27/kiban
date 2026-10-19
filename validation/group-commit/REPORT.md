# Group commit for durable shared writes

**Result:** with concurrent durable writers, group commit raises Kiban's durable
write throughput from 718 to **5,379 writes/s at 16 writers (7.5×)** while issuing
**477 fdatasyncs instead of 4,000**. That matches RocksDB's own write groups
(5,595 writes/s, within 4%) and is ~8.6× SQLite WAL+FULL. A single writer is
unchanged (727 vs 671 writes/s, within noise). Every write is still synced before it
is acknowledged.

Design: `SharedKiban::put_durable`, `delete_durable` and `write_durable` queue behind
a leader (`src/group_commit.rs`). The leader appends every queued WAL record, issues
one fdatasync, publishes the writes to the memtables in queue order under one mirror
guard, then wakes the followers. Nothing is visible or acknowledged before the sync
succeeds. A failed append or sync poisons the engine and fails the whole group. A lone
writer leads its own group immediately, with no added delay. The `&mut Kiban` API is
unchanged.

## Method

`tools/agentbench/readpath/src/bin/group-commit-bench.rs`. `T` writer threads each
insert fresh JSON memories (the agent-memory model, seed 20261019), one
acknowledged-durable write at a time, into a 10k-record base database. Each run does
4,000 writes in total (`4000/T` per thread, at least 250). Every run closes and reopens
the database and verifies **every record byte for byte** (all 100 runs verified).

- **Kiban sync:** `put` followed by `sync()`, which is today's durable path on
  `SharedKiban`. **Kiban group:** `put_durable`.
- **SQLite:** bundled SQLite, WAL, `synchronous=FULL`, one connection behind a mutex.
- **RocksDB:** 11.8.1, `WriteOptions::sync=true`, with its own write groups.
- **LMDB:** not in the matrix. LMDB allows one write transaction at a time, so it
  cannot scale with writers. Its measured single-writer durable write was 1,212 µs
  (about 825 writes/s) in #16/#17. The harness's LMDB backend could not reopen its
  environment for the verification pass and was dropped rather than reported unverified.
- **fdatasync counts** come from an `LD_PRELOAD` probe
  (`group_commit_probe.c`) counting `fdatasync`, `fsync`, `msync` and
  `sync_file_range`; the table sums `fdatasync + fsync`.
- Timed processes pinned `taskset -c 0,1,2,5`. Five alternating trials, order flipped
  per trial and thread count, database copied fresh for each run. Medians below.
  Raw results are in `results/`, and the driver is `run_matrix.py`.

## Results (median of five trials)

| Writers | Engine | Writes/s | p50 µs | p99 µs | Syncs per 4,000 writes |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | Kiban sync | 671 | 1,301 | 6,756 | 4,000 |
| 1 | **Kiban group** | 727 | 1,299 | 3,130 | 4,000 |
| 1 | SQLite | 646 | 1,490 | 4,302 | 4,044 |
| 1 | RocksDB | 678 | 1,403 | 3,509 | 4,000 |
| 2 | Kiban sync | 789 | 1,729 | 18,799 | 4,000 |
| 2 | **Kiban group** | 1,082 | 1,687 | 4,150 | 2,759 |
| 2 | RocksDB | 1,042 | 1,691 | 4,899 | 2,508 |
| 4 | Kiban sync | 778 | 1,712 | 55,019 | 4,000 |
| 4 | **Kiban group** | 1,730 | 2,051 | 5,194 | 1,641 |
| 4 | RocksDB | 1,928 | 1,854 | 4,650 | 1,391 |
| 8 | Kiban sync | 759 | 1,797 | 112,256 | 4,000 |
| 8 | **Kiban group** | 3,103 | 2,111 | 5,531 | 911 |
| 8 | RocksDB | 3,070 | 2,512 | 4,508 | 879 |
| 16 | Kiban sync | 718 | 1,860 | 200,098 | 4,000 |
| 16 | **Kiban group** | 5,379 | 2,284 | 12,897 | 477 |
| 16 | SQLite | 627 | 1,537 | 621,933 | 4,032 |
| 16 | RocksDB | 5,595 | 2,767 | 5,312 | 479 |

(SQLite is flat at ~640 writes/s from 1 to 16 writers, with its p99 growing to 622 ms
as writers queue on one connection. Its 2/4/8-writer rows are in `results/`.)

## What the numbers say

- **Throughput scales with writers.** Kiban group goes 727 → 1,082 → 1,730 → 3,103 →
  5,379 writes/s. Per-write p50 grows only from 1.3 to 2.3 ms, and the fsync count falls
  from 4,000 to 477 (about 8.4 writes per sync at 16 writers).
- **The tail collapses.** Without grouping, p99 at 16 writers is 200 ms, because each
  writer queues for its own sync. With grouping it is 12.9 ms.
- **Parity with RocksDB, not a win.** RocksDB is 11% ahead at 4 writers and 4% ahead at 16;
  Kiban is 4% ahead at 2 writers and 1% ahead at 8. All of these are within trial variation. RocksDB's p99 is lower at 16
  writers (5.3 ms vs 12.9 ms). Kiban group matches but does not beat RocksDB's write
  groups.
- **Single-writer latency is unchanged**, as designed.

## Correctness evidence

Tests (`src/group_commit_tests.rs`, about 760 lines) cover grouping and wakeup, panic
safety, failed-sync sweeps, batch atomicity, and seeded concurrent crashes at every
group stage (before append, after append before sync, after sync before
acknowledgement, during memtable publication). A loom model covers the queue handoff.
Gate results are in `gates/` (see below). A mutation check that removed the sync-before-publish
ordering was caught by the tests and reverted.

## Limits

- This is a host with one NVMe behind LUKS, where an fdatasync costs about 1.3 ms.
  Faster devices change the absolute numbers and the group sizes.
- Single process, JSON values of ~250 bytes, 10k-record base.
- Only the shared API is grouped. The single-threaded `&mut Kiban` path is unchanged.
- Writers are saturating (no think time). Open-loop arrival rates were not measured.
