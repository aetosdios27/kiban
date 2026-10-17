# Point-index cost reduction

Keep `KibanOptions::point_index` opt-in. PH3 reduces locator bytes by 61% and
observed paired BUILD overhead to 1.263x, but flush overhead is still 1.362x
against a 1.15x target. The warm point-read advantage over LMDB remains, while
bulk read-tail comparisons do not clear a strict no-regression requirement.
CPU 9 was not continuously idle because of unrelated host activity; timing
conclusions therefore remain qualified. No default-flip commit is included.

Implementation: `4feca511`, based on main `06a457db`. The second commit records
validation and freezes the encoding contract in [DESIGN.md](DESIGN.md). No
per-value checksum extension is implemented here. Bit 0 is reserved there for
the small-cache task; flag-zero bytes and interpretation are stable.

## What changed and why

PH3 keeps compact MVCC metadata beside each value address, prefix-compresses
keys against an anchor every 1,024 distinct keys, uses 4-byte pointers when
possible, and chooses field widths from actual table maxima. It retains
full-key verification and all supported key/value/sequence limits. Values stay
in the original sorted blocks. Narrowing is conditional; wide encodings remain
supported and tested.

MPHF construction now distributes indices into contiguous buckets using counts
and prefix sums. Equal hashes necessarily share a bucket, so duplicate detection
uses local comparisons instead of a global sort. A mean bucket size of two
rather than three increases pilot bytes from 1.333 to 2 per key but reduces
expensive multi-key pilot searches. Common compact lookup layouts are inlined.
Open-time validation uses one ordered backing-block walk to check the compact
metadata, anchors, slots and addresses against the authoritative sorted records.
No validation, checksum, full-key check or cache charge is skipped.

A smaller draft that stored sorted-header addresses saved another dependent
metadata copy but regressed normalized warm point p50 by 22.5% over five pairs.
It was rejected. Its receipts are in [rejected-header](rejected-header/README.md).
The final format stores compact MVCC fields locally to avoid that dependent
header load. [screen](screen/README.md) is a single pre-inlining development pair,
also excluded from the final tables.

## Profiling

`perf record` sampled only the BUILD phase of the maintenance benchmark, using
FIFO enable/disable controls, `cpu-clock:u`, 499 Hz, user callchains and 4,096-byte
DWARF stacks. Each recording covers five alternating on/off pairs on CPU 8.
The baseline engine is main with only the benchmark phase probes copied in:
[patch](baseline-profile-patch.diff), [binary/source provenance](baseline-profile-source.json).
Ordinary before/after maintenance tables below use unprofiled binaries.

| BUILD self CPU share, both on/off paths | PH2 | PH3 |
|---|---:|---:|
| MPHF construction | 13.25% | 8.91% |
| `memmove` | 17.59% | 18.34% |
| `memset` | 6.74% | 7.70% |
| Main free routine | 8.64% | 8.96% |
| Main malloc routines combined | 8.00% | 6.96% |
| CRC update | 7.82% | 8.33% |

PH2 recorded 2,211 samples (about 4.431 seconds sampled CPU); PH3 recorded
1,897 (about 3.802 seconds). Both report **zero lost samples**. These shares
include shared engine work and do not partition the incremental index cost.
Reports: [before](before-build-profile.txt), [after](after-build-profile.txt).
Raw perf data remains under `target/pointindex-cost/{before,after}-build.perf.data`.
Earlier whole-job `before-profile*` reports and perf probes are superseded;
the whole-job recording lost samples and is not used for conclusions.

Separate phase probes read per-thread CPU ticks (100 Hz). They sum both flushes
per trial. Medians below are diagnostic CPU seconds, not the elapsed results or
additional accepted performance trials:

| Phase | PH2 off / on CPU s | PH3 off / on CPU s |
|---|---:|---:|
| Two flushes | 0.73 / 1.40 | 0.74 / 1.31 |
| BUILD | 0.45 / 0.75 | 0.45 / 0.64 |
| COMMIT | <0.01 / 0.02 | <0.01 / 0.02 |

