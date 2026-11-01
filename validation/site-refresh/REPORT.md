# Website benchmark refresh

**Result:** at one commit, on the agent-memory workload, kiban leads every warm read
against tuned RocksDB and SQLite with mmap. It leads LMDB except latest-ten at a 32 MiB
cache. Single-writer durable writes tie RocksDB, and group commit edges it at 16 writers. RocksDB
still finishes both long write-heavy runs about three times sooner and uses less memory and disk.

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
| point read | 1.09 / 1.57 | 2.75 / 4.61 | 2.581 [2.389, 2.706] | 1.92 / 2.75 | 1.871 [1.556, 1.967] | 1.34 / 1.95 | 1.258 [1.083, 1.41] |
| latest ten | 1.95 / 2.62 | 8.19 / 12.03 | 4.242 [4.066, 4.852] | 9.47 / 13.06 | 4.852 [4.485, 5.246] | 2.43 / 3.58 | 1.241 [1.182, 1.344] |
| point read, small cache | 1.25 / 2.24 | 6.53 / 11.52 | 5.158 [4.533, 5.368] | 1.92 / 2.50 | 1.5 [1.289, 1.641] | 1.34 / 1.82 | 1.053 [0.911, 1.237] |
| latest ten, small cache | 3.39 / 5.25 | 8.45 / 12.80 | 2.462 [2.357, 2.593] | 9.47 / 13.82 | 2.741 [2.642, 2.792] | 2.43 / 3.90 | 0.698 [0.685, 0.75] |
| new record | 492 / 2,032 | 500 / 1,737 | 0.984 [0.931, 1.052] | 541 / 2,228 | 1.111 [1.1, 1.138] | 1,049 / 2,949 | 2.159 [2.133, 2.172] |
| update | 500 / 1,704 | 483 / 1,540 | 1.0 [0.833, 1.065] | 492 / 1,507 | 1.0 [0.97, 1.065] | 1,081 / 2,949 | 2.169 [2.098, 2.258] |
| 16 concurrent writers | 14,101 / 3,001 | 12,830 / 2,710 | 1.095 [1.087, 1.33] | 1,451 / 279,986 | 9.892 [9.718, 10.254] | — | — |

## Writer sweep

Median writes/s, p99 µs, and syncs per 4,000 writes.

| Writers | kiban group | RocksDB | kiban ungrouped | SQLite |
|---:|---:|---:|---:|---:|
| 1 | 1,655 / 1,983 / 4,000 | 1,788 / 1,614 / 4,000 | 1,754 / 1,464 / 4,000 | 1,399 / 2,142 / 4,044 |
| 2 | 2,534 / 2,465 / 2,686 | 2,704 / 2,725 / 2,462 | 1,948 / 3,688 / 4,000 | 1,463 / 24,996 / 4,035 |
| 4 | 4,066 / 2,936 / 1,616 | 4,652 / 2,736 / 1,400 | 1,897 / 9,555 / 4,000 | 1,436 / 58,384 / 4,035 |
| 8 | 7,065 / 2,736 / 901 | 7,841 / 2,799 / 779 | 1,882 / 22,466 / 4,000 | 1,438 / 130,072 / 4,032 |
| 16 | 14,101 / 3,001 / 480 | 12,830 / 2,710 / 482 | 1,836 / 44,352 / 4,000 | 1,451 / 279,986 / 4,032 |

## Long runs

Median ops/s of three trials, with the range.

| Case | kiban | RocksDB |
|---|---:|---:|
| read-100m | 1,743,364 (1,619,380–1,755,896) | 233,765 (223,640–285,517) |
| updates-10m | 387,251 (371,227–602,969) | 1,025,096 (918,013–1,038,579) |
| write-10m | 83,711 (82,640–163,261) | 285,817 (247,390–289,857) |

## Resources

Peak RSS and logical disk in MiB, 1M records, normal layout.

| Engine | RSS | Disk |
|---|---:|---:|
| kiban | 450 | 352 |
| rocksdb | 356 | 304 |
| sqlite | 349 | 315 |
| lmdb | 839 | 799 |

## Point index cost at a small cache

`read-100m` is lower than the v0.1.0 report on the same plan: kiban about 1.74M vs 2.90M
ops/s. Mains power, the performance profile and CPU pinning do not change it. Rebuilding
`tools/compare` with `point_index: false` and running the plan three more times gives
3.71M–3.88M ops/s, p50 0.94–0.99 µs, and a 153 MB database, against 182 MB with the index.
Raw summaries are in `point-index-probe/`. The default-on index therefore costs this
33 MiB-cache workload more than half its throughput. The mechanism is not profiled.

## Deviations and limits

- All accepted results ran on mains power with the `performance` platform profile, on a
  shared laptop with a browser and video playback open.
- RocksDB's screen was reused, not rerun. Its code did not change between campaigns.
- The bulk 32 MiB RocksDB rows are excluded from the board. They reflect cache pressure on
  one large SST and would overstate the gap. The raw rows are retained.
- Earlier attempts were discarded: one interrupted by suspend, one on battery where the
  wall timeout stopped RocksDB, and one complete run on the `balanced` profile.
- The long runs have outlier trials: kiban `updates-10m` 602,969 and `write-10m` 163,261
  ops/s against two trials near 380k and 83k. They are retained; medians are used.
- Warm cache, one read worker, 200-byte text records. This says nothing about cold reads,
  many readers or data far larger than memory.
