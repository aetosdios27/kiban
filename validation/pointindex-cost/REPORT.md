# Point-index read-latency follow-up

The retained change routes admitted point indexes before the Bloom filter.
Across five alternating PH2/candidate pairs per case, all eight point p50/p99
median paired ratios are below 1.03, both raw and LMDB-normalized. Raw paired
medians range from 0.872 to 0.956. SST overhead remains +9.43% bulk and +9.59%
normal. The frozen PH3 encoding and reserved checksum bit 0 are unchanged.

The CPU-9 idle condition did not fully hold. Its measured activity and broad
individual-pair ranges qualify the result. These are passing observed paired
medians, not a controlled-host guarantee for every trial. The option remains
opt-in. The original PH3 cost campaign is preserved in
[PH3_COST_REPORT.md](PH3_COST_REPORT.md).

## Profile before changing the lookup

The independent evidence was read without modification and summarized using
`tools/agentbench/readpath/paired_report.py`:
[bulk](latency/independent-bulk.md), [normal](latency/independent-normal.md).
It reproduces the reported bulk p50 0.960 → 1.120 µs and normal p50
1.120 → 1.248 µs regression.

Counters and DWARF callchains cover only 3M warmed point hits on CPU 8. FIFO
controls exclude open, cache admission, corpus verification, and ID generation.
Each command verifies the corpus and checks returned IDs. Samples are diagnostic
runs, excluded from the paired latency matrix. Hardware user cycles at 499 Hz,
4,096-byte DWARF stacks, user-only callchains, and 32 mmap pages work on this
host. Initial software sampling and kernel-callchain attempts were denied.
Every retained recording reports zero lost samples.

| Layout/version | Cycles, billions | Instructions, billions | Branch misses, millions |
|---|---:|---:|---:|
| bulk PH2 | 12.234 | 16.923 | 2.319 |
| bulk original PH3 | 13.309 | 17.031 | 6.236 |
| bulk retained change | 11.044 | 16.062 | 6.113 |
| normal PH2 | 14.522 | 21.869 | 11.910 |
| normal original PH3 | 14.521 | 22.022 | 16.091 |
| normal retained change | 14.099 | 20.893 | 12.060 |

Original bulk PH3 needs 8.8% more cycles with 0.64% more instructions than PH2,
while cache misses fall from 74.20M to 66.40M. Branch misses rise 2.69×.
The retained change removes 5.7% of bulk instructions and 17.0% of bulk cycles
relative to original PH3. Normal instructions fall 5.1% and cycles fall 2.9%.
These single profile observations do not replace the paired comparisons.

The annotated hot lookup is inlined into `get_from_version_at::{closure#0}`.
In original bulk PH3, 51.64% of sampled self cycles are there. Within that
function, the pilot-load consumer accounts for 22.96%, the slot-fingerprint
consumer 24.79%, and the record shared/suffix-header load consumer 25.25%.
These are sample skid locations around dependent loads, not independent
per-operation cost estimates. The Bloom bit-load consumer is another 13.18%
of that function's samples.

The source and assembly establish the following facts:

- PH3 already compares anchor prefix and suffix as two slices. It does not
  reconstruct or allocate a full key. Fingerprints already precede record decode.
- Both PH2 and PH3 use u32 pilots. There is no u24 pilot decode to remove.
- The common PH3 width tuples already inline constant-width loads. The generic
  decoder remains for other valid widths. Bounds checks remain in the final code.
- The slot-to-record-to-anchor/value dependencies remain. PH3 has an extra key
  span and variable comparison lengths. About 64% of bulk prefixes exceed
  32 bytes, with most others at or below that boundary. This can add comparison
  branch pressure; counters alone do not assign each extra miss to a branch.
- The resident MPHF fingerprint and full-key check already reject misses.
  A preliminary Bloom hash and bit probes duplicate that rejection work.

The original annotated assembly and counters are
[before](latency/before-lookup-annotate.txt),
[after](latency/after-lookup-annotate.txt), and
[profile provenance](latency/profile-provenance.json).
The final and normal-layout reports are under [profiles](latency/profiles/).
Raw perf data remains in `target/pointindex-latency/`.

