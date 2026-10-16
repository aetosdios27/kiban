# Read-path pass and range cache (October 2026)

This pass reworked the foreground read path, ported HelixDB's sparse byte-range
cache as an optional tier beneath the block cache, and hardened two recovery
edges. Changes are uncommitted on `main`, on top of the audit repairs in
`AUDIT_REPAIRS.md`.

## Changes

- **Indexed leveled lookups.** Each published `Version` indexes every level above
  L0 by first key and verifies that the level is disjoint. A point lookup
  binary-searches one candidate table per level instead of scanning every table
  in the version. An overlapping level (never produced by recovery or compaction)
  falls back to probing every table, and a regression test covers that.
- **Per-level scan sources and range pruning.** Previously every scan source was
  one table iterator, and the merge filled every head eagerly. Creating a range
  read therefore read the first block at or after `start` in *every* table in the
  database. A disjoint level is now one lazily advancing source, and
  `range(start, end)` skips L0 and leveled tables that lie wholly outside
  `[start, end)`.
- **Block search without allocation; restart seeking.** Restart-point binary
  search compares borrowed keys instead of allocating one per probe. An
  iterator's first block seeks to the right restart interval instead of
  decoding from the start of the block.
- **Range cache tier (opt-in).** `src/range_cache.rs` ports HelixDB's
  `range-cache` 0.1.1 (Apache-2.0) to std-only Rust. It holds sparse,
  non-overlapping ranges per immutable SST, merges adjacent ranges and evicts
  least-recently-used whole ranges under a byte ceiling. Three deliberate
  differences from the original:
  - Inserts store only the uncovered gaps instead of replacing overlapped bytes.
  - Merging is capped at one read-ahead window. Unbounded merging re-copies the
    whole growing range on every sequential insert, which is quadratic.
  - Payloads are shared windows. Sequential iterators pin a window and serve its
    blocks with no lock and no syscall.

  Iterators read two blocks singly, then read ahead in windows that double from
  16 KiB up to `range_readahead_bytes`. Point lookups never use the tier.
  `KibanOptions::range_cache_bytes` defaults to 0, which disables it.
  `KibanStats::range_cache` reports hits, misses, source reads and bytes, and
  evictions. Ranges are invalidated when their SST is reclaimed.
- **WAL zero-filled tail.** A crash can commit a file's size before its data, so
  an unwritten tail reads back as zeros. An all-zero header frames "validly"
  (length 0, and CRC-32 of empty input is 0), and the empty payload then failed
  as semantic corruption, so the database refused to open. The writer never
  emits an empty payload, so a zero-only suffix from a frame boundary is now
  truncated as a torn tail. A zero run followed by any nonzero byte is still
  corruption, and the log is left untouched.
- **File-number allocation.** `freeze` now reserves its WAL number with
  `fetch_add` before creating the file, instead of `load` followed by a later
  `store`. Post-commit counter updates use `fetch_max`, so the allocator can
  never move backwards past a concurrent reservation. The existing gate
  discipline already excluded the race. This removes the reliance on it, and
  matches flush's rule that a failed attempt never reuses an identity.

## Benchmark

`benches/read_paths.rs` seeds 1,000,000 keys (14-byte keys, 100-byte values) in
a pseudo-random permutation through `SharedKiban`. The result is a compacted
L2 of 3 tables (9 MiB) and an L3 of 28 tables (110 MiB). The block cache is the
default 32 MiB. Every workload is read-only and uses the same seeded directory.
The baseline is a copy of the tree before this pass, running the same benchmark
source.

Host: 22 logical CPUs, 30 GiB RAM, on battery with the `powersave` governor. No
power or global cache setting was changed. Values are means of two interleaved
rounds, each the median of three samples, in ops/s.

- *Warm*: the database is on tmpfs, so every read is page-cache resident.
- *OS-cold*: the database is on btrfs on LUKS on NVMe. The SSTs are evicted with
  `posix_fadvise(DONTNEED)` before every sample. Kiban's own block cache stays
  warm across samples, so OS-cold samples vary widely; treat their direction,
  not their magnitude, as the signal.

### Read path: baseline vs this pass (range tier off)

