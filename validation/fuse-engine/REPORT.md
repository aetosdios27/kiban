# Integrated binary fuse SST filters

Status: measurements running; numerical summary is finalized only after all
paired receipts and maintenance checks pass. Implementation and correctness
gates are complete.

## Design and compatibility

`KibanOptions::binary_fuse_filter` is an opt-in writer switch (default false).
It selects a dependency-free, four-wise binary fuse8 filter with XXH3-64,
seed zero and the standard secret. `src/xxh3.rs` ports the BSD-2 xxHash v0.8.3
reference, including long inputs. `src/sstable/filter.rs` ports the supplied
experiment's peeling builder and FastFilter geometry (Apache-2.0). Both licenses
are retained under `licenses/`; no production Cargo dependency was added.

Footer versions 3/4 keep the exact Bloom decoder, FNV hash and block type 0.
Versions 5/6 select tagged filter block type 1. Version 6 retains the existing
PH3 footer extension, feature flags and checksum. Every new filter has a
48-byte `KIBANFL2`, revision-1 header and the existing CRC trailer covering
header, payload and type. The reader checks CRC, supported algorithm/hash/width,
lengths and geometry once, before any lookup can reject a key. Legacy and new
SSTs coexist under one manifest, independent of the current writer switch.
Main's frozen reader rejects versions 5 and 6 (old-reader-reject-*.log).

The exact header layout is: magic at 0, revision at 8, algorithm at 9 (0
positive, 2 fuse8), hash at 10 (1 XXH3 for fuse), width at 11 (8 for fuse),
header length at 12 (48), distinct-key count at 16, graph seed at 24, segment
length at 32, starting-segment count at 36, fingerprint length at 40, payload
length at 44. Integer fields are little endian. The array length is
`(starting_segments + 3) * segment_length`; payload length equals array length.
Segment length is a power of two at most 262144. Graph hash is the exact
SplitMix finalizer of `xxh3(key).wrapping_add(seed)`; the low byte is the
fingerprint. A positive fallback has no payload and canonical zero fields after
the header length. Disabling the switch does not rewrite existing tables;
downgrading to an old reader requires rewriting new-format SSTs first.

The builder hashes each distinct sorted user key once, including tombstones and
retained versions. It detects equal hashes, bounds construction to 100 seed
attempts and a conservative 64 MiB graph-workspace budget, and encodes positive
fallback on collision, exhaustion or unsupported geometry/budget. Singletons
work normally; empty SSTs remain rejected as before. Positive fallback cannot
hide a key and its algorithm tag persists the fallback for inventory counting.
All measured candidate SSTs used fuse8, with no positive fallbacks.

Every constructed graph checks all retained input hashes in release. The
serialized/decoded filter is then verified against every source key by scanning
the finished sorted blocks, in release and debug, before the existing atomic
SST/manifest publication path. This deliberately adds source-scan cost rather
than relying on a release spot check. Source keys are not retained in a second
vector. PH3 fingerprint-first lookup, key/MVCC checks and admission/fallback
ordering are unchanged. Filter memory remains table-owned metadata outside the
block cache, exactly as Bloom metadata was; no cache-budget accounting was
relaxed. Graph workspace excludes the existing hash vector, whole SST buffer
and optional PH3 builder.

## Correctness evidence

All eleven checks in `gates-final/status.json` passed: debug/release workspace
tests, clippy with warnings denied, formatting, fixed crash corpora, both
20,000-seed suites, loom, site-facts, and readbench clippy/tests/formatting.
The seeded suites completed 40,000 seeds and 160,000 simulated crashes with
filter and PH3 formats varied independently and toggled on reopen. A focused
real-engine test also fails every publication syscall across flush and
compaction, with and without PH3, and compares recovered gets with scans.

Tests cover two million distinct members and require actual fuse graphs;
random, binary, shared-prefix and long keys; repeated versions and tombstones;
snapshots; mixed formats; denied PH3 admission; empty/singleton behavior;
forced retries/budget/collision fallback; every filter header/payload/trailer
bit flip; truncation; and malformed headers with recomputed valid CRCs.