## Candidates and retained implementation

Only the direct resident-index path is retained. `SstTable::get` now calls an
admitted PH2 or PH3 index first. That index performs the existing fingerprint,
full-key, snapshot, and tombstone checks. The sorted fallback still uses Bloom.
Open-time section, backing-record, and Bloom integrity checks remain in force.
No allocation, cache charge, writer logic, or persisted field changes.

The complete four-case direct-path matrix below passes the requested point
median bounds. The final rebuilt read binary has exactly the same `.text`
SHA-256 as the measured candidate:
[comparison](latency/final-text-equivalence.json).

Load-time decoder dispatch was rejected. It selected monomorphized narrow
functions once per table and made the slot stride constant. Five alternating
pairs against original PH3 show bulk p50/p99 unchanged at 1.120/1.472 µs.
Normal rises from 1.280/1.728 to 1.376/1.824 µs, +7.5%/+5.6% by separate
medians. [Patch](latency/dispatch.patch), [bulk](latency/dispatch-bulk.md),
[normal](latency/dispatch-normal.md).

Record-line T0 prefetch after the fingerprint and slot-pointer load was also
rejected. In bulk, its paired p50/p99 medians against the direct path are
0.964/0.925. Normal paired medians are 1.000/1.000, with normalized p99 1.089.
It does not establish a consistent win across layouts. All five pairs per layout
are retained, including an LMDB p99 outlier of 12.288 µs.
[Patch](latency/prefetch.patch), [bulk](latency/prefetch-table.md),
[normal](latency/prefetch-normal-table.md).
A prematurely started PH2/prefetch confirmation was cancelled after inspecting
that normal paired result. Its partial receipts and reason are retained under
`latency/cancelled-prefetch-confirmation/`; they contribute to no reported table.

Storing a single contiguous full key or changing anchor grouping would require
new resident storage or a versioned encoding. The direct-path change meets the
observed target without either change. Two-slice comparison, fingerprint-first
decode, suffix lengths, u32 pilots, and the 1,024-key grouping remain intact.

## Paired PH2 comparison

The PH2 source is main `06a457db`. The candidate starts at `97735164` and changes
only the resident lookup order in production code. Both read executables were
built with the same unchanged release readbench profile and mimalloc. Frozen
hashes are in [binaries.json](latency/binaries.json), build logs remain under
`target/pointindex-latency/`, and each matrix records its exact binaries.

Each case contains five alternating before/after trials. Every Kiban command is
immediately followed by its own LMDB control. Each process verifies the 1M-record
corpus before and after 100k point plus 100k latest-K calls. The accepted matrix
contains 80 successful commands, 16M timed calls, and zero errors. Every process
reports CPU affinity 8. No builds or tests from this task overlap these timed runs.

All latency columns are separate run medians in µs. Paired columns are the
median and full range of five candidate/PH2 ratios. Normalized ratios divide
that per-trial ratio by the corresponding candidate-control/PH2-control LMDB
ratio. Ratios below one favor the candidate. Ranges are not confidence intervals.

| Case | Workload | p50 µs baseline → candidate | p99 µs baseline → candidate | Paired p50 [min,max] | Paired p99 [min,max] | LMDB-normalized p50 / p99 |
|---|---|---:|---:|---:|---:|---:|
| bulk-512 | Point | 1.056 → 1.024 | 1.408 → 1.344 | 0.943 [0.909,1.103] | 0.938 [0.896,1.077] | 0.921 / 0.955 |
| bulk-512 | Latest-K | 3.008 → 3.200 | 4.864 → 5.376 | 1.125 [1.061,1.225] | 1.176 [1.050,1.395] | 1.011 / 1.078 |
| bulk-default | Point | 1.088 → 0.976 | 1.440 → 1.376 | 0.934 [0.847,1.048] | 0.956 [0.804,1.222] | 1.010 / 0.956 |
| bulk-default | Latest-K | 3.008 → 3.264 | 5.760 → 5.632 | 1.041 [0.980,1.295] | 0.911 [0.667,1.579] | 1.076 / 1.163 |
| normal-512 | Point | 1.504 → 1.376 | 2.240 → 1.920 | 0.953 [0.750,0.979] | 0.924 [0.757,1.057] | 0.934 / 0.938 |
| normal-512 | Latest-K | 4.224 → 4.352 | 8.192 → 7.936 | 1.030 [0.941,1.082] | 0.969 [0.607,1.333] | 0.942 / 0.941 |
| normal-default | Point | 1.504 → 1.312 | 2.048 → 1.792 | 0.872 [0.848,0.898] | 0.897 [0.833,0.953] | 0.885 / 0.953 |
| normal-default | Latest-K | 4.032 → 4.352 | 6.912 → 6.784 | 1.065 [0.967,1.091] | 0.980 [0.852,1.593] | 1.018 / 1.543 |

