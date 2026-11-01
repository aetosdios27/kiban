# Kiban v2 locator-index validation

Verdict: HOLD: strict scan gate not cleared. The paired p50 scan gate passes: required latest-K medians improve 4.9–8.1% after LMDB normalization, and range p50 improves 15.7–36.5%. Bulk/512 latest-K paired normalized p99 is 1.125x main, so full tail non-regression is not established. Keep the feature opt-in pending tail investigation.

Main is `8bbb4934f8d102ed0285c813a3770831b21c63e5`, extracted with `git archive` and built without engine edits. The final engine uses full-key MPHF records with locators into sorted data. Values appear once on disk. Both builds keep main's RAM-sized default cache. The point-index write option remains opt-in.

The original five-trial read, scan, write and maintenance matrices below precede the allocation fix. The memory section and the three-trial latency follow-up measure that fix separately.

All latency tables show microseconds. Except for the allocation follow-up, entries are medians across five alternating before/after trials. Each read/write candidate has its own following LMDB control. Ratios divide Kiban by that control; lower is better. These are warm, single-thread public operations on CPU 8. Builds and correctness tests run separately on CPUs 2,3,4,5,6,7,10,11 at nice 10. CPU 9 is never assigned. The independent crash sweep on CPUs 12–21 remains untouched.

## Point reads and latest-K

Each stage has 100,000 timed calls. Every call checks returned payloads after timing. Every command verifies the full 1M-record database before and after timing. Point calls include key formatting, value copying and JSON decoding. Latest-K uses the bounded range API and returns ten decoded records for one agent.

| Layout/cache | Operation | Main p50/p99 | Final p50/p99 | LMDB final p50/p99 | Paired p50 K/LMDB main → final | Paired p99 K/LMDB main → final |
|---|---|---|---|---|---|---|
| bulk/512 | recall-by-id | 1.888/2.880 | 1.152/1.536 | 1.568/2.240 | 1.220 → 0.740 | 1.286 → 0.671 |
| bulk/512 | recent-for-agent | 3.072/5.888 | 3.136/6.016 | 2.624/4.608 | 1.237 → 1.195 | 1.225 → 1.314 |
| bulk/32 | recall-by-id | 3.840/8.960 | 4.096/9.216 | 1.568/2.240 | 2.500 → 2.571 | 4.111 → 3.886 |
| bulk/32 | recent-for-agent | 3.008/6.400 | 3.264/6.656 | 2.752/5.248 | 1.231 → 1.186 | 1.405 → 1.333 |
| bulk/default | recall-by-id | 1.952/2.944 | 1.248/1.600 | 1.504/2.112 | 1.255 → 0.830 | 1.324 → 0.742 |
| bulk/default | recent-for-agent | 3.008/6.400 | 3.136/6.400 | 2.624/4.992 | 1.256 → 1.200 | 1.351 → 1.282 |
| normal/512 | recall-by-id | 2.496/5.248 | 1.600/2.368 | 1.792/2.944 | 1.362 → 0.833 | 1.629 → 0.696 |
| normal/512 | recent-for-agent | 4.224/10.752 | 4.032/10.496 | 3.200/7.424 | 1.514 → 1.360 | 1.595 → 1.241 |
| normal/32 | recall-by-id | 5.376/14.080 | 4.992/12.544 | 1.920/3.520 | 2.820 → 2.621 | 4.340 → 3.709 |
| normal/32 | recent-for-agent | 4.480/12.800 | 4.480/11.264 | 3.264/7.808 | 1.545 → 1.373 | 2.075 → 1.517 |
| normal/default | recall-by-id | 2.048/3.712 | 1.376/1.952 | 1.600/2.560 | 1.306 → 0.860 | 1.487 → 0.756 |
| normal/default | recent-for-agent | 3.584/6.912 | 3.584/6.656 | 2.688/4.736 | 1.474 → 1.333 | 1.588 → 1.444 |

See [READS.md](READS.md) and [read-summary.json](read-summary.json) for every trial's point and latest-K latency range.

