# Read-path profile

Work is continuing on `perf/read-path`, based on main `082858f` (PR #10).
No push or PR is authorized. The default block cache remains 32 MiB.

## Codec prerequisite

Commit `3606fa0` fixes `BloomFilter::empty()` to allocate the same number of
bytes as the normal builder. The decoder and on-disk encoding are unchanged.
`empty_filter_roundtrips_through_encode_and_decode` failed before the change
with `None` in place of `Some(empty_filter)` and passes afterward. It also
checks that the decoded filter rejects empty, textual, and binary keys.
This constructor has no production SST caller, so this fix does not account
for any claimed read improvement.

[Before/after regression evidence and all commit gates](read-path-results/bloom-checks).
Both long crash suites passed 20,000 release seeds each, on P-cores, together
covering 160,000 simulated crashes. Tests, release tests, workspace Clippy,
formatting, both fixed corpora, loom, site facts, and the benchmark's separate
checks passed. No concurrency behavior changed.

## Measurement setup

The Helix-free driver reuses the existing W2/W3 API calls, model, verifier,
random sequence, histogram, LMDB backend, allocator, and release settings.
Its lockfile preserves the original dependencies, including `heed3 0.22.0`,
`lmdb-master3-sys 0.2.5`, `mimalloc 0.1.48`, `libmimalloc-sys 0.1.44`, and
`serde_json 1.0.143`. Builds/checks use CPUs 2–5. All latency and profile
workloads run on CPU 3. The independent E-core crash sweep is not modified.

The original bulk loader leaves one L0 SST at N=1M. A second loader performs
individual writes and flushes at the default write-buffer budget, allowing
`Kiban::flush` to run its normal compaction policy. Its final 1M topology is
one L1 table, six L2 tables, and 77 L3 tables. No compaction option was changed.
At N=10k both loading methods fit in one L0 SST. Both layouts are closed,
independently reopened, and fully verified before measurement.

Each read trial has 100,000 W2 and 100,000 W3 calls. Five trials alternate
Kiban and LMDB on the same core for each population, layout, and cache budget.
Every timed result is checked outside its timer; complete verification also
runs before and after timing. No timed read trial writes or changes topology.

The host rebooted between the initial stop and resumption. New measurements
under `baseline-resumed` are the comparison baseline. The original `baseline`
results are retained as historical evidence and are not mixed into comparisons.
An initial LMDB preload from a newer C library was rejected by the pinned
library with `MDB_VERSION_MISMATCH`; it was recreated. That aborted setup is
retained separately under `invalid-new-lmdb-format` and excluded.

Profile loops exclude the payload oracle, preload, warmup and histogram.
Perf FIFO control enables counters only around each isolated operation loop.
Counters include destruction of the returned owned objects. Flame graphs use
user call chains and DWARF unwinding. Full latency trials retain the original
harness timing boundary. The standalone microbenchmarks isolate components;
their overlapping costs must not be added.

See [reproduction instructions](../tools/agentbench/readpath/README.md).

## Phase 1: latency and counters

All latency cells below are microseconds, median of five trials. Each cell
lists Kiban / LMDB. Min/max ranges, all percentiles, and every raw trial are
retained in [the full baseline](read-path-results/BASELINE_RESUMED.md).

| N | Layout | Cache MiB | W2 p50 K / LMDB | W2 p99 K / LMDB | W3 p50 K / LMDB | W3 p99 K / LMDB |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 10000 | bulk | 32 | 0.944 / 0.864 | 1.728 / 1.472 | 3.968 / 3.136 | 7.936 / 5.760 |
| 10000 | bulk | 512 | 0.944 / 0.896 | 1.728 / 1.472 | 4.096 / 3.136 | 7.936 / 5.760 |
| 10000 | normal | 32 | 0.896 / 0.880 | 1.728 / 1.376 | 4.096 / 3.264 | 8.448 / 5.888 |
| 10000 | normal | 512 | 0.912 / 0.896 | 1.632 / 1.568 | 3.904 / 3.200 | 7.680 / 6.528 |
| 1000000 | bulk | 32 | 8.448 / 1.760 | 13.824 / 2.688 | 4.480 / 3.392 | 10.496 / 6.912 |
| 1000000 | bulk | 512 | 3.072 / 1.792 | 5.248 / 2.752 | 4.736 / 3.456 | 10.496 / 6.912 |
| 1000000 | normal | 32 | 9.216 / 1.888 | 16.128 / 2.944 | 5.760 / 3.584 | 12.544 / 7.296 |
| 1000000 | normal | 512 | 3.136 / 1.952 | 5.120 / 3.072 | 5.888 / 3.520 | 12.800 / 6.912 |

On this host the cache-hit W2 user-cycle gap is about 3,051 cycles for bulk
and 3,423 for normal, rather than assuming the earlier roughly 2,000-cycle
estimate. Bulk Kiban uses 8,504 cycles and 8,129 instructions per operation,
versus LMDB's 5,452 cycles and 7,889 instructions. IPC is 0.956 versus 1.447.
Normal uses 9,224 / 5,800 cycles and 9,481 / 7,889 instructions, IPC 1.028 /
1.360. Extra time is largely memory access and dependent search, not simply
many more instructions. Kiban bulk has 20.97 LLC-load misses per W2 operation
versus LMDB's 15.40. Its dTLB-load misses are lower, 0.42 versus 1.98.

W3 is different. At 512 MiB Kiban uses 51,865 instructions per bulk scan
versus LMDB's 42,342, and 59,278 instructions per normal scan. The normal
scan has 41.73 branch misses versus LMDB's 24.63 and more merge/source work.
Cache capacity does not fix that CPU work. Default-cache W2 user cycles rise
to 20,687 bulk and 22,293 normal, while W3 changes little.

[All counter events normalized per operation](read-path-results/COUNTERS.md)
include cycles, instructions, IPC, L1 and LLC load misses, branch misses and
dTLB load misses. Events ran 100% of their enabled interval, without
multiplex scaling. Host permissions allow user events only. Kernel work,
including pread, is present in wall latency but excluded from these cycles.

### Ranked cache-hit cost attribution

The table uses exclusive sampled instruction-pointer percentages multiplied
by the separate counter run's cycles/op. It is approximate attribution, not
a sum of component microbenchmark timings. Search includes Kiban's inlined
index/filter routing and byte comparisons, and LMDB's B-tree search. LMDB
has no equivalent application block cache; its OS cache is not claimed free.

| Category | Bulk K cycles/op | Bulk LMDB | Bulk extra | Normal K | Normal LMDB | Normal extra |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Kiban block-cache lookup/ownership | 1260 | 0 | 1260 | 1203 | 0 | 1203 |
| Kiban in-block entry decoding | 1054 | 0 | 1054 | 945 | 0 | 945 |
| Search, routing and byte comparison | 4674 | 3756 | 919 | 5014 | 3950 | 1065 |
| Heap allocation/free | 224 | 197 | 27 | 195 | 216 | -22 |
| JSON and text decoding | 706 | 791 | -85 | 764 | 799 | -36 |
| Other work | 588 | 710 | -123 | 1103 | 836 | 267 |

The extra Kiban-specific cache ownership/lookup and entry-header decoding,
plus more expensive separator/routing work, account for the residual. Heap
allocation is not the dominant point-read explanation. In the bulk point
sample, BlockCache::get is 14.35% self, header_at is 10.48%, memcmp is 28.92%,
and the inlined SST lookup closure is 26.05%. LMDB spends 27.81% in
mdb_page_search_root and 26.61% in mdb_node_search. Those are exclusive
symbols; their parent-inclusive percentages are not added again.

[Eight SVG flame graphs, text reports, and excerpts](read-path-results/SAMPLES.md)
cover W2 and W3 for both engines and both layouts at 512 MiB. DWARF often
could not unwind Rust callers. The graph converter restores every empty
chain's actual sampled IP and labels its missing callers, instead of dropping
those samples. Every decoded sample's period is accounted for. The original
partial graphs in profile-resumed are superseded by profile-complete-stacks.
The text reports retain the sampled self symbol even when callers are absent.

### Component evidence and experiments

The isolated million-operation loops measured: memtable miss 27.8 ns,
memtable hit 163.7 ns, Bloom lookup 71.0 ns, separator search 624.8 ns,
cache hit 489.3 ns, in-block seek 863.8 ns, value copy 34.8 ns, and JSON
materialization 266.3 ns. An uncontended read lock was 19.1 ns and a hot Arc
clone 16.3 ns. These hot lock/refcount numbers do not explain the complete
cold-metadata cache hit. The latter includes hashing, map access and ownership
of scattered blocks. A full linear block iterator was 808.2 ns, but also copies
entries; it is not an equivalent search implementation or evidence to replace
restart search with linear iteration.

Constructing a range costs 389 ns bulk / 471 ns normal. Producing its first
entry costs 862 / 1,862 ns, and producing ten costs 1,894 / 3,400 ns, before
JSON decoding. Source shows initial positioning allocates key/value Vecs for
entries discarded before the lower bound; normal must position three level
sources. Merge duplicate suppression also clones one key per emitted entry.

Three experiments guide the implementation order:

- Hardware CRC-32/IEEE reference: 3,063 ns [3,030, 3,082] down to 714 ns
  [706, 723], medians [min, max] over five alternating runs. Every real SST
  block matched its stored checksum. The reference is crc32fast 1.5.0,
  a benchmark-only dependency. No engine dependency has been added.
- Inline 38-byte separators: 453 ns [420, 460] down to 279 ns [270, 289]
  over five alternating runs. The parsed entry is one 64-byte cache line,
  with an allocation fallback for longer keys. This changes only RAM layout.
- A more complex radix directory: 422 ns [383, 522] versus 409 ns
  [388, 426]. The ranges overlap; do not integrate this prototype.

The component loops have different cache footprints from a complete read.
The inline and CRC experiment controls therefore use their own alternating
baselines rather than subtracting earlier microbenchmarks. Expected gains
below require confirmation in W2/W3. Raw trials are under
[crc-prototype](read-path-results/crc-prototype),
[inline-prototype](read-path-results/inline-prototype), and
[index-prototype](read-path-results/index-prototype).

### Ranked fixes to try

| Rank | Fix | Expected end-to-end gain | Evidence and limit |
| ---: | --- | --- | --- |
| 1 | Runtime-dispatched PCLMUL CRC-32/IEEE, scalar fallback | 1.5–2.3 µs per default-cache 1M W2; little on a warmed hit | 2.35 µs/block component saving; default working set is about ten times its cache. Preserve polynomial, streaming state, checksum bytes, and small-input behavior. |
| 2 | Position block iterators before materializing discarded entries | 0.1–0.6 µs per W3, larger for normal | First-entry cost and source inspection identify avoidable copies before the range start. Keep first equal-key version and corruption checks. |
| 3 | Inline short SST separators in a cache-line-sized entry | 0.1–0.2 µs per cache-hit point read | 174 ns component median saving; confirm both layouts and scan tails. No file encoding change. |
| 4 | Reduce source-construction allocations and reuse deduplication key storage | 0.1–0.3 µs per W3 | Construction is 389–471 ns; repeated per-entry key cloning is visible. Preserve merge ordering, snapshots and error behavior. |
| 5 | Borrow a resident block while holding the cache read guard | 0.02–0.2 µs per cache-hit W2 | Avoid Arc ownership traffic, but most cache cost is random metadata access. Longer guards require concurrency proof and mixed-hit checks. |

These are estimates, not promised improvements. Sharding a 19 ns uncontended
lock is not the first single-client fix. Index and filter data are already
resident. Restart-point binary search and SST range pruning already exist.
Cache hits already skip checksum verification. Value-copy removal alone is
only about 35 ns here and cannot close a roughly microsecond residual.

## Write baseline

Five alternating trials use the original W1–W5 plan on fresh copies of the
10k bulk database, with 1,000 calls per stage, in fast and durable modes.
Every trial warms and verifies the base and verifies all updates and final
state. Full distributions are in [the write baseline](read-path-results/WRITES_BASELINE.md).
This checks the direct WAL/memtable path; normal compaction and larger
populations remain covered by the seeded suites and the read-layout tests.

Fast Kiban W1/W4 and durable W1/W4 will be compared against this baseline
for each retained fix. The baseline already preserves Kiban's write lead.
No performance implementation is included in the profile commit.

## Proposals requiring a decision

- Default cache budget: consider min(RAM / 64, 512 MiB), retaining an explicit
  override. On this approximately 30 GiB machine that is about 480 MiB, enough
  for the roughly 309 MiB data-block working set. The 512 MiB baseline reduced
  bulk W2 p50 from 8.448 to 3.072 µs and normal from 9.216 to 3.136 µs,
  closing roughly 81–84% of the contemporaneous point gap. This consumes
  substantially more application memory and must respect containers and
  multi-database processes. The default is unchanged in this task.
- A shorter restart interval, per-entry offsets, or a different block search
  representation could reduce the dependent header walk (7.91 LLC-load
  misses/op in the isolated seek). A writer/block-layout change is reserved
  for your decision. Existing files would need compaction to benefit; more
  restart keys increase bytes and may affect writes. The current block format
  and builder are unchanged.
- A borrowed read/range API could remove returned key/value Vec copies.
  The point copy alone saves only about 35 ns; a ten-row range has many more
  copies. Lifetimes must pin cached blocks and snapshots through consumption.
  No API change is included, and an end-to-end gain is not claimed without
  a prototype against the same harness.

## Implementation results

### Accelerate IEEE checksums with PCLMUL

The checksum update now dispatches inputs of at least 128 bytes to an
x86_64 PCLMULQDQ/SSE4.1 implementation. Short inputs and other CPUs retain
slicing-by-eight. The optimized implementation is adapted from crc32fast
1.5.0, with its MIT notice retained under licenses. The engine adds no runtime
dependency. Public functions, unfinalized streaming state, polynomial,
checksums, SST format, and default cache size are unchanged.

The independent bitwise tests still pass across lengths, alignments and
streaming states. A new dispatched-versus-scalar test covers offsets 0–63,
every length 112–272, larger fold boundaries, and three initial states.
Existing SSTs written before this change were verified by every read trial.
No engine synchronization or shared state changed; the standard library CPU
feature detector supplies the dispatch guard. Existing loom models remain
applicable to the unchanged engine publication and locking paths.

At 1M and 32 MiB, W2 p50 bulk improved 8.448 → 5.120 µs and normal
9.216 → 5.504 µs. W2 p99 improved 13.824 → 7.552 µs and 16.128 →
9.216 µs. Paired Kiban/LMDB p50 ratios improved 4.784 → 3.154 bulk and
5.053 → 3.510 normal, with non-overlapping bulk trial ratio ranges.
Cache-hit and scan trial ranges overlap, so their raw decreases are not
attributed to CRC acceleration. LMDB's own timings also shifted.

[Complete before/after p50/p99 matrix, all populations/layouts/cache budgets,
and paired control ranges](read-path-results/CRC_COMPARISON.md).
[After distributions](read-path-results/AFTER_CRC.md) preserve all five trials.

The write check found no regression. Fast W1 p50 was 1.152 → 0.816 µs
and W4 1.856 → 0.928 µs; call throughput 536,892 → 693,545 and
378,534 → 815,254 ops/s. Durable W1/W4 p50 was 589.824 → 557.056 µs.
Durable throughput ranges overlap, as expected for fsync. No durable speedup
is claimed. [All write p50/p99 and throughput ranges, including LMDB controls](read-path-results/CRC_WRITE_COMPARISON.md).

[Correctness gates and exact tested source hashes](read-path-results/crc-checks)
are retained with the change. The fix is retained for its clear cache-miss
read improvement; Kiban still trails LMDB on reads, so work continues.