16,420 frozen vectors from upstream C cover every length 0..4097, long
stripe/block boundaries through 1 MiB, four binary patterns and unaligned
inputs. The independent `check-persisted.c` query reads actual SST bytes and
uses reference XXH3, checking all 1,010,000 members of the new 1M/10k bulk
fixtures with zero false negatives, plus the old Bloom controls. One million
absent queries per distribution per fixture give the counts in
`persisted-reference-check.jsonl`; these are filter FPR checks, not timed engine
miss calls. At 1M, fuse positives are 3900/1M uniform and 3896/1M prefix,
versus Bloom's 8334/1M for both distributions.

## Measurement protocol and limits

Baseline is fetched main `76a6a190`. The implementation is `55752393`; the
common absent-ID harness is `180aca0e`. `baseline-harness.patch` records the
same harness added to main, with only the two nonexistent new engine-option
assignments removed. Frozen binary hashes, toolchain, hardware and build
settings are in `provenance.json`. Engine source hashes before and after gates
match (`gates-final/*source-hashes.json`). No push, PR or subagent was used.

Own builds/tests ran on 12-21 at nice 10 and ended before diagnostic/timing
runs. Every timing child runs on CPU 8 and its affinity is checked. CPU 9's
busy ticks are recorded per whole process, including verification. Other host
work continued; the strict idle-sibling requirement was not met, so these
results cannot establish quiet-sibling tails or justify an unconditional default.
No source-level profiler is installed: `profile/` records CPU/IO accounting
around an excluded one-million-hit diagnostic phase. It establishes CPU-bound
warm lookup work, with about 897k cached file reads per million random hits at
32 MiB and effectively none at the default cache. It does not establish an
instruction-level hot-function breakdown.

Each bulk/normal × 32 MiB/default × index off/on × hit/latest or missing-ID
cell has five alternating before/after pairs, each side followed by LMDB. There
are 100k timed calls per stage, full million-record verification before/after,
per-call result checks, immutable read layouts, and raw per-process receipts.
The default cache is the actual production default, recorded in requested
configuration logs; it is not replaced with a nominal 512 MiB.

Absent IDs use the high half of the u64 domain, disjoint from the loaded
0..999999. Uniform spans the 64 agents in a balanced round-robin order with
pseudorandom remaining ID bits; prefix fixes agent 7. Missing byte keys are
prepared outside their per-call timer; the existing hit adapter formats its ID
key inside its call. These are before/after comparisons within each workload.
Range routing can reject misses before any filter call, especially on normal
layouts. No forced-filter throughput is inferred from engine timings.

The write suite uses fresh copies of a 10k population and 10k insert, recall,
latest-K, update and mixed stages (21k writes total), fast and durable, with
immediate readback and full reopen verification. Maintenance has two 500k-key
flushes and a forced L0 merge of one million binary keys with 296-byte values,
five pairs with PH3 off and on. Reported flush/build times include construction,
checksums, full-source filter verification, atomic output writes and reopening;
memtable fill and final full-member checks are excluded. Manifest commit is
reported separately. GNU time records each whole maintenance process's peak
RSS across both index states; it is not incremental filter workspace.

## Paired results, costs and default decision

Round one completed and validated every requested trial: `TABLES.md` and
`paired-summary.csv` contain median and full five-run ranges, paired total
call-time ratios, LMDB-relative ratios, maintenance costs and SST bytes. All
360 processes and ten maintenance processes succeeded; `analysis-check.json`
records the counts.

## Filter before point index on misses

Round-1 `TABLES.md` showed fused tables with a resident point index answering
absent keys in ~0.38 µs on bulk/default because the point index is probed
before any filter, while fused tables without a resident index answer in
~0.06 µs. Round two therefore reorders the reader: for tables carrying BOTH
`KibanOptions::point_index` and a fuse filter, `SstTable::get` consults the
fuse filter first — a fuse negative is a proof of absence (the peeling
construction has zero false negatives and the builder verifies every
published filter against all sorted source keys in release before atomic
publication, a check this round leaves untouched), so not-found returns
immediately; positive answers fall through to the unchanged point-index
path. Bloom tables, positive fallbacks (`SstFilter::Positive`) and tables
without the fuse filter keep the original ordering. The new test
`fuse_absent_answers_short_circuit_resident_point_index_and_keep_versioned_visibility`
in `src/fuse_filter_tests.rs` pins the configuration on disk (footer 6,
`KIBANFL2` algorithm 2) and checks absent answers at every limit plus
tombstone and multi-version/snapshot visibility.