### Allocation-only latency follow-up

Three alternating pre-fix/fixed pairs on the same bulk/default fixture, with one adjacent LMDB control per candidate. Each stage checks 100,000 timed calls and every process verifies all 1M records before and after timing. Times are microseconds. The normalized ratio divides fixed Kiban/LMDB by pre-fix Kiban/LMDB within each pair.

| Operation | Before p50 [range] | Fixed p50 [range] | Before → fixed p99 medians | Paired normalized p50 fixed/before [range] | Paired normalized p99 fixed/before [range] |
|---|---|---|---|---|---|
| recall-by-id | 0.976 [0.928, 1.024] | 0.912 [0.912, 0.944] | 1.312 → 1.216 | 0.983 [0.956, 0.991] | 0.984 [0.982, 1.107] |
| recent-for-agent | 2.688 [2.688, 2.752] | 2.560 [2.560, 2.560] | 4.864 → 4.864 | 1.010 [0.985, 1.039] | 0.989 [0.980, 1.144] |

The quick check shows no consistent latency regression from the allocation fix. Latest-K paired normalized p50 varies from 0.985 to 1.039x before, with a 1.010x median. This three-trial check does not replace the original five-trial scan-tail gate or change the HOLD verdict. All distributions and controls are in [open-memory/summary.json](open-memory/summary.json).

## Scan gate

Ratios below pair trial numbers. The normalized ratio divides final Kiban/LMDB by main Kiban/LMDB. Raw ratios retain the public API latency; controls expose drift. Ratios are computed within each trial before taking medians, so dividing displayed medians need not reproduce paired changes. Range probes use one identical source linked separately against each engine library. They start at varying keys within each agent and check every returned key and complete payload. Range samples exclude verification and include iterator construction and value copying.

| Layout/cache | Raw final/main p50 [range] | Control-normalized final/main p50 [range] | Raw final/main p99 [range] | Control-normalized final/main p99 [range] |
|---|---|---|---|---|
| bulk/512 | 1.021 [0.941, 1.043] | 0.946 [0.874, 0.991] | 1.041 [0.636, 1.182] | 1.125 [0.842, 1.280] |
| bulk/32 | 1.064 [1.062, 1.109] | 0.980 [0.808, 0.988] | 1.040 [1.000, 1.346] | 0.960 [0.469, 1.158] |
| bulk/default | 1.021 [1.000, 1.213] | 0.951 [0.922, 1.182] | 1.000 [0.960, 2.000] | 0.949 [0.887, 1.870] |
| normal/512 | 1.054 [0.843, 1.167] | 0.948 [0.657, 1.108] | 0.936 [0.852, 1.390] | 1.006 [0.517, 1.224] |
| normal/32 | 1.000 [0.946, 1.029] | 0.944 [0.734, 0.975] | 0.880 [0.764, 1.000] | 0.756 [0.405, 1.232] |
| normal/default | 1.000 [0.934, 1.018] | 0.919 [0.890, 0.970] | 0.944 [0.722, 1.020] | 0.906 [0.692, 0.973] |

| Layout/cache/count | Main p50 µs [range] | Final p50 µs [range] | Paired final/main p50 [range] | Paired final/main p99 [range] |
|---|---|---|---|---|
| bulk/512/10 | 2.529 [2.397, 2.572] | 2.043 [1.847, 2.138] | 0.808 [0.718, 0.887] | 0.890 [0.572, 1.130] |
| bulk/512/256 | 29.104 [28.361, 29.143] | 18.262 [17.734, 18.547] | 0.635 [0.609, 0.646] | 0.596 [0.386, 0.687] |
| bulk/default/10 | 2.500 [2.416, 2.532] | 1.941 [1.833, 2.123] | 0.777 [0.724, 0.879] | 1.006 [0.548, 1.200] |
| bulk/default/256 | 28.417 [28.157, 29.884] | 18.246 [17.922, 18.763] | 0.638 [0.600, 0.660] | 0.627 [0.524, 0.883] |
| normal/512/10 | 3.342 [3.215, 3.560] | 2.649 [2.468, 2.817] | 0.775 [0.738, 0.864] | 0.879 [0.507, 1.211] |
| normal/512/256 | 32.180 [30.767, 33.260] | 21.538 [20.605, 21.793] | 0.671 [0.638, 0.690] | 0.660 [0.229, 0.713] |
| normal/default/10 | 3.392 [3.154, 3.406] | 2.669 [2.655, 2.989] | 0.843 [0.781, 0.878] | 0.972 [0.646, 1.379] |
| normal/default/256 | 30.818 [30.331, 31.745] | 22.179 [21.326, 22.280] | 0.715 [0.672, 0.732] | 0.778 [0.428, 1.134] |