The evidence identifies construction, copying/allocation and CRC work rather
than a single 2.9x bottleneck. Temporary full-key/MVCC collection and MPHF
workspace remain, as do sorted-data writes and admitted-open verification.
Flush CPU overhead remains large, so elapsed improvement alone cannot justify
a default flip. COMMIT does not open/validate SSTs: BUILD opens them before
publication. COMMIT still publishes the manifest/version and releases pinned
input state; it retains a measurable cost. No concurrency or reclamation change
is included here.

## Maintenance: five alternating pairs, 1M records

Each trial validates all 1M records. The ratio is the **median of five paired
on/off ratios**, with the full observed range in brackets. Off and on columns
are separate elapsed medians, so their quotient need not equal the paired
median. All trials are retained, including scheduler/writeback outliers.

| Version | Operation | Index off median | Index on median | Paired on/off [min, max] |
|---|---|---:|---:|---:|
| PH2 before | Two flushes | 0.948 s | 1.727 s | 2.146 [0.467, 3.346] |
| PH2 before | Compaction BUILD | 0.729 s | 0.948 s | 1.532 [1.072, 4.397] |
| PH2 before | Compaction COMMIT | 3.127 ms | 20.964 ms | 7.887 [2.180, 10.830] |
| PH3 after | Two flushes | 1.700 s | 3.033 s | 1.362 [0.586, 3.449] |
| PH3 after | Compaction BUILD | 1.194 s | 1.348 s | 1.263 [0.560, 2.448] |
| PH3 after | Compaction COMMIT | 7.536 ms | 23.847 ms | 4.717 [1.454, 5.154] |

Observed PH3 BUILD meets 1.5x; flush misses 1.15x. Broad ranges and sibling
activity prevent treating these as controlled-host pass certificates. The
published v2 BUILD result was 0.72 → 2.47 s (2.9x); a fresh main baseline here
is 0.729 → 0.948 s, paired 1.532x. That historical slowdown did not reproduce.
Off-path elapsed time also changed materially between before and after runs,
so this report does not claim an absolute elapsed speedup from their raw
medians. The CPU probes provide independent evidence of reduced BUILD work.

Compaction output bytes fall from 373,293,666 to 347,833,741 (89 → 83 files);
the format-3 control is 322,951,967 (77 files). These maintenance records use a
different key/value shape from the readbench corpus, so their byte totals must
not be combined with the following per-key table.

## SST bytes and memory

Fresh readbench fixtures contain 1M 38-byte keys and 256-byte values. The bulk
format-3 control is 314.564 MiB; normal is 314.572 MiB. Section bytes include
header, pilots, slots, arena and trailer. Total SST overhead is relative to the
matching format-3 fixture, including file-count/partition differences.

| Layout | Encoding | SST MiB | Over format 3 | Point-section bytes/key | SSTs |
|---|---|---:|---:|---:|---:|
| bulk | PH2 | 391.176 | +24.35% | 80.333 | 1 |
| bulk | PH3 | 344.224 | +9.43% | 31.101 | 1 |
| normal | PH2 | 391.195 | +24.36% | 80.337 | 99 |
| normal | PH3 | 344.734 | +9.59% | 31.622 | 94 |

PH3 meets the +12% disk target in both layouts. Typical bulk PH2 costs are
38 key + 8 group + 21 version + 12 slot + 1.333 pilot bytes/key. PH3 costs
7 key-header + 10 version + 6 slot + 2 pilot + 6.101 suffix/anchor bytes/key.
The measured section falls from 80.333 to 31.101 bytes/key, a 61.3% reduction.
The small fixed section overhead is included in the measured totals.

At the end of the maintenance transaction, charged resident bytes increase
from 164,352,463 (156.74 MiB) to 189,225,548 (180.46 MiB). This is not a
per-key residency comparison: input readers stay pinned while outputs are
admitted under the same 512 MiB budget, and smaller sections allow more output
admissions. After reopening read fixtures, peak process RSS falls materially
at warm-cache sizes; 32 MiB cases retain the sorted-read fallback.

| Read case | CPU 9 busy, median % [min, max] | Kiban peak RSS MiB, PH2 → PH3 |
|---|---:|---:|
| bulk-32 | 19.51% [3.98, 31.36] | 63.30 → 63.41 |
| bulk-512 | 3.99% [0.00, 18.28] | 422.12 → 367.30 |
| bulk-default | 12.50% [1.59, 21.01] | 422.29 → 367.26 |
| normal-32 | 4.76% [2.83, 16.22] | 61.47 → 61.42 |
| normal-512 | 5.59% [3.67, 10.46] | 469.18 → 403.08 |
| normal-default | 4.77% [2.55, 6.35] | 442.86 → 396.69 |