Correctness gates all passed in one run
(`gates-fpi/status.json`, 11 checks code 0): fmt, clippy with warnings as
errors, workspace tests debug+release, fixed crash corpora, both
20,000-seed suites (40,000 seeds over `seeded_crash_long` and
`shared_seeded_crash_long`), loom, site-facts and readbench
clippy/tests/formatting.

### Measurement

Five alternating before/after paired trials with an LMDB control in every
trial, per `README.md`; only this round's requested cells ran, index on,
bulk and normal layouts, 32 MiB and default cache, workloads
absent-uniform, absent-agent-prefix, recall-by-id and recent-for-agent
(160 processes in `results2`, every process pinned CPUs-allowed=8,
success, errors 0, counts and full one-million-record verification
checked). The driver `fuse_miss_order.py` reuses `fuse_engine.execute`, so
the harness's assertion, affinity, sibling and receipt checks all apply;
`filter_first_report.py` validates the receipts and writes
`FILTER-FIRST-TABLES.md`. Before and after share the frozen after-format
fixtures byte-for-byte: the change is lookup-order only, the writer and
on-disk formats are untouched, and `provenance-filter-first.json` records
both binary hashes (`fd9e5aba…` before = the frozen round-1 build at
180aca0e; `1cc5a719…` after = 121e229c).

CPU 9 busy percentage over the timing processes: 9.028 [2.626, 58.195];
other host work continued, so the strict idle-sibling requirement from
round one is still not met and the same promotion caveat applies.

Kiban index-on cells (full range in `FILTER-FIRST-TABLES.md`):

| Layout | Cache MiB | Workload | Before p50 µs | After p50 µs | Before p99 µs | After p99 µs | Paired call-time ratio | Relative to paired LMDB |
|---|---|---|---:|---:|---:|---:|---:|---:|
| bulk | 32 | absent-agent-prefix | 0.055 [0.053, 0.058] | 0.057 [0.055, 0.059] | 0.244 [0.212, 0.252] | 0.272 [0.236, 0.296] | 1.064 [0.981, 1.215] | 1.029 [0.989, 1.236] |
| bulk | 32 | absent-uniform | 0.055 [0.053, 0.058] | 0.057 [0.055, 0.059] | 0.344 [0.328, 0.352] | 0.344 [0.336, 0.352] | 1.030 [1.008, 1.103] | 1.007 [0.969, 1.110] |
| bulk | 32 | recall-by-id | 4.608 [4.608, 4.608] | 4.608 [4.608, 4.992] | 7.168 [6.912, 7.552] | 7.168 [6.912, 8.448] | 1.001 [0.992, 1.104] | 1.027 [0.976, 1.117] |
| bulk | 32 | recent-for-agent | 3.904 [3.840, 3.968] | 3.712 [3.584, 3.712] | 7.296 [6.400, 8.192] | 6.784 [6.784, 7.296] | 0.956 [0.915, 0.968] | 1.021 [0.974, 1.103] |
| bulk | default | absent-agent-prefix | 0.376 [0.368, 0.376] | 0.066 [0.060, 0.132] | 0.528 [0.512, 0.544] | 0.336 [0.304, 0.456] | 0.274 [0.237, 0.515] | 0.323 [0.229, 0.478] |
| bulk | default | absent-uniform | 0.368 [0.360, 0.384] | 0.074 [0.061, 0.244] | 0.528 [0.504, 0.544] | 0.400 [0.352, 0.496] | 0.350 [0.324, 0.644] | 0.417 [0.307, 0.527] |
| bulk | default | recall-by-id | 1.088 [1.024, 1.120] | 1.248 [1.216, 1.248] | 1.568 [1.440, 1.664] | 1.760 [1.664, 1.920] | 1.133 [1.072, 1.180] | 1.205 [1.135, 1.346] |
| bulk | default | recent-for-agent | 4.224 [3.968, 4.352] | 3.968 [3.648, 4.032] | 8.192 [7.552, 9.216] | 8.064 [6.784, 8.960] | 0.925 [0.862, 1.095] | 1.025 [0.886, 1.213] |
| normal | 32 | absent-agent-prefix | 0.304 [0.288, 0.320] | 0.272 [0.256, 0.304] | 0.472 [0.448, 0.656] | 0.656 [0.480, 0.656] | 0.891 [0.804, 1.060] | 0.934 [0.828, 1.170] |
| normal | 32 | absent-uniform | 0.320 [0.312, 0.368] | 0.280 [0.272, 0.312] | 0.784 [0.688, 0.816] | 0.672 [0.576, 0.800] | 0.893 [0.768, 1.107] | 0.853 [0.729, 0.969] |
| normal | 32 | recall-by-id | 4.992 [4.480, 5.120] | 4.864 [4.224, 5.248] | 7.552 [6.272, 8.064] | 7.552 [7.168, 8.448] | 0.992 [0.943, 1.127] | 0.959 [0.928, 1.147] |
| normal | 32 | recent-for-agent | 4.864 [4.480, 5.248] | 4.992 [4.352, 5.248] | 10.240 [8.064, 12.800] | 10.496 [8.448, 12.032] | 0.945 [0.861, 1.366] | 1.127 [0.825, 1.407] |
| normal | default | absent-agent-prefix | 0.328 [0.320, 0.360] | 0.252 [0.248, 0.256] | 0.576 [0.480, 0.784] | 0.424 [0.424, 0.560] | 0.756 [0.632, 0.782] | 0.737 [0.638, 0.775] |
| normal | default | absent-uniform | 0.608 [0.608, 0.640] | 0.264 [0.264, 0.272] | 0.880 [0.880, 0.992] | 0.544 [0.544, 0.656] | 0.473 [0.416, 0.499] | 0.455 [0.438, 0.494] |
| normal | default | recall-by-id | 1.376 [1.344, 1.408] | 1.408 [1.408, 1.504] | 2.016 [1.856, 2.176] | 2.016 [1.984, 2.176] | 1.025 [0.997, 1.057] | 1.035 [0.958, 1.049] |
| normal | default | recent-for-agent | 4.480 [4.224, 4.864] | 4.224 [4.096, 4.608] | 8.448 [8.064, 9.728] | 7.808 [7.168, 9.216] | 0.954 [0.915, 1.023] | 0.996 [0.909, 1.052] |