Explicit 512 MiB is 536,870,912 bytes. The actual default is 449,030,592 bytes (428.229 MiB). An ancestor cgroup memory.max of 28,737,957,888 bytes limits the default below the physical-RAM estimate. Both binaries used this actual default; the earlier 492 MiB descriptive estimate was incorrect. Default runs are independent trials.

## Disk space and SST count

| Records/layout | Main SST MiB | Final SST MiB | Final/main | Main → final SST files | Locator MiB |
|---|---|---|---|---|---|
| 10000/bulk | 3.128 | 3.894 | 1.245 | 1 → 1 | 0.766 |
| 10000/normal | 3.128 | 3.894 | 1.245 | 1 → 1 | 0.766 |
| 1000000/bulk | 314.564 | 391.176 | 1.244 | 1 → 1 | 76.612 |
| 1000000/normal | 314.572 | 391.195 | 1.244 | 84 → 99 | 76.615 |

Locator metadata retains each full key, MVCC fields, a u64 value address, 12-byte hash slots and about 1.333 pilot bytes per distinct key. It increases output size estimates and can change compaction boundaries. The sorted prefix-compressed value data is unchanged.

## Write and maintenance costs

The write suite starts from 10k records and performs 1,000 calls per workload in fast and durable modes. Durable mode syncs each mutation. Per-call latencies exclude final flush. Completion time below includes sync, flush and verification. [WRITES.md](WRITES.md) retains paired p50/p99 and LMDB controls.

| Mode | Main finish seconds [range] | Final finish seconds [range] |
|---|---|---|
| fast | 0.024 [0.023, 0.025] | 0.024 [0.022, 0.024] |
| durable | 0.027 [0.026, 0.029] | 0.024 [0.023, 0.025] |

Maintenance uses the final engine with the index off and on, five alternating pairs on CPU 8. Each pair writes 1M fixed 8-byte keys and 296-byte values, flushes two 500k-record tables, and compacts them. Every output value is checked after timing. BUILD includes merge, index construction, output installation and open-time validation. The standard Rust test allocator differs from the mimalloc read/write executables.

| Metric | Index off median [range] | Index on median [range] | Paired on/off [range] |
|---|---|---|---|
| Two flushes, seconds | 2.990 [1.505, 3.200] | 3.567 [2.349, 4.611] | 1.361 [0.930, 1.587] |
| Compaction BUILD, seconds | 0.716 [0.639, 1.736] | 2.473 [1.142, 3.580] | 2.930 [1.481, 3.872] |
| Compaction COMMIT, ms | 2.544 [2.482, 3.696] | 27.459 [15.663, 28.212] | 10.912 [4.238, 11.335] |
| Output MiB | 307.991 [307.991, 307.991] | 356.001 [356.001, 356.001] | 1.156 [1.156, 1.156] |
| Output SST count | 77.000 [77.000, 77.000] | 89.000 [89.000, 89.000] | 1.156 [1.156, 1.156] |
| Resident point/backing MiB | 0.000 [0.000, 0.000] | 156.739 [156.739, 156.739] | n/a |

## Memory and limits