| workload | warm baseline | warm new | Δ | OS-cold baseline | OS-cold new | Δ |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| get hit / 1t | 68,410 | 68,943 | +1% | 35,250 | 36,256 | +3% |
| get miss / 1t | 1,567,990 | 1,773,169 | +13% | 1,121,453 | 1,209,028 | +8% |
| scan 16 / 1t | 20,589 | 36,766 | +79% | 21,725 | 43,906 | +102% |
| scan 1000 / 1t | 1,592 | 2,128 | +34% | 2,596 | 4,178 | +61% |
| get hit / 8t | 316,313 | 346,545 | +10% | 135,132 | 138,466 | +2% |
| get miss / 8t | 2,930,060 | 3,130,153 | +7% | 2,345,583 | 3,215,704 | +37% |
| scan 16 / 8t | 73,148 | 132,429 | +81% | 128,014 | 250,233 | +95% |
| scan 1000 / 8t | 6,079 | 7,072 | +16% | 11,974 | 16,932 | +41% |
| full scan / 1t | 1,865,905 | 2,369,216 | +27% | 1,767,062 | 2,214,626 | +25% |

Scans gain the most, because they no longer read a block from every table.
Get-miss samples are short (100k–400k ops at over 1.5M ops/s), so their deltas
are noisy.

Point-hit lookups barely moved, so they were profiled separately. With the
whole dataset block-cached, one lookup costs about 3.6 µs:
- about 0.2 µs to locate the table and run its filter;
- about 0.45 µs of block-index binary search;
- about 0.95 µs for the block-cache hit;
- about 1.9 µs searching inside the block.

Cutting the restart interval from 16 to 4 left the in-block cost unchanged and
grew the files by 2.5%. The time goes to dependent cache misses on a random
4 KiB block in a 120 MB working set, not to decode work. Further gains need
layout work, for example a contiguous separator arena or a faster block-cache
hasher, so the restart interval stays at 16.

### Range tier: off vs on (warm, this pass)

| workload | tier off | tier 8 MiB | Δ | tier 32 MiB | Δ |
| --- | ---: | ---: | ---: | ---: | ---: |
| scan 16 / 1t | 36,155 | 36,144 | 0% | 37,924 | +5% |
| scan 1000 / 1t | 2,068 | 1,882 | −9% | 1,801 | −13% |
| scan 16 / 8t | 114,428 | 120,567 | +5% | 120,648 | +5% |
| scan 1000 / 8t | 6,370 | 5,310 | −17% | 5,139 | −19% |
| full scan / 1t | 2,371,157 | 2,407,237 | +2% | 2,336,658 | −1% |

Point lookups are omitted because they never touch the tier, so their spread
here is run-to-run noise.

Earlier versions measured worse:
- With unbounded merges and a lock and copy per block, 8-thread medium scans ran
  at about half the tier-off rate.
- With read-ahead starting on an iterator's second block, short scans lost
  10–15%.

Pinned windows and RocksDB-style delayed read-ahead removed those regressions.
What remains is structural. With buffered I/O, the kernel page cache already is
a range cache with its own sequential read-ahead. A user-space tier can only add
copies and window overshoot on top of it. That is why medium scans, which stop
partway through their last window, lose.

HelixDB built this structure for object storage, where every miss is a costly
remote request and nothing sits underneath. In OS-cold runs the tier showed no
consistent gain over the tier off either: kernel read-ahead already turns
sequential `pread`s into large device reads.

**Conclusion:** the tier ships disabled. It is worth enabling only where SST
reads bypass or defeat the page cache, for example under memory pressure that
evicts SST pages, or a future direct-I/O or remote backend. Re-measure with
`KIBAN_RANGE_CACHE_BYTES` in that setting before enabling it.

## Reproduce

```sh
cargo build --release --bench read_paths
B=$(ls target/release/deps/read_paths-* | grep -v '\.d$')
KIBAN_BENCH_DIR=/tmp/kiban-bench-db $B                                  # tier off
KIBAN_BENCH_DIR=/tmp/kiban-bench-db KIBAN_RANGE_CACHE_BYTES=8388608 $B  # tier on
cp -r /tmp/kiban-bench-db ~/.cache/kiban-bench-db
KIBAN_BENCH_DIR=~/.cache/kiban-bench-db KIBAN_BENCH_COLD=1 KIBAN_BENCH_GETS=20000 \
  KIBAN_BENCH_SHORT_SCANS=5000 KIBAN_BENCH_LONG_SCANS=200 $B             # OS-cold
```

## Validation

New tests:
- a randomized differential test over multi-block tables across several levels,
  with overwrites, deletes and a pinned snapshot, comparing random point lookups
  and bounded and unbounded ranges against a `BTreeMap` (tier on and off, direct
  and shared handles);
- the overlapping-level fallback;
- range pruning;
- read-ahead coalescing and rescan behaviour;
- a randomized byte-model test of the range cache, plus its unit tests;
- the WAL zero tail.

Test option helpers enable a small range tier, so the existing compaction and
snapshot suites also run through it.