### Hit cost and verdict

- Absent-ID misses where the point index is resident drop from
  0.37–0.61 µs to 0.25–0.27 µs on normal/default and to 0.066–0.074 µs
  on bulk/default — indistinguishable from the round-1 index-off cost
  (~0.06 µs), with LMDB-adjusted paired ratios 0.32–0.46. The bulk/32
  cell is unchanged (~1.03, within the paired LMDB band) because a
  189 MB resident index cannot be admitted under a 32 MiB cache, so
  both binaries already ran the filter-only path there.
- Point hits do not regress where the resident index is absent
  (bulk/32 paired 1.00/1.06) and recent-for-agent stays at 0.93–0.96 in
  every cell. normal/default recall-by-id moves 1.025 [0.997, 1.057]
  paired, 1.035 LMDB-adjusted (p50 1.376 → 1.408 µs) — within noise.
- Point hits DO regress on one cell: bulk/default recall-by-id
  1.088 → 1.248 µs p50 (paired 1.133 [1.072, 1.180]; LMDB-adjusted
  1.205 [1.135, 1.346]) while the paired LMDB control for the same
  workload sat at 0.973 [0.870, 0.997]. This exceeds the noise band and
  is measured, not claimed. The extra fuse probe (XXH3 plus four
  fingerprint positions) is the only added work on that path; a
  subsequent run should confirm the size of the cost before any
  unconditional adoption.

Verdict: the shortcut does exactly what it claims — resident-index
tables now answer absent keys at the pure filter cost — and point-hit
cost is unchanged at 32 MiB and on normal/default; recent-for-agent is
unchanged everywhere. Bulk/default point hits pay ~+0.16 µs p50
(+13 %). Net effect on a workload is a win whenever absent lookups
carry more than roughly a third of the traffic on those tables;
present-heavy pure-recall workloads on large resident bulk tables pay
the measured hit cost instead. The engine change stays in the tree with
the measurement recorded; a follow-up may want to shave the hit-path
probe before recommending an unconditional default for
`binary_fuse_filter`+`point_index` writer combinations.