The open-allocation follow-up compares locator v2 at `6fc0a0a6` with the fix, using self-built release readbench binaries. The pre-fix binary is byte-identical to the original validated v2 binary. Entries are median [min, max] peak RSS in MiB from three alternating pairs on CPU 8. `/usr/bin/time` measures the whole process, including open, both full-model verifications and the timed reads. Both binaries read the same private copies of the 1M-record locator fixtures. Builds and tests finished before measurements began.

| Layout/cache | V2 before fix peak RSS MiB [range] | V2 after fix peak RSS MiB [range] | Median reduction MiB |
|---|---|---|---|
| bulk/512 | 714.438 [714.383, 714.590] | 422.340 [422.336, 422.355] | 292.098 |
| bulk/default | 714.387 [713.621, 714.504] | 421.457 [417.930, 421.535] | 292.930 |
| normal/512 | 470.684 [469.922, 471.344] | 463.590 [462.660, 465.086] | 7.094 |
| normal/default | 476.762 [476.750, 477.000] | 469.250 [469.160, 469.324] | 7.512 |

Both the point section and the sorted data region now load directly into their final `Arc<[u8]>` allocations through `FileLease::read_shared_range_at`. The reader initializes each allocation in place and fills it through the existing simulated-device or positioned-file read. `PointIndex::decode` retains those allocations without copying. Point-section checksum verification, per-block checksum verification and locator-to-sorted-record validation still run once at load. The combined `offset + len` reservation is acquired before either allocation and continues to charge both resident byte regions. Bulk/512 peak RSS falls from 714.438 to 422.340 MiB, below the original main median of 433.363 MiB.

Point locators and the complete sorted backing region share the existing raw block-cache capacity. Admitted scans reuse verified backing views. Denied tables verify the locator section in bounded chunks and use sorted blocks. Admission is per table and depends on open order and pinned snapshots/compaction inputs. Reopen can change admission; the latency matrix measures reopened, warm databases. It does not measure cold-open latency or concurrent foreground reads during compaction.

Arc overhead, saved block metadata and other SST metadata remain outside the raw-byte cache budget. Whole-SST builders and MPHF construction need temporary workspace proportional to one output. A bulk SST and oversized key/version groups retain the existing soft-size exceptions. Peak RSS includes allocator and verification buffers, but does not bound builder workspace or the host page cache.

The [follow-up summary](open-memory/summary.json) retains every RSS and latency sample. [Build provenance](open-memory/build-provenance.json), [measurement metadata](open-memory/metadata.json) and [command receipts](open-memory/commands.jsonl) record the frozen binaries, engine source hashes and CPU affinity. The 30 commands check 6M timed calls with zero payload errors. Reproduce with `v2_open_memory.py` as described in [VERIFY.md](VERIFY.md).

Format 4 uses locator feature flag 2 and section magic KIBANPH2. It rejects the earlier unpublished duplicate-value encoding. Format-3 readers reject format 4. Rewriting with point_index=false is required before downgrading to an engine that only reads format 3. The final API default remains point_index=false.

The five-trial results share host memory, LLC and power with the independent crash sweep and desktop processes. Alternating runs and adjacent LMDB controls reduce drift; they do not remove contention. Tail ratios retain the measured control variation. These results apply to this seed, workload and warm-cache policy.

The earlier duplicate-value matrix is rejected and retained in [scan-regression-screening](scan-regression-screening/NOTICE.md). Its normal/512 latest-K p50 Kiban/LMDB ratio rose from 1.407 to 1.667 and SST count doubled from 84 to 167. The profile recorded 53.9B → 57.1B instructions and 25.3M → 46.2M branch misses for 1M latest-K calls. Those counters diagnose that rejected layout; they are not a profile of final locators.

## Correctness gates

All 11 gates pass again for the allocation fix. [open-memory/checks](open-memory/checks) captures the exact engine source hashes before and after verification. The original matrix gate receipts remain in `final-checks`. Both seeded suites cover 20,000 seeds starting at zero. Seed parity selects the initial write format, and crash reopens flip format policy. Mixed-format reads, acknowledged-prefix recovery, MVCC snapshots, tombstones, full-key comparison despite fingerprint collisions, corruption, and cache accounting retain their focused tests. New tests check shared positioned reads and short-read errors, retain both supplied allocation addresses through decode, and verify the combined reservation and its release.