RSS is process-wide `VmHWM`, median of five Kiban processes, not cache-accounted
locator bytes. Complete sorted backing remains charged together with the index;
PH3 does not by itself solve index residency under a 32 MiB cache.

## Paired point/latest-K check

The unchanged readpath workloads run 100k point probes and 100k latest-K probes
per process on CPU 8. Every process verifies 1M records before and after the
timed stages. Each layout/cache uses five trials, alternating PH2/PH3 order;
each Kiban command is immediately followed by its matching LMDB control.
There are 120 successful read commands and 24M timed calls, with zero errors.

The normalized ratio for each trial is `(PH3 Kiban / PH3 LMDB) /
(PH2 Kiban / PH2 LMDB)`. Values below one favor PH3. Tables give medians and
full paired ranges, not confidence intervals. Raw latency medians are ns.
The `default` cache uses the existing memory-sized budget; 512 and 32 are MiB.

| Layout | Cache | Workload | Raw p50 PH2 → PH3 | Raw p99 PH2 → PH3 | Normalized p50 [min, max] | Normalized p99 [min, max] |
|---|---|---|---:|---:|---:|---:|
| bulk | 512 | Point | 1152 → 1120 | 1632 → 1536 | 1.033 [0.848, 1.280] | 1.167 [0.721, 1.636] |
| bulk | 512 | Latest-K | 3072 → 3328 | 6400 → 6912 | 1.058 [0.775, 1.205] | 1.067 [0.516, 1.407] |
| bulk | 32 | Point | 5120 → 4224 | 9728 → 7936 | 1.007 [0.815, 1.174] | 1.154 [0.718, 1.399] |
| bulk | 32 | Latest-K | 3776 → 3712 | 9472 → 8192 | 0.963 [0.913, 1.109] | 0.983 [0.799, 1.136] |
| bulk | default | Point | 1120 → 1120 | 1568 → 1728 | 0.981 [0.842, 1.192] | 1.103 [0.685, 1.549] |
| bulk | default | Latest-K | 3328 → 3456 | 6784 → 7424 | 1.066 [0.875, 1.091] | 1.128 [0.869, 1.194] |
| normal | 512 | Point | 1248 → 1280 | 1952 → 2016 | 1.003 [0.868, 1.070] | 0.947 [0.710, 1.057] |
| normal | 512 | Latest-K | 3328 → 3456 | 7296 → 7168 | 0.958 [0.886, 1.169] | 0.921 [0.829, 1.310] |
| normal | 32 | Point | 3648 → 3520 | 6400 → 6016 | 0.846 [0.759, 1.085] | 0.868 [0.711, 1.167] |
| normal | 32 | Latest-K | 3840 → 3584 | 8192 → 7552 | 0.844 [0.814, 1.069] | 0.861 [0.678, 1.237] |
| normal | default | Point | 1408 → 1344 | 2176 → 2112 | 0.971 [0.853, 1.043] | 0.916 [0.832, 1.153] |
| normal | default | Latest-K | 3840 → 3712 | 7808 → 7552 | 0.957 [0.836, 1.044] | 0.941 [0.825, 1.095] |

Warm point reads still beat LMDB: PH3 median per-trial Kiban/LMDB p50 is
0.700 bulk/512, 0.681 bulk/default, 0.870 normal/512 and 0.885 normal/default.
The normal/512 normalized point p50 changes +0.3%; bulk/512 changes +3.3%.
Bulk p99 and latest-K comparisons include larger increases, with broad ranges
and drifting LMDB controls. **Strict no-regression is not established.**
Latest-K itself remains slower than LMDB, as in the baseline. The 32 MiB bulk
point path still loses to LMDB because the backing-plus-index admission fails.

## Verification and compatibility

All 11 full `verify.py` gates pass on the final engine source bytes:

- Debug and release workspace tests; fmt; clippy with `-D warnings`.
- Both fixed crash corpora.
- Both long crash suites, 20,000 seeds each starting at zero (160,000 crash
  scenarios combined), with format-3/PH3 writes selected by seed parity and
  format-switching reopens.
- Loom, site-facts validation, readbench clippy/tests/fmt.

[Gate status](checks/status.json), [source hashes](checks/source-hashes.json)
and individual logs are preserved. That premeasurement gate receipt names
`fda287cd`, before the benchmark-driver-only amend to `4feca511`; engine source
hashes match the measured and committed implementation exactly. The validation
commit is followed by another full run in the ignored local directory
`target/pointindex-cost/final-commit-checks`, whose metadata records its exact
HEAD. No benchmark data is regenerated during that gate run.

Focused coverage includes PH2/PH3/format-3 mixed tables with snapshots and
reopens; every-bit section/footer corruption; checksummed malformed headers,
widths, pointers and metadata; in-range wrong-value pointers; full-key checking
on fingerprint collision; narrow/wide addresses; multiple anchor groups; empty,
binary and 90k-byte keys; 100k-byte values; sequence extremes and 65,537 versions;
tombstones; backing-block corruption; and reservation release on failure.
No existing-engine correctness bug or unfixable integrity bug was found.
The only early focused-test failure was an expansion-reservation fixture whose
old size premise no longer held; increasing the fixture preserved its intended
budget-failure assertion. Candidate development failures are retained in logs.

PH2 readers remain explicit in the implementation and mixed-version tests.
New writers emit PH3 in the unchanged format-4 container. Older PH2-only engines
reject PH3; downgrade requires rewriting tables with the point index off.
[DESIGN.md](DESIGN.md) gives byte offsets, flag-zero compatibility, extension
rules and the reserved optional per-version checksum contract.

## Reproduction and provenance

Host/toolchain are in [host.json](host.json) and [toolchain.txt](toolchain.txt).
Readbench uses its existing release profile and mimalloc; the maintenance test
uses the root crate release profile and standard allocator. Before/after use
the same profile per measurement. Frozen executable hashes are in
[binaries.json](binaries.json); read source/binary hashes are in
[read-provenance.json](read-provenance.json). Raw commands/configs/statuses are
under `prepare/` and `reads/`; all maintenance records and profiling commands
are alongside this report. [summary.json](summary.json) retains individual
paired data; [analysis.json](analysis.json) retains derived statistics.

All timed processes were pinned to **CPU 8 only**. Builds, tests and gates used
`taskset -c 12-21 nice -n 10`; no task here assigned CPUs 3/4, 6/7 or 9.
The sibling-idle requirement was not fully achieved: CPU 9 activity from other
host work is recorded per read command in `reads/*/sibling.jsonl` and tabulated
above. Its cause was not isolated, and foreign process affinities were not
changed. This qualifies the timing evidence and is another reason to hold
the default. Maintenance runs did not record sibling utilization, so that
condition cannot be certified for those runs either.

From fresh frozen fixtures/binaries, the local driver commands are:

```sh
# Build/archive main and candidate binaries separately before these commands.
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py prepare-before
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py prepare-after
# Run each frozen release unit-test binary with this command for five on/off pairs:
KIBAN_POINT_BENCH_N=1000000 taskset -c 8 target/pointindex-cost/bin/before-test point_index_maintenance_measurement --ignored --nocapture --test-threads=1
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py maintenance
taskset -c 12-21 nice -n 10 python3 tools/agentbench/readpath/verify.py target/pointindex-cost/final-commit-checks --cpus 12-21
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py read
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py summarize
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_space.py
# Point --perf at an available perf binary; profiles enable events only around BUILD.
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py profile --label before --perf /path/to/perf
taskset -c 12-21 python3 tools/agentbench/readpath/pointindex_cost.py profile --label after --perf /path/to/perf
```

Drivers refuse to overwrite measurement fixtures/receipts. Preserve the current
receipts before a new run. `prepare-after` recomputes `space.json`, so run the
format-3 space helper afterwards. The extra gate destination above is local;
`read` expects the committed premeasurement gates in `checks/`.

The next default decision needs further flush-copy/open-validation work and a
repeat of the paired matrix with CPU 9 actually idle. These observations justify
retaining PH3 as an opt-in improvement, not flipping the default yet.