Point call-throughput medians use the unchanged `call_ops_s` stage field,
computed from timed calls rather than the reciprocal of a histogram percentile:

| Case | Point call ops/s, PH2 → candidate |
|---|---:|
| bulk-512 | 950,212 → 968,376 |
| bulk-default | 919,834 → 1,002,369 |
| normal-512 | 660,713 → 720,869 |
| normal-default | 654,794 → 744,287 |

Point p50/p99 median ratios pass in all four cases. Individual pairs can exceed
3%, particularly in noisy bulk trials. Latest-K has no consistent improvement:
bulk/512 paired p50/p99 are 1.125/1.176, and normal/default normalized p99 is
1.543. This change does not establish latest-K tail non-regression. All raw
histograms, throughput values, controls, and process status remain in
[direct](latency/direct/), with derived values in
[summary.json](latency/direct/summary.json).

CPU 9 was never assigned work by this task. Its aggregate activity during each
command, including verification, is recorded in every `sibling.jsonl`:

| Case | CPU 9 busy %, median [min,max] |
|---|---:|
| bulk-512 | 1.32 [0.24,4.75] |
| bulk-default | 11.69 [3.66,17.03] |
| normal-512 | 3.30 [1.27,7.71] |
| normal-default | 1.77 [0.20,17.57] |

Unrelated desktop, browser, kernel, and other task activity remained on the host.
Foreign affinities were not changed. The sibling-idle condition cannot be
certified, and no trial was discarded for a slow latency or a busy sibling.

## Maintenance against current PH3

The baseline is `97735164`, with only the identical benchmark-trial selector
copied into its test driver. Each selected trial still runs both index-on and
index-off controls. Five version pairs alternate before/after order on CPU 8,
with `KIBAN_POINT_BENCH_N=1000000`. Both versions use the root release profile
and standard allocator. Per-thread CPU-tick probes run outside the elapsed
phase intervals; no perf sampler runs during these measurements.

| Index | Operation | Current PH3 → candidate, ms | Paired after/before [min,max] |
|---|---|---:|---:|
| off | Two flushes | 588.840 → 599.455 | 1.018 [0.923,1.087] |
| off | BUILD | 400.897 → 434.062 | 1.033 [1.003,1.188] |
| off | COMMIT | 17.663 → 14.337 | 0.917 [0.643,1.379] |
| on | Two flushes | 1254.173 → 1255.394 | 1.005 [0.943,1.014] |
| on | BUILD | 614.562 → 578.845 | 0.972 [0.942,1.075] |
| on | COMMIT | 28.823 → 26.077 | 0.954 [0.894,1.012] |

Indexed flush is +0.47% and BUILD -2.76% by paired medians. Flush variation spans
0.943–1.014 and BUILD spans 0.942–1.075. The result shows no material maintenance
regression. Indexed flush CPU medians are 1.26 → 1.24 s; BUILD is 0.60 → 0.58 s,
with 100 Hz tick quantization. Every run verifies all 1M records. All indexed
compactions produce exactly 347,833,741 bytes in both versions. CPU 9 busy time
is median 3.20%, range 0.57–10.27%, so this is also a qualified host comparison.
[Raw commands and logs](latency/maintenance/),
[derived values](latency/maintenance/summary.json),
[baseline source](latency/maintenance-baseline-source.json).