| Gate | Status | Seconds |
|---|---|---|
| test | PASS | 9.80 |
| release | PASS | 18.86 |
| clippy | PASS | 1.16 |
| fmt | PASS | 0.31 |
| fixed | PASS | 0.39 |
| long-20000-each | PASS | 92.53 |
| loom | PASS | 18.07 |
| site-facts | PASS | 0.10 |
| readbench-clippy | PASS | 0.50 |
| readbench-test | PASS | 1.70 |
| readbench-fmt | PASS | 0.06 |

Reproduce with [VERIFY.md](VERIFY.md). Engine source hashes, binary hashes, command receipts and per-trial samples are committed beside this report. [decisions.tsv](decisions.tsv) records the design changes and verdict. No push or PR was made.


## Final CPU counters

One diagnostic run per row, outside the five-trial latency matrix. Counters cover 1M full public operations, including key formatting and JSON decoding. Events are user-mode cpu_core counters. These observations diagnose CPU work; they are not additional latency trials.

| Workload/build/backend | cycles/op | instructions/op | LLC misses/op | dTLB misses/op | branch misses/op |
|---|---|---|---|---|---|
| profile/bulk-after-kiban | 4350.296 | 5638.876 | 10.531 | 0.020 | 1.719 |
| profile/bulk-after-lmdb | 6070.815 | 7893.384 | 15.368 | 1.976 | 11.179 |
| profile/bulk-before-kiban | 7408.691 | 8253.156 | 13.755 | 0.156 | 2.720 |
| profile/bulk-before-lmdb | 6052.775 | 7893.385 | 15.658 | 2.024 | 10.237 |
| profile/normal-after-kiban | 5322.451 | 7287.593 | 10.094 | 0.098 | 4.951 |
| profile/normal-after-lmdb | 6035.627 | 7893.384 | 15.421 | 1.976 | 11.180 |
| profile/normal-before-kiban | 7747.652 | 9557.304 | 12.795 | 0.118 | 5.903 |
| profile/normal-before-lmdb | 5885.389 | 7893.384 | 15.263 | 1.973 | 10.094 |
| profile-scan/bulk-after-kiban | 12140.591 | 48449.041 | 0.108 | 0.102 | 22.591 |
| profile-scan/bulk-after-lmdb | 10444.764 | 42341.872 | 0.106 | 0.127 | 27.020 |
| profile-scan/bulk-before-kiban | 11759.379 | 48826.370 | 0.170 | 0.164 | 13.890 |
| profile-scan/bulk-before-lmdb | 9361.793 | 42341.847 | 0.106 | 0.082 | 13.974 |
| profile-scan/normal-after-kiban | 14667.076 | 53256.873 | 0.301 | 0.146 | 36.413 |
| profile-scan/normal-after-lmdb | 10195.437 | 42341.869 | 0.076 | 0.117 | 23.954 |
| profile-scan/normal-before-kiban | 14001.813 | 53914.668 | 0.431 | 0.175 | 25.853 |
| profile-scan/normal-before-lmdb | 9368.872 | 42341.852 | 0.086 | 0.084 | 14.120 |

The point gain combines hash routing and contiguous backing storage. In the diagnostic bulk get, instructions fall from 8,253 to 5,639 per call and dTLB misses from 0.156 to 0.020. This integrated measurement does not isolate hashing from memory layout or THP; the earlier ablation addresses those effects for its own layout.

The remaining scan concern has supporting counter evidence. Bulk latest-K cycles rise from 11,759 to 12,141 per call, while branch misses rise from 13.890 to 22.591 despite slightly fewer instructions. Normal latest-K also records more branch misses. LMDB scan controls show branch-miss drift too, so these observations do not locate a source-level cause. The data justify holding the strict tail gate, not assigning the entire p99 shift to the index.
