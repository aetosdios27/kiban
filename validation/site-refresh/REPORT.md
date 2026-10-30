# Website benchmark refresh

**Result:** at one commit, on the agent-memory workload, kiban leads every warm read
against tuned RocksDB and SQLite with mmap. It leads LMDB except latest-ten at a 32 MiB
cache. Durable writes tie RocksDB, and group commit ties RocksDB at 16 writers. RocksDB
still finishes both long write runs about three times sooner and uses less memory and disk.

These runs measure `3935977707444665a083ab2e6f9a7a914ea52d1b` with `KibanOptions::agent_memory`-equivalent settings
(point index on, `scan_prefix_len = 18`). The earlier reports stay valid for their own
revisions. Numbers here are not comparable to them across host intervals.

## Method

- Reads: 1M records, 100k calls per stage, five rotated trials, CPU 10 only.
  `campaign.py` reuses the runner in `tools/agentbench/readpath/rocksdb_bench.py`, with
  full-value verification, full-population checks before and after, and receipts.
- Durable writes: 10k-record base, 1k calls per stage, five rotated trials, reopened and verified.
- Engines: kiban, RocksDB 11.8.1 with the frozen winner in `validation/rocksdb/raw/selection.json`
  (Ribbon, 16 KiB blocks, total-order seek), bundled SQLite (rowid, WAL, 1 GiB mmap for
  reads, `synchronous=FULL` for durable writes), and the existing LMDB control.
- Ratios are medians of same-trial ratios. Above 1 favors kiban. `min` and `max` are the
  trial range. A range that spans 1 is reported as even.
- Group commit: `group_commit.py`, the published matrix, 100 verified runs on CPUs 0,1,2,5.
- Long runs: `tools/compare/plans/scale.json`, three trials per engine, CPUs 0-11. The
  plan's `kiban` pin moved to this commit through `compare-main.patch`. All 18 runs are valid.
- Sync audit: `raw/sync-probe`. kiban 2,102 fdatasync, RocksDB 2,107 fdatasync, LMDB 2,101
  fdatasync, SQLite 2,113 fsync.

## Scoreboard

p50 and p99 in µs, or writes/s for the writer row. Normal layout, default cache unless noted.

| Row | kiban | RocksDB | ratio [range] | SQLite | ratio [range] | LMDB | ratio [range] |
|---|---:|---:|---:|---:|---:|---:|---:|
| point read | 0.96 / 1.41 | 2.50 / 4.35 | 2.6 [2.364, 2.644] | 1.82 / 2.62 | 1.932 [1.727, 1.933] | 1.25 / 1.76 | 1.3 [1.182, 1.356] |
| latest ten | 1.89 / 3.14 | 7.81 / 11.26 | 4.136 [4.068, 4.136] | 8.96 / 12.03 | 4.746 [4.61, 4.746] | 2.30 / 3.84 | 1.22 [1.22, 1.254] |
| point read, small cache | 1.15 / 2.24 | 5.38 / 9.22 | 4.667 [4.556, 4.686] | 1.82 / 2.62 | 1.611 [1.556, 1.629] | 1.25 / 2.18 | 1.083 [1.083, 1.114] |
| latest ten, small cache | 3.33 / 5.50 | 7.81 / 11.01 | 2.346 [2.259, 2.385] | 8.96 / 12.29 | 2.692 [2.593, 2.692] | 2.30 / 3.84 | 0.692 [0.667, 0.698] |
| new record | 516 / 1,409 | 541 / 1,278 | 1.0 [0.846, 1.048] | 623 / 1,507 | 1.176 [1.026, 1.213] | 1,114 / 2,490 | 2.159 [1.949, 2.23] |
| update | 524 / 1,278 | 541 / 2,949 | 0.938 [0.897, 1.079] | 516 / 1,409 | 1.0 [0.829, 1.029] | 1,147 / 2,294 | 2.188 [1.854, 2.222] |
| 16 concurrent writers | 11,815 / 2,632 | 12,702 / 2,827 | 1.076 [0.493, 1.65] | 1,206 / 299,718 | 10.537 [4.794, 16.735] | — | — |

## Writer sweep

Median writes/s, p99 µs, and syncs per 4,000 writes.

| Writers | kiban group | RocksDB | kiban ungrouped | SQLite |
|---:|---:|---:|---:|---:|
| 1 | 1,640 / 1,467 / 4,000 | 1,788 / 1,314 / 4,000 | 1,575 / 3,163 / 4,000 | 1,256 / 2,205 / 4,044 |
| 2 | 2,341 / 3,008 / 2,679 | 2,897 / 1,596 / 2,355 | 1,695 / 5,425 / 4,000 | 1,201 / 22,077 / 4,038 |
| 4 | 3,817 / 5,085 / 1,617 | 5,030 / 1,799 / 1,342 | 1,659 / 15,763 / 4,000 | 1,120 / 91,348 / 4,035 |
| 8 | 7,281 / 2,734 / 901 | 7,425 / 2,681 / 867 | 1,566 / 46,799 / 4,000 | 1,189 / 124,489 / 4,035 |
| 16 | 11,815 / 2,632 / 479 | 12,702 / 2,827 / 479 | 1,461 / 94,128 / 4,000 | 1,206 / 299,718 / 4,032 |

## Long runs

Median ops/s of three trials, with the range.

| Case | kiban | RocksDB |
|---|---:|---:|
| read-100m | 1,702,014 (1,690,541–1,740,310) | 267,466 (242,883–303,744) |
| updates-10m | 355,844 (349,125–570,348) | 1,024,521 (1,001,280–1,046,301) |
| write-10m | 78,364 (76,498–90,985) | 229,374 (191,395–249,189) |

## Resources

Peak RSS and logical disk in MiB, 1M records, normal layout.

| Engine | RSS | Disk |
|---|---:|---:|
| kiban | 451 | 352 |
| rocksdb | 356 | 303 |
| sqlite | 347 | 315 |
| lmdb | 839 | 799 |

## Deviations and limits

- RocksDB's screen was reused, not rerun. Its code did not change between campaigns.
- The host is a shared laptop on `balanced`, with `powersave` and turbo enabled.
- The system suspended from 15:54 to 15:58 during `read-bulk-32/r3-rocksdb`. That
  scenario's bulk RocksDB rows (about 155 µs) are excluded from the board. They reflect
  cache pressure on one large SST and would overstate the gap. The raw rows are retained.
- Two long-run attempts were discarded: one interrupted by suspend, one on battery,
  where the wall timeout stopped RocksDB. All accepted long runs ran on mains power.
- Group commit throughput is about double the `validation/group-commit` report for every
  engine, including SQLite and ungrouped kiban, so it reflects device sync time on this
  interval, not an engine change. Spread at 16 writers is wide: kiban 6,348–13,662 and
  RocksDB 5,496–12,877 writes/s.
- `read-100m` is lower than the v0.1.0 report on the same plan: kiban 1.70M vs 2.90M ops/s
  and RocksDB 267k vs 340k. Unpinned (1.64M) and physical-core (1.68M) kiban probes rule out
  CPU pinning. kiban's read p50 rose from 1.2 to 2.0 µs and its database from 153 to 182 MB.
  The cause is not isolated. The default point index under this plan's 33 MiB cache is a
  candidate, not a finding.
- Warm cache, one read worker, 200-byte text records. This says nothing about cold reads,
  many readers or data far larger than memory.