Cargo initially reused the archived baseline test artifact when target directories
were shared. Identical binary hashes exposed the problem before either binary
was used for maintenance or final gates. The candidate test executable and all
final gates use a fresh `target/pointindex-latency/verified-target` directory.
The two accepted maintenance executable hashes differ. The PH2 read binary has
no PH3 magic, its build log records compilation from the PH2 archive, and the
final read `.text` matches the accepted measured candidate. This closes the
artifact-integrity issue rather than accepting cached build claims.

## Bytes and frozen contract

The actual six fixture totals were reread and match the original receipts
exactly: [space-rechecked.json](latency/space-rechecked.json).

| Layout | PH3 SST bytes | Format-3 SST bytes | Overhead | Point bytes/key |
|---|---:|---:|---:|---:|
| bulk | 360,945,109 | 329,843,853 | +9.43% | 31.101 |
| normal | 361,480,243 | 329,852,455 | +9.59% | 31.622 |

Both layouts remain below +12%. No writer or point-encoding source changes are
retained. PH2 and current PH3 remain readable. [DESIGN.md](DESIGN.md) keeps the
flag-zero bytes and meanings frozen, including the optional value-checksum
reservation at bit 0. This task does not implement that extension.

The byte-only anchor curve reconstructs every current key, checks section CRCs,
and recomputes suffix sizes while retaining existing SST partitions:

| Layout | Anchor interval | Projected point bytes/key | Projected SST overhead |
|---|---:|---:|---:|
| bulk | 1024 | 31.101 | +9.43% |
| bulk | 512 | 30.480 | +9.24% |
| bulk | 256 | 30.096 | +9.12% |
| normal | 1024 | 31.622 | +9.59% |
| normal | 512 | 30.746 | +9.32% |
| normal | 256 | 30.170 | +9.15% |

The smaller intervals save suffix bytes in this corpus because they cross fewer
agent-prefix boundaries. These are projections, not newly emitted files or
latency measurements. A normal writer can also repartition tables differently.
No latency claim is made for 512/256 anchors. The observed target already passes,
so a format revision is not unavoidable and was not implemented.

## Gates and reproduction

Final engine bytes run through all 11 `verify.py` gates in
[checks](latency/checks/), including debug/release workspace tests, fmt, clippy
with `-D warnings`, both fixed crash corpora, both 20,000-seed suites starting at
zero, loom, site facts, and readbench checks. The source hashes in that directory
identify the tested bytes. A second full run after the commit records the exact
final HEAD under `target/pointindex-latency/postcommit-checks/`.

Build and test commands use `taskset -c 12-21 nice -n 10`. Timed commands use
CPU 8 only. The read, profile, and maintenance drivers and their children inherit
nice 10 uniformly on both sides. This task assigns no work to CPUs 3/4, 6/7, 9, or 10/11.
The versioned drivers reproduce the read matrix, summaries, point profiles,
maintenance pairing, and anchor byte projection. From the worktree root:

```sh
# Freeze source-specific release binaries in separate target directories first.
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/pointindex_latency.py --before target/pointindex-latency/ph2-readbench --after target/pointindex-latency/final-readbench --out target/pointindex-latency/repeat-reads
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/pointindex_latency_report.py target/pointindex-latency/repeat-reads
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/pointindex_maintenance.py --before target/pointindex-latency/ph3-test --after target/pointindex-latency/final-test --out target/pointindex-latency/repeat-maintenance
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/pointindex_profile.py --binary target/pointindex-latency/final-readbench --db target/pointindex-cost/data/after/kiban-bulk --out target/pointindex-latency/repeat-profile --perf /path/to/perf
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/pointindex_anchor_curve.py
CARGO_TARGET_DIR=target/pointindex-latency/verified-target taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/verify.py target/pointindex-latency/repeat-checks --cpus 12-21
```

Drivers refuse to overwrite run receipts. The maintenance selector
`KIBAN_POINT_BENCH_TRIAL=1..5` permits interleaving versions without changing the
measured phases. Its default still executes all five trials. Decisions,
rejections, and integrity findings are recorded in [decisions.tsv](decisions.tsv).
