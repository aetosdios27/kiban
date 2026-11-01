# PH3 scan extension: latest-K faster than LMDB

**Result:** with `point_index = true` and `scan_prefix_len` set, the prefix "latest ten"
read (`recent-for-agent`) beats LMDB in every measured cell. It meets the target of
paired Kiban/LMDB p50 ≤ 0.85 and p99 ≤ 1.0 on bulk and normal layouts, at 512 MiB and
default cache. Point reads and writes do not regress. The feature is opt-in. Design:
[DESIGN.md](DESIGN.md). Decision trail: [decisions.tsv](decisions.tsv).

Before is `main` f3d21dc1 (PH3 point index on, no scan extension). After is this
branch at 74c40c46. Each scenario ran five alternating before/after pairs, each
Kiban run followed by an LMDB control, with timed runs on CPU 3 only. Every timed
result is payload-verified; the full dataset is verified before and after.
Binaries and source hashes are in [final/binaries.json](final/binaries.json) and
[final/source-hashes.json](final/source-hashes.json).

## Latest ten (1M records, µs)

| Scenario | Kiban p50/p99 before | after | LMDB after | Paired K/LMDB p50 before → after | p99 before → after |
|---|---:|---:|---:|---:|---:|
| bulk-512 | 3.968/7.296 | **2.624/4.992** | 3.136/6.272 | 1.216 → **0.837** | 1.245 → **0.796** |
| bulk-default | 3.968/7.680 | **2.560/4.864** | 3.136/5.376 | 1.240 → **0.833** | 1.277 → **0.881** |
| normal-512 | 4.480/8.960 | **2.560/4.608** | 3.136/5.504 | 1.400 → **0.816** | 1.321 → **0.878** |
| normal-default | 4.608/9.728 | **2.624/4.992** | 3.136/5.760 | 1.400 → **0.833** | 1.462 → **0.767** |

Varied bounds, 100k distinct agent bounds ([final/VARIED.md](final/VARIED.md)): paired
p50 0.803–0.833, p99 0.847–0.881 vs LMDB. The win does not depend on the 64 hot bounds.

## No regressions

- **Point reads** ([final/READS.md](final/READS.md)): paired Kiban/LMDB p50 before → after
  is 0.645 → 0.633 (bulk-512), 0.629 → 0.627 (bulk-default), 0.852 → 0.867 (normal-512)
  and 0.931 → 0.825 (normal-default).
- **Writes** ([final/WRITES.md](final/WRITES.md), 10k records, five pairs): durable
  remember/update p50 is 1,114.1/1,146.9 → 1,146.9/1,114.1 µs. Fast remember/update is
  0.704/0.832 → 0.736/0.864 µs. All of these are within trial noise, and Kiban
  durable/fast stay ~2×/~10× faster than LMDB.
- **General range scans**, 10 and 256 records from arbitrary starts
  ([final/range](final/range)): p50 after/before is 0.95–1.00 in all eight cells, with
  zero errors. p99 is 0.60–1.07 except bulk-default/256 at 1.37. That cell has
  1,000 samples per trial and its p50 is flat (1.00), so treat it as a tail
  measurement to recheck, not a proven regression.

## Costs

| Index on, 1M records (five pairs, median) | Before | After |
|---|---:|---:|
| Two flushes | 1.62 s | 1.64 s |
| Compaction BUILD | 0.78 s | 0.81 s |
| Compaction output | 347.8 MB | 351.9 MB (+1.2%; ~4 bytes/key scan offsets) |
| Bulk SST bytes | 360.9 MB | 364.9 MB |

All 1M compaction outputs were verified with zero errors
([final/maintenance](final/maintenance)). The global ordering view is derived per pinned
Version on first eligible use, not at publication.

## Correctness

All 13 gates pass on the final source ([final-gates](final-gates), [final/gates](final/gates)):

- fmt and clippy `-D warnings`;
- debug and release workspace tests;
- fixed crash corpora;
- both 20k-seed crash/model suites;
- loom;
- site facts;
- readbench fmt, clippy and tests, including the scan harness.

A scan-merge bug found during development (a superseded memtable value or a
resurrected tombstone could be emitted) was fixed at its root. Red and green logs are
in [regressions](regressions). An independent correctness review (separate worktree,
`validation/audit`) reproduced the fix across active and frozen memtables at all
sequence limits.

Compatibility: PH3 flag bit 1 advertises the `KIBANSC1` extension, and bit 0 stays
reserved. Older admitting readers reject the nonzero flag. Their verified
sorted-block fallback remains correct. Disable the extension and rewrite tables
before downgrading.

## Limits

- The measurements were taken on a shared host; sibling-idle checks are recorded per run.
- Warm cache only.
- Single reader.
- The gain applies to prefix-bounded scans whose prefix matches the configured
  `scan_prefix_len`; other ranges use the unchanged engine merge.
- Reports finalized by the parent after the implementing session hit its usage
  limit. All numbers come from the recorded receipts listed above.
