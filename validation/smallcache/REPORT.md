# Small-cache point reads

**The mapped warm point target passes, and the five-pair final confirmation shows no latest-ten regression (see the first section).** The final engine is rebased onto main `37feee99`. Only durable-write p99 non-regression remains unproven (not observed to regress). Keep mmap opt-in and do not claim the full requested goal is met.

Rebased source was frozen at `72e057a0f4886f48633f50999b0a6d632b2120c1`. Engine hashes in [rebased provenance](rebased-provenance.json) identify the validated source independently of later evidence-only commits.

## Final five-pair confirmation (supersedes the three-pair latest-ten verdict)

**Outcome: the latest-ten "regression" is not reproducible, and no engine change was needed.** The scan path (iterators, block cache hit path, block CRC) is byte-identical to main in this branch; the branch only changes point lookup, open-time admission, and a cached L0 boundary. Five alternating main/candidate pairs, each side with an adjacent LMDB control, on every mode/layout/cache cell (CPU 6 only, 100,000 calls each, all 1M records verified) put candidate latest-ten at or below main within noise everywhere. The three-pair 1.24x/1.51x at mapped/bulk/32 was a noise outlier, as the earlier independent five-pair control (1.029/0.966) already suggested. Engine source is unchanged: all 40 engine file hashes still match `rebased-checks/source-hashes.json`.

Diagnostics supporting this: in `rebased-scan-cache` the candidate latest-ten hot loop at 32 MiB has 0 misses and 0 evictions (110 resident blocks beside 32.6 MB of locators), with the same 1.7 block hits per call as main; the perf-stat run (`rebased-latest-profile`) shows user instructions per call 49,044 vs 49,117 and cycles 12,218 vs 12,550 (candidate vs main), so no extra copy, CRC, or page-touch work exists on the scan path. Remaining mean differences track host load (load average about 24 on 22 CPUs during this work).

Latencies in microseconds, median of five alternating trials. Ratio is raw candidate/main p50 / p99. LMDB columns are the control run next to each side. Trial ranges: [mapped 32](final-a-mapped-read/SUMMARY.md), [mapped bulk 512/default](final-b-mapped-read/SUMMARY.md), [mapped normal](final-c-mapped-read/SUMMARY.md), [pread](final-d-pread-read/SUMMARY.md).

| Mode / layout / cache MiB / workload | Main p50/p99 | Candidate p50/p99 | Cand/main p50 / p99 | LMDB (main side) p50/p99 | LMDB (cand side) p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| mapped / bulk / 32 / recall-by-id | 3.584 / 6.144 | 1.184 / 1.600 | 0.33 / 0.26 | 1.728 / 2.560 | 1.728 / 2.688 |
| mapped / bulk / 32 / recent-for-agent | 3.840 / 8.960 | 3.712 / 8.704 | 0.97 / 0.97 | 3.008 / 6.144 | 2.816 / 6.016 |
| mapped / bulk / 512 / recall-by-id | 1.216 / 1.632 | 1.056 / 1.504 | 0.87 / 0.92 | 1.856 / 2.816 | 1.824 / 2.880 |
| mapped / bulk / 512 / recent-for-agent | 3.904 / 8.704 | 3.904 / 8.960 | 1.00 / 1.03 | 3.456 / 8.192 | 3.264 / 7.680 |
| mapped / bulk / default / recall-by-id | 1.008 / 1.472 | 1.056 / 1.536 | 1.05 / 1.04 | 1.760 / 2.624 | 1.792 / 2.752 |
| mapped / bulk / default / recent-for-agent | 3.776 / 7.552 | 3.712 / 8.448 | 0.98 / 1.12 | 3.136 / 6.528 | 2.944 / 6.400 |
| mapped / normal / 32 / recall-by-id | 3.328 / 5.248 | 1.344 / 2.048 | 0.40 / 0.39 | 1.824 / 2.944 | 1.728 / 2.496 |
| mapped / normal / 32 / recent-for-agent | 4.224 / 8.192 | 3.712 / 7.296 | 0.88 / 0.89 | 3.008 / 6.272 | 2.816 / 5.760 |
| mapped / normal / 512 / recall-by-id | 1.376 / 2.112 | 1.280 / 2.016 | 0.93 / 0.95 | 1.792 / 2.688 | 1.760 / 2.688 |
| mapped / normal / 512 / recent-for-agent | 4.352 / 7.936 | 4.032 / 7.936 | 0.93 / 1.00 | 3.008 / 5.888 | 2.944 / 6.272 |
| mapped / normal / default / recall-by-id | 1.376 / 2.112 | 1.216 / 1.760 | 0.88 / 0.83 | 1.728 / 2.560 | 1.760 / 2.752 |
| mapped / normal / default / recent-for-agent | 4.352 / 8.192 | 3.776 / 7.808 | 0.87 / 0.95 | 2.944 / 5.888 | 3.008 / 6.144 |
| pread / bulk / 32 / recall-by-id | 4.864 / 7.680 | 1.920 / 2.944 | 0.39 / 0.38 | 1.856 / 3.136 | 1.888 / 2.944 |
| pread / bulk / 32 / recent-for-agent | 3.840 / 8.704 | 3.968 / 8.192 | 1.03 / 0.94 | 3.136 / 6.912 | 3.200 / 7.168 |
| pread / bulk / 512 / recall-by-id | 1.120 / 1.536 | 1.088 / 1.568 | 0.97 / 1.02 | 1.760 / 2.624 | 1.760 / 2.624 |
| pread / bulk / 512 / recent-for-agent | 4.608 / 9.472 | 3.968 / 8.960 | 0.86 / 0.95 | 3.456 / 6.528 | 3.072 / 6.016 |
| pread / bulk / default / recall-by-id | 1.152 / 1.664 | 1.120 / 1.568 | 0.97 / 0.94 | 1.920 / 2.880 | 1.952 / 3.008 |
| pread / bulk / default / recent-for-agent | 4.608 / 10.240 | 4.608 / 9.728 | 1.00 / 0.95 | 3.712 / 7.424 | 3.648 / 7.552 |
| pread / normal / 32 / recall-by-id | 4.736 / 7.424 | 2.240 / 3.648 | 0.47 / 0.49 | 1.824 / 2.880 | 1.824 / 2.816 |
| pread / normal / 32 / recent-for-agent | 4.736 / 10.496 | 4.352 / 9.216 | 0.92 / 0.88 | 3.328 / 6.528 | 3.328 / 6.784 |
| pread / normal / 512 / recall-by-id | 1.472 / 2.176 | 1.216 / 1.792 | 0.83 / 0.82 | 1.760 / 2.624 | 1.760 / 2.688 |
| pread / normal / 512 / recent-for-agent | 4.480 / 8.704 | 3.904 / 7.552 | 0.87 / 0.87 | 3.200 / 6.400 | 3.008 / 6.144 |
| pread / normal / default / recall-by-id | 1.440 / 2.176 | 1.280 / 1.920 | 0.89 / 0.88 | 1.792 / 2.752 | 1.728 / 2.624 |
| pread / normal / default / recent-for-agent | 4.608 / 8.960 | 4.096 / 8.704 | 0.89 / 0.97 | 3.264 / 6.016 | 3.072 / 5.504 |

Worst latest-ten cells versus main: pread bulk/32 p50 +3.3% (3.968 vs 3.840), mapped bulk/512 p99 +2.9%; every other latest-ten cell is equal or faster. Point reads keep the win: mapped 32 MiB point p50/p99 is 3.584/6.144 -> 1.184/1.600 (bulk) and 3.328/5.248 -> 1.344/2.048 (normal), against LMDB 1.728/2.7 (Kiban/LMDB 0.685 bulk, 0.78 normal). Larger-cache point cells are at parity or better than main (largest deviation: mapped bulk default p50 1.008 -> 1.056, +4.8%, p99 +4.3%, while still 0.56x LMDB).

### Writes: five alternating pairs, fast and durable, index off/on (10k ops per stage, default cache)

Source: [rebased-write-confirm](rebased-write-confirm/SUMMARY.md). Microseconds, medians; main -> candidate.

| Mode / index / op | p50 | p99 |
| --- | ---: | ---: |
| fast / off / remember | 1.120 -> 1.120 | 9.728 -> 9.216 |
| fast / off / update | 1.760 -> 1.632 | 11.008 -> 9.984 |
| fast / on / remember | 1.120 -> 1.120 | 9.472 -> 9.984 |
| fast / on / update | 1.664 -> 1.600 | 10.240 -> 10.496 |
| durable / off / remember | 622.6 -> 622.6 | 3211 -> 3277 |
| durable / off / update | 606.2 -> 622.6 | 1868 -> 3277 |
| durable / on / remember | 639.0 -> 655.4 | 5374 -> 3342 |
| durable / on / update | 622.6 -> 655.4 | 3473 -> 4194 |

Fast-write p50/p99 are equal within noise (index on: p99 +1.1% to +2.5%; trial ranges overlap). Durable p50 is within +5%. Durable p99 is dominated by fsync outliers and swings in both directions across the same code (index-off update 1.9 -> 3.3 ms and index-on remember 5.4 -> 3.3 ms are the same code path, both writers unchanged; LMDB controls moved 8.9 -> 11.8 ms). The data show no consistent durable p99 regression, but five pairs cannot prove p99 non-regression on fsync-bound tails; this stays a limitation. Per-value CRC adds 4 bytes per version to the point section only when point_index is on, and does not touch WAL, flush, or fsync ordering.

### Gates

No engine source changed after the rebased gates, so they apply to the final source (hash-verified): 11/11 gates in [rebased-checks/status.json](rebased-checks/status.json) passed (debug and release workspace tests, clippy `-D warnings`, fmt, fixed crash corpora, loom, both 20,000-seed suites, readbench clippy/test/fmt). `cargo fmt --check` and `cargo clippy --all-targets --workspace -- -D warnings` were also rerun clean on the final tree.

### Limitations

- mmap stays opt-in (cold-storage tradeoff documented below). Durable-write p99 is not proven non-regressive, only not observed to regress. CPU 7 (sibling of CPU 6) was not provably idle. Ratios above 1 within a few percent in individual cells (listed above) are inside the trial ranges.


## Rebased warm confirmation against current main

Three alternating pairs per layout, cache, and access mode. Every Kiban process has an adjacent LMDB control. Timed children use CPU 6; builds, gates, and coordinators use CPUs 12–21 at nice 10. Other host work remained active. Each trial runs 100,000 point calls and 100,000 latest-ten calls and verifies all 1M records. Ratios are medians of within-pair ratios. Latencies are microseconds.

Both binaries include main's SQLite benchmark support. Main fixtures are fresh flag-zero PH3, while candidate fixtures are the frozen CRC-bearing PH3 datasets. The production engine logic matches the frozen campaign; the reader has only main's explanatory comment, and the ignored maintenance benchmark gains a trial selector. See [source comparison](rebased-source-comparison.json).

| Mode / layout / cache MiB | Main p50/p99 | Candidate p50/p99 | LMDB p50/p99 | Paired K/L p50/p99 | Adjusted candidate/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| mapped / bulk / 32 | 3.968 / 6.784 | 1.440 / 2.240 | 1.952 / 3.072 | 0.682 / 0.660 | 0.389 / 0.364 |
| mapped / bulk / 512 | 1.216 / 1.728 | 1.280 / 1.856 | 1.920 / 3.072 | 0.667 / 0.617 | 1.000 / 1.015 |
| mapped / bulk / default | 1.184 / 1.792 | 1.184 / 1.792 | 1.888 / 2.944 | 0.627 / 0.565 | 1.068 / 1.000 |
| mapped / normal / 32 | 3.520 / 5.760 | 1.568 / 2.688 | 1.888 / 3.136 | 0.831 / 0.878 | 0.467 / 0.479 |
| mapped / normal / 512 | 1.920 / 2.944 | 1.504 / 2.560 | 1.984 / 3.264 | 0.758 / 0.771 | 0.783 / 0.962 |
| mapped / normal / default | 1.760 / 2.624 | 1.568 / 2.432 | 1.984 / 3.712 | 0.790 / 0.655 | 0.847 / 0.815 |
| pread / bulk / 32 | 5.120 / 9.728 | 2.240 / 3.712 | 1.920 / 3.136 | 1.186 / 1.204 | 0.438 / 0.336 |
| pread / bulk / 512 | 1.248 / 1.920 | 1.184 / 1.792 | 1.984 / 3.200 | 0.597 / 0.560 | 0.927 / 0.906 |
| pread / bulk / default | 1.248 / 2.016 | 1.376 / 2.112 | 1.952 / 3.200 | 0.717 / 0.611 | 1.034 / 0.999 |
| pread / normal / 32 | 5.120 / 8.704 | 2.496 / 4.352 | 1.984 / 3.328 | 1.246 / 1.311 | 0.471 / 0.543 |
| pread / normal / 512 | 1.728 / 2.816 | 1.664 / 2.368 | 1.920 / 3.136 | 0.765 / 0.673 | 0.845 / 0.689 |
| pread / normal / default | 1.600 / 2.560 | 1.696 / 2.496 | 1.920 / 3.072 | 0.869 / 0.804 | 0.981 / 1.075 |

[Mapped trial ranges](rebased-mapped-read/SUMMARY.md), [pread trial ranges](rebased-pread-read/SUMMARY.md). Values above one in the adjusted columns remain visible; three pairs are a confirmation, not a replacement for the five-pair campaign.

| Latest-ten mode / layout / cache MiB | Main p50/p99 | Candidate p50/p99 | Adjusted candidate/main p50/p99 |
| --- | ---: | ---: | ---: |
| mapped / bulk / 32 | 4.032 / 8.192 | 4.992 / 11.264 | 1.241 / 1.512 |
| mapped / bulk / 512 | 4.032 / 9.472 | 4.096 / 8.960 | 1.102 / 1.111 |
| mapped / bulk / default | 3.968 / 8.448 | 3.776 / 8.064 | 1.010 / 0.900 |
| mapped / normal / 32 | 4.480 / 8.448 | 3.968 / 7.808 | 0.923 / 1.017 |
| mapped / normal / 512 | 4.992 / 13.056 | 4.224 / 10.240 | 0.903 / 0.853 |
| mapped / normal / default | 4.608 / 9.472 | 4.224 / 9.984 | 0.935 / 0.902 |
| pread / bulk / 32 | 4.352 / 8.960 | 4.096 / 8.704 | 1.000 / 0.909 |
| pread / bulk / 512 | 4.096 / 8.960 | 3.968 / 8.960 | 0.905 / 0.349 |
| pread / bulk / default | 4.096 / 9.472 | 3.968 / 8.960 | 1.039 / 1.273 |
| pread / normal / 32 | 4.736 / 10.496 | 4.096 / 8.960 | 0.897 / 1.024 |
| pread / normal / 512 | 4.608 / 9.472 | 4.096 / 8.448 | 0.943 / 0.997 |
| pread / normal / default | 4.608 / 9.984 | 3.968 / 8.448 | 0.915 / 0.887 |

### Bulk control confirmation

Five additional alternating pairs repeat the three uncertain mapped bulk cache cases with the same binaries and fixtures. The original three-pair matrix is retained in full. Latencies are microseconds.

| Cache MiB / workload | Main p50/p99 | Candidate p50/p99 | Adjusted candidate/main p50/p99 |
| --- | ---: | ---: | ---: |
| 32 / recall-by-id | 3.968 / 7.040 | 1.376 / 2.112 | 0.350 / 0.276 |
| 32 / recent-for-agent | 4.032 / 8.192 | 3.968 / 7.680 | 1.029 / 0.966 |
| 512 / recall-by-id | 1.216 / 1.952 | 1.344 / 2.112 | 1.045 / 1.049 |
| 512 / recent-for-agent | 4.032 / 8.960 | 3.904 / 8.448 | 0.987 / 1.014 |
| default / recall-by-id | 1.280 / 2.048 | 1.312 / 1.952 | 1.038 / 0.899 |
| default / recent-for-agent | 3.968 / 7.936 | 3.840 / 7.808 | 1.050 / 1.139 |

The large initial bulk/32 latest-ten slowdown does not reproduce at five pairs: adjusted p50/p99 is 1.029/0.966. Other controls remain above main. Default point is 1.038/0.899, 512 MiB point is 1.045/1.049, and default latest-ten is 1.050/1.139. The data do not establish the requested larger-cache and latest-ten non-regression bounds. All trial ranges and both series remain visible.

[All five-pair controls and ranges](rebased-controls-mapped-read/SUMMARY.md).

## Rebase and final gates

PH3 base `4feca511` is already present through squash merge `f3d21dc1` and was dropped from the replay. Main's Bloom bypass is retained. Duplicate engine commit `0283fc7a` was dropped, with its unique tools and measurement evidence preserved in the campaign evidence commit. The locator lookup follows the same exact-key-before-Bloom ordering. The CRC extension, resident packing, exact reservation accounting, and L0 prefix fix remain. Main's PH3 field offsets and frozen flag-zero contract are retained in the design document. SQLite benchmarking from `37feee99` is unchanged.

All 11 gates passed again on the rebased source, including 353 debug and release library tests, fixed corpora, loom, and both complete 20,000-seed suites. No timing runs overlapped this task's builds or tests. See [gate status](rebased-checks/status.json), [source hashes](rebased-checks/source-hashes.json), and [reproduction commands](VERIFY.md).

The verifier now adds only the niceness increment needed to reach nice 10. A partial run with inherited nice 19 was preserved under rebased-checks-inherited-nice19, then all 11 gates were repeated at effective nice 10. The final metadata records coordinator nice 10 and increment zero.

No task process uses sibling CPU 7, but the sibling was not established to be idle. Whole-child CPU 7 busy fractions were median 12.3% for mapped, 10.8% for pread, and 11.9% for the extra controls, with maxima 48.8%, 87.9%, and 23.2%. These samples include open and verification, so they do not isolate activity during API timers. This is an active-host measurement, and strict idle-sibling acceptance is not established. See [sibling summary](rebased-sibling-summary.json) and the per-child sibling.jsonl receipts.

Cold, concurrent, profiling, and write rows below retain the original five-pair campaign against `06a457db`; they were not repeated against updated main. The separate write confirmation is also preserved. Those results establish the observed cold mmap tradeoff and leave durable-write p99 uncertain. The source comparison supports carrying that evidence forward, while the new three-pair warm matrix directly checks the rebased binary and main's PH3 baseline.

The [frozen source patch](experiments/frozen-source-from-37feee99.patch) reconstructs the archived engine and Rust readbench source from public main `37feee99`, independently of old local commit reachability after rebase. Applying it to a private Git index reproduced all 40 frozen engine file hashes. See the [reconstruction receipt](frozen-reconstruction.json).

## Archived five-pair campaign

Engine source: `0283fc7af8cfba828ff05343c8b77bc27b228962`; baseline main `06a457db`. All timing children use CPU 6; build/test/coordinator processes use CPUs 12–21 at nice 10. No task work runs on sibling CPU 7. Other host workloads were active; this is not an isolated-host claim.

The opt-in configuration is **CRC-bearing PH3 SSTs, point_index=true, block_cache_bytes=32 MiB, mmap_max_level=Some(u32::MAX)**. Both warm layout targets are evaluated on five alternating main/PH3 pairs, each side followed by its own LMDB control. Ratios use the median of within-trial ratios, rather than dividing unrelated medians. All five trials are retained. The warm read target is met; durable-write p99 non-regression remains uncertain after the additional confirmation block, as detailed below.

| Warm 32 MiB layout | Paired PH3/LMDB p50 | Paired PH3/LMDB p99 | Meets ≤0.9 p50 and ≤1 p99 |
| --- | ---: | ---: | --- |
| bulk | 0.641 | 0.587 | True |
| normal | 0.778 | 0.797 | True |

## Verification and read path

PH3 feature bit 0 appends an IEEE value CRC per version, preserving flag-zero PH3 and PH2 reads. Admission checks the section CRC, every sorted block CRC, exact key/MVCC/address/coverage correspondence, and each stored value CRC. External hits copy exact value bytes and CRC the owned return buffer against immutable validated metadata. Resident full-data hits retain verified owned backing; scans and sorted fallbacks retain block verification. No mapped value is returned unchecked. See [DESIGN.md](DESIGN.md).

The resident view preserves the PH3 arena and packs its hash tables (u24 pilots, no redundant fingerprints; full keys still compare). The variable locator buffer is charged exactly to the same block-cache budget; fixed table/index bookkeeping remains outside that byte accounting. Actual 32 MiB locator charges are 32,601,232 bytes bulk and 32,503,515 bytes normal. Locator mode retains no whole-SST heap backing for values; per-call owned return buffers remain necessary. Temporary open allocations, allocator retention and the OS page cache still contribute to RSS. Inspection receipts include peak/RSS and open time.

Admitted point indexes compare complete keys directly and skip redundant Bloom probes; the Bloom section is still verified at open and remains in the sorted fallback. The immutable Version caches its L0 prefix boundary, avoiding a per-point walk across every leveled table merely to filter L0. Probe order and cross-table sequence resolution remain unchanged. mmap_max_level retains its existing level-selection and best-effort fallback semantics. The option is explicitly enabled for mapped rows, and remains None for pread rows. Only CRC-bearing PH3 tables qualify for locator-only admission; existing PH2 and flag-zero PH3 remain readable through their original full-resident or sorted paths. Reservations are decided when a table handle opens and are not dynamically promoted after another snapshot or compaction releases budget.

## Measurement conditions

Mode labels refer to Kiban; LMDB always uses its native mmap-backed engine. Warm rows run 100,000 point calls and 100,000 latest-ten calls per trial, with complete 1M-record verification. Main indexed fixtures are PH2; candidate indexed fixtures use PH3+CRC, prepared from the same seed and logical workload. Normal compaction topology and bytes change with the section size; the measurements include that effect. Index-off fixtures and LMDB fixtures are shared. The computed default is 449,030,592 bytes on this host (about 428.23 MiB); explicit 512 MiB is separate.

Cold rows use cold.rs: post-open madvise(DONTNEED) and file-specific posix_fadvise(DONTNEED), then 10,000 point calls with an oracle check on each result. Pages warm during those calls. Four readers share CPU 6 and exercise I/O concurrency, not four-core CPU scaling. Storage bytes/faults are retained. Main can retain verified full backing for a subset of normal-layout tables even at 32 MiB; the new locator mode uses that budget for metadata instead. Default/512 index-on configurations retain full verified heap values through file-page eviction, so those rows are retained-resident tests, not cold-storage speedups. Open time is excluded from latency, and locator validation reads the full data region at open.

All latency tables use microseconds, median of five trials. The adjusted PH3/main columns divide each side by its adjacent LMDB control before comparing the paired results. This shows host drift but does not remove every source of noise; full min/max ranges are linked. A value above one in an adjusted column is a measured relative regression, not hidden by the headline target.

## Warm point: mapped

Mapped means Some(u32::MAX); pread means None.

| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk / 0 / 32 / 1 | 3.136 / 4.608 | 3.136 / 4.736 | 1.504 / 2.112 | 2.085 | 1.000 / 0.955 |
| bulk / 0 / 512 / 1 | 1.760 / 2.496 | 1.728 / 2.432 | 1.408 / 1.920 | 1.227 | 0.978 / 0.943 |
| bulk / 0 / default / 1 | 1.952 / 2.752 | 1.920 / 2.752 | 1.536 / 2.176 | 1.200 | 0.935 / 0.911 |
| bulk / 1 / 32 / 1 | 3.200 / 4.736 | 0.944 / 1.280 | 1.440 / 2.016 | 0.641 | 0.294 / 0.254 |
| bulk / 1 / 512 / 1 | 1.056 / 1.408 | 0.944 / 1.216 | 1.504 / 1.984 | 0.641 | 0.868 / 0.861 |
| bulk / 1 / default / 1 | 1.088 / 1.504 | 0.960 / 1.344 | 1.568 / 2.304 | 0.618 | 0.818 / 0.759 |
| normal / 0 / 32 / 1 | 3.136 / 5.504 | 3.072 / 5.248 | 1.472 / 1.984 | 2.091 | 0.999 / 1.120 |
| normal / 0 / 512 / 1 | 1.984 / 3.072 | 1.888 / 2.944 | 1.472 / 2.016 | 1.341 | 0.977 / 0.971 |
| normal / 0 / default / 1 | 1.824 / 2.752 | 1.856 / 2.816 | 1.376 / 1.824 | 1.318 | 1.002 / 1.012 |
| normal / 1 / 32 / 1 | 2.368 / 4.096 | 1.120 / 1.664 | 1.472 / 2.048 | 0.778 | 0.427 / 0.387 |
| normal / 1 / 512 / 1 | 1.248 / 1.696 | 1.008 / 1.312 | 1.440 / 1.952 | 0.750 | 0.808 / 0.788 |
| normal / 1 / default / 1 | 1.152 / 1.568 | 1.024 / 1.344 | 1.408 / 1.888 | 0.716 | 0.835 / 0.833 |

Raw trials, controls and ranges: [ph3-mapped-read/SUMMARY.md](ph3-mapped-read/SUMMARY.md).

## Warm latest-ten: mapped

| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk / 0 / 32 / 1 | 3.264 / 6.144 | 3.392 / 5.376 | 2.752 / 4.608 | 1.233 | 0.952 / 0.768 |
| bulk / 0 / 512 / 1 | 2.944 / 4.736 | 3.072 / 4.864 | 2.560 / 4.032 | 1.200 | 0.944 / 0.921 |
| bulk / 0 / default / 1 | 3.200 / 6.016 | 3.328 / 5.376 | 2.752 / 4.096 | 1.225 | 0.974 / 0.878 |
| bulk / 1 / 32 / 1 | 3.136 / 4.736 | 3.008 / 4.864 | 2.560 / 3.968 | 1.211 | 0.940 / 0.900 |
| bulk / 1 / 512 / 1 | 3.200 / 5.504 | 3.264 / 5.120 | 2.752 / 4.480 | 1.186 | 0.925 / 0.849 |
| bulk / 1 / default / 1 | 3.136 / 5.376 | 3.328 / 5.760 | 2.880 / 4.736 | 1.156 | 0.899 / 0.862 |
| normal / 0 / 32 / 1 | 3.584 / 6.912 | 3.776 / 6.528 | 2.624 / 4.224 | 1.439 | 0.964 / 1.054 |
| normal / 0 / 512 / 1 | 3.584 / 6.272 | 3.520 / 6.144 | 2.624 / 4.352 | 1.375 | 0.917 / 0.916 |
| normal / 0 / default / 1 | 3.392 / 5.504 | 3.584 / 5.632 | 2.432 / 3.968 | 1.459 | 0.972 / 0.943 |
| normal / 1 / 32 / 1 | 3.584 / 6.400 | 3.200 / 5.376 | 2.624 / 4.224 | 1.220 | 0.720 / 0.713 |
| normal / 1 / 512 / 1 | 3.392 / 6.016 | 3.008 / 5.248 | 2.496 / 4.224 | 1.205 | 0.809 / 0.786 |
| normal / 1 / default / 1 | 3.264 / 6.016 | 3.072 / 5.120 | 2.432 / 3.840 | 1.263 | 0.811 / 0.725 |

Raw trials, controls and ranges: [ph3-mapped-read/SUMMARY.md](ph3-mapped-read/SUMMARY.md).

## Cold-start point: mapped

Default/512 index-on rows retain heap values.

| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk / 0 / 32 / 1 | 3.392 / 655.360 | 3.584 / 655.360 | 3.904 / 1114.112 | 0.966 | 1.073 / 1.059 |
| bulk / 0 / 32 / 4 | 3.584 / 2162.688 | 3.520 / 2162.688 | 3.904 / 3407.872 | 0.917 | 1.034 / 0.944 |
| bulk / 0 / 512 / 1 | 3.264 / 638.976 | 3.200 / 638.976 | 3.776 / 1081.344 | 0.845 | 1.043 / 1.042 |
| bulk / 0 / 512 / 4 | 3.456 / 2228.224 | 3.456 / 2228.224 | 3.840 / 3276.800 | 0.900 | 1.047 / 1.024 |
| bulk / 0 / default / 1 | 3.264 / 638.976 | 3.264 / 671.744 | 3.968 / 1114.112 | 0.877 | 1.032 / 0.988 |
| bulk / 0 / default / 4 | 3.264 / 2162.688 | 3.392 / 2064.384 | 3.840 / 3276.800 | 0.883 | 1.000 / 1.016 |
| bulk / 1 / 32 / 1 | 3.584 / 688.128 | 1.344 / 606.208 | 4.096 / 1146.880 | 0.331 | 0.376 / 0.900 |
| bulk / 1 / 32 / 4 | 3.648 / 2359.296 | 1.280 / 1540.096 | 3.648 / 3145.728 | 0.351 | 0.351 / 0.663 |
| bulk / 1 / 512 / 1 † | 1.152 / 1.472 | 0.976 / 1.248 | 5.120 / 1212.416 | 0.191 | 0.851 / 0.932 |
| bulk / 1 / 512 / 4 † | 1.184 / 1.504 | 1.008 / 1.376 | 5.376 / 3866.624 | 0.192 | 0.873 / 0.962 |
| bulk / 1 / default / 1 † | 1.152 / 1.440 | 0.944 / 1.248 | 4.992 / 1212.416 | 0.186 | 0.789 / 0.778 |
| bulk / 1 / default / 4 † | 1.184 / 1.504 | 1.008 / 1.312 | 5.248 / 3735.552 | 0.192 | 0.806 / 0.862 |
| normal / 0 / 32 / 1 | 3.904 / 606.208 | 3.776 / 589.824 | 4.480 / 1146.880 | 0.833 | 0.989 / 0.949 |
| normal / 0 / 32 / 4 | 3.968 / 2752.512 | 4.096 / 2686.976 | 4.608 / 3670.016 | 0.871 | 1.003 / 0.945 |
| normal / 0 / 512 / 1 | 3.584 / 589.824 | 3.520 / 589.824 | 4.736 / 1179.648 | 0.743 | 1.006 / 0.950 |
| normal / 0 / 512 / 4 | 3.712 / 2424.832 | 3.712 / 2555.904 | 4.736 / 3604.480 | 0.776 | 1.000 / 1.057 |
| normal / 0 / default / 1 | 3.520 / 589.824 | 3.520 / 557.056 | 4.608 / 1114.112 | 0.786 | 1.037 / 0.944 |
| normal / 0 / default / 4 | 3.648 / 2555.904 | 3.712 / 2686.976 | 4.864 / 3866.624 | 0.763 | 1.027 / 1.015 |
| normal / 1 / 32 / 1 | 3.712 / 557.056 | 1.536 / 720.896 | 4.992 / 1179.648 | 0.308 | 0.418 / 1.294 |
| normal / 1 / 32 / 4 | 3.776 / 2686.976 | 1.504 / 2097.152 | 5.248 / 3735.552 | 0.294 | 0.434 / 0.808 |
| normal / 1 / 512 / 1 † | 1.440 / 1.920 | 1.184 / 1.600 | 5.376 / 1179.648 | 0.215 | 0.803 / 0.750 |
| normal / 1 / 512 / 4 † | 1.472 / 1.856 | 1.184 / 1.600 | 5.248 / 3670.016 | 0.220 | 0.809 / 0.888 |
| normal / 1 / default / 1 † | 1.408 / 1.856 | 1.152 / 1.632 | 5.376 / 1146.880 | 0.219 | 0.808 / 0.850 |
| normal / 1 / default / 4 † | 1.472 / 1.952 | 1.216 / 1.664 | 5.376 / 3670.016 | 0.226 | 0.792 / 0.875 |

† Retained verified heap backing; file-page eviction does not make these values cold.

Raw trials, controls and ranges: [ph3-mapped-cold/SUMMARY.md](ph3-mapped-cold/SUMMARY.md).

## Warm point: pread

Mapped means Some(u32::MAX); pread means None.

| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk / 0 / 32 / 1 | 3.712 / 5.376 | 3.968 / 5.760 | 1.472 / 2.016 | 2.571 | 0.952 / 0.909 |
| bulk / 0 / 512 / 1 | 1.920 / 2.944 | 1.888 / 2.816 | 1.440 / 2.304 | 1.318 | 0.949 / 0.884 |
| bulk / 0 / default / 1 | 1.888 / 2.688 | 1.952 / 2.816 | 1.504 / 2.112 | 1.277 | 0.963 / 0.901 |
| bulk / 1 / 32 / 1 | 3.968 / 6.272 | 1.504 / 2.304 | 1.408 / 1.888 | 1.024 | 0.398 / 0.374 |
| bulk / 1 / 512 / 1 | 0.912 / 1.248 | 0.864 / 1.120 | 1.344 / 1.888 | 0.643 | 0.921 / 0.921 |
| bulk / 1 / default / 1 | 1.120 / 1.440 | 0.976 / 1.248 | 1.568 / 2.240 | 0.622 | 0.904 / 0.945 |
| normal / 0 / 32 / 1 | 3.968 / 6.400 | 3.968 / 6.400 | 1.440 / 2.112 | 2.844 | 1.000 / 1.012 |
| normal / 0 / 512 / 1 | 1.888 / 3.008 | 1.888 / 3.008 | 1.408 / 1.952 | 1.405 | 0.968 / 0.894 |
| normal / 0 / default / 1 | 1.984 / 3.328 | 1.856 / 2.880 | 1.408 / 2.048 | 1.349 | 0.967 / 0.914 |
| normal / 1 / 32 / 1 | 3.456 / 5.504 | 1.664 / 2.432 | 1.568 / 2.304 | 1.118 | 0.462 / 0.422 |
| normal / 1 / 512 / 1 | 1.184 / 1.600 | 1.056 / 1.440 | 1.344 / 1.792 | 0.756 | 0.944 / 1.106 |
| normal / 1 / default / 1 | 1.216 / 1.760 | 1.056 / 1.376 | 1.408 / 1.888 | 0.708 | 0.820 / 0.796 |

Raw trials, controls and ranges: [ph3-pread-read/SUMMARY.md](ph3-pread-read/SUMMARY.md).

## Warm latest-ten: pread

| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk / 0 / 32 / 1 | 2.880 / 5.376 | 3.200 / 4.992 | 2.624 / 4.352 | 1.200 | 0.971 / 0.894 |
| bulk / 0 / 512 / 1 | 2.944 / 6.016 | 3.072 / 5.888 | 2.560 / 4.992 | 1.211 | 0.952 / 0.955 |
| bulk / 0 / default / 1 | 3.008 / 4.736 | 3.200 / 5.120 | 2.752 / 4.480 | 1.190 | 0.913 / 0.992 |
| bulk / 1 / 32 / 1 | 3.008 / 5.888 | 3.072 / 5.376 | 2.560 / 4.224 | 1.211 | 0.941 / 0.916 |
| bulk / 1 / 512 / 1 | 2.816 / 4.736 | 2.880 / 4.864 | 2.432 / 3.840 | 1.184 | 1.013 / 1.061 |
| bulk / 1 / default / 1 | 3.136 / 5.504 | 3.328 / 6.016 | 2.624 / 4.608 | 1.182 | 0.956 / 1.016 |
| normal / 0 / 32 / 1 | 3.520 / 5.888 | 3.520 / 6.016 | 2.432 / 4.224 | 1.421 | 0.904 / 1.005 |
| normal / 0 / 512 / 1 | 3.392 / 6.016 | 3.200 / 5.760 | 2.368 / 3.776 | 1.429 | 0.933 / 0.982 |
| normal / 0 / default / 1 | 3.520 / 6.400 | 3.584 / 5.760 | 2.432 / 4.224 | 1.474 | 0.974 / 1.056 |
| normal / 1 / 32 / 1 | 3.392 / 6.400 | 3.008 / 5.632 | 2.624 / 4.736 | 1.178 | 0.793 / 0.731 |
| normal / 1 / 512 / 1 | 3.264 / 5.376 | 3.328 / 5.504 | 2.368 / 3.712 | 1.333 | 0.972 / 1.033 |
| normal / 1 / default / 1 | 3.264 / 5.504 | 3.136 / 4.992 | 2.624 / 4.480 | 1.186 | 0.830 / 0.781 |

Raw trials, controls and ranges: [ph3-pread-read/SUMMARY.md](ph3-pread-read/SUMMARY.md).

## Cold-start point: pread

Default/512 index-on rows retain heap values.

| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk / 0 / 32 / 1 | 7.168 / 434.176 | 6.912 / 450.560 | 5.248 / 1179.648 | 1.350 | 0.976 / 1.019 |
| bulk / 0 / 32 / 4 | 7.040 / 540.672 | 7.040 / 507.904 | 5.248 / 3735.552 | 1.341 | 1.000 / 0.984 |
| bulk / 0 / 512 / 1 | 7.552 / 557.056 | 7.168 / 466.944 | 5.248 / 1277.952 | 1.366 | 0.968 / 0.850 |
| bulk / 0 / 512 / 4 | 7.040 / 1114.112 | 7.296 / 1376.256 | 5.504 / 4980.736 | 1.436 | 1.070 / 1.120 |
| bulk / 0 / default / 1 | 7.040 / 417.792 | 7.296 / 425.984 | 5.120 / 1179.648 | 1.415 | 1.018 / 0.983 |
| bulk / 0 / default / 4 | 7.424 / 557.056 | 7.040 / 540.672 | 5.376 / 4063.232 | 1.310 | 0.948 / 0.938 |
| bulk / 1 / 32 / 1 | 7.296 / 458.752 | 3.584 / 417.792 | 5.376 / 1245.184 | 0.683 | 0.491 / 0.963 |
| bulk / 1 / 32 / 4 | 7.296 / 540.672 | 3.264 / 434.176 | 5.248 / 3735.552 | 0.634 | 0.456 / 0.849 |
| bulk / 1 / 512 / 1 † | 1.120 / 1.376 | 0.912 / 1.152 | 5.248 / 1212.416 | 0.170 | 0.781 / 0.813 |
| bulk / 1 / 512 / 4 † | 1.152 / 1.408 | 0.960 / 1.344 | 5.120 / 3538.944 | 0.184 | 0.833 / 1.013 |
| bulk / 1 / default / 1 † | 1.120 / 1.376 | 0.912 / 1.152 | 5.120 / 1114.112 | 0.178 | 0.814 / 0.791 |
| bulk / 1 / default / 4 † | 1.120 / 1.408 | 0.912 / 1.184 | 5.120 / 3538.944 | 0.181 | 0.829 / 0.834 |
| normal / 0 / 32 / 1 | 8.448 / 417.792 | 8.448 / 417.792 | 5.120 / 1212.416 | 1.600 | 0.923 / 0.974 |
| normal / 0 / 32 / 4 | 8.064 / 507.904 | 8.448 / 557.056 | 4.992 / 3866.624 | 1.692 | 1.052 / 1.104 |
| normal / 0 / 512 / 1 | 7.424 / 466.944 | 7.680 / 466.944 | 4.992 / 1114.112 | 1.525 | 1.068 / 1.022 |
| normal / 0 / 512 / 4 | 8.704 / 589.824 | 8.960 / 655.360 | 5.504 / 4194.304 | 1.600 | 1.016 / 1.179 |
| normal / 0 / default / 1 | 7.808 / 475.136 | 7.680 / 466.944 | 5.120 / 1114.112 | 1.475 | 0.968 / 1.006 |
| normal / 0 / default / 4 | 7.808 / 540.672 | 7.680 / 540.672 | 5.248 / 3801.088 | 1.463 | 1.000 / 1.034 |
| normal / 1 / 32 / 1 | 7.552 / 434.176 | 4.864 / 393.216 | 5.248 / 1146.880 | 0.927 | 0.677 / 0.938 |
| normal / 1 / 32 / 4 | 7.424 / 499.712 | 4.224 / 491.520 | 5.376 / 3932.160 | 0.786 | 0.576 / 1.034 |
| normal / 1 / 512 / 1 † | 1.472 / 1.952 | 1.184 / 1.664 | 5.504 / 1212.416 | 0.216 | 0.810 / 0.909 |
| normal / 1 / 512 / 4 † | 1.504 / 2.048 | 1.216 / 1.632 | 5.504 / 3866.624 | 0.215 | 0.787 / 0.751 |
| normal / 1 / default / 1 † | 1.408 / 1.920 | 1.120 / 1.632 | 5.248 / 1146.880 | 0.213 | 0.795 / 0.784 |
| normal / 1 / default / 4 † | 1.408 / 1.920 | 1.120 / 1.600 | 5.248 / 3538.944 | 0.219 | 0.815 / 0.890 |

† Retained verified heap backing; file-page eviction does not make these values cold.

Raw trials, controls and ranges: [ph3-pread-cold/SUMMARY.md](ph3-pread-cold/SUMMARY.md).

## 10k writes

Each trial starts from a fresh 10k-record fixture and uses 10,000 operations per harness stage, both fast and durable modes, with index off/on. The write helper keeps its existing default cache/mapping settings; these populations fit the cache. Remember and update each perform 10,000 writes; mixed-90-10 performs 1,000 writes and 9,000 reads, with separate histograms retained in raw receipts. Flush/sync/final verification are outside call timers and retained in completion receipts.

| Index / durable / stage | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: |
| 0 / 0 / mixed-90-10 | 0.800 / 2.368 | 0.864 / 2.368 | 0.976 / 8.704 | 1.033 / 0.971 |
| 0 / 0 / recall-by-id | 0.848 / 1.920 | 0.880 / 2.048 | 0.944 / 1.248 | 1.001 / 1.100 |
| 0 / 0 / recent-for-agent | 3.328 / 5.504 | 3.520 / 5.760 | 2.880 / 4.736 | 0.965 / 0.872 |
| 0 / 0 / remember | 0.784 / 6.016 | 0.768 / 6.016 | 7.552 / 12.032 | 0.980 / 1.044 |
| 0 / 0 / update | 1.088 / 6.656 | 1.120 / 6.656 | 7.936 / 11.008 | 1.000 / 1.029 |
| 0 / 1 / mixed-90-10 | 1.728 / 655.360 | 1.856 / 688.128 | 1.888 / 1703.936 | 1.035 / 0.928 |
| 0 / 1 / recall-by-id | 0.880 / 1.984 | 1.008 / 2.240 | 0.896 / 1.472 | 1.293 / 1.319 |
| 0 / 1 / recent-for-agent | 3.136 / 4.992 | 3.904 / 7.936 | 2.688 / 4.224 | 1.297 / 1.809 |
| 0 / 1 / remember | 573.440 / 3080.192 | 589.824 / 7077.888 | 1245.184 / 9699.328 | 1.056 / 3.795 |
| 0 / 1 / update | 573.440 / 3276.800 | 589.824 / 5505.024 | 1310.720 / 11272.192 | 1.003 / 1.404 |
| 1 / 0 / mixed-90-10 | 0.800 / 1.600 | 0.832 / 1.600 | 0.992 / 8.704 | 1.040 / 1.044 |
| 1 / 0 / recall-by-id | 0.800 / 1.248 | 0.768 / 1.184 | 0.992 / 1.504 | 0.924 / 1.011 |
| 1 / 0 / recent-for-agent | 3.264 / 6.144 | 3.520 / 6.400 | 2.944 / 4.608 | 0.968 / 1.028 |
| 1 / 0 / remember | 0.784 / 6.144 | 0.784 / 6.400 | 8.064 / 13.312 | 0.980 / 1.062 |
| 1 / 0 / update | 1.088 / 6.656 | 1.184 / 7.424 | 7.936 / 11.264 | 1.088 / 1.141 |
| 1 / 1 / mixed-90-10 | 1.760 / 1146.880 | 1.632 / 819.200 | 1.792 / 2359.296 | 0.878 / 1.490 |
| 1 / 1 / recall-by-id | 0.784 / 1.472 | 0.720 / 1.376 | 0.896 / 1.504 | 0.875 / 1.396 |
| 1 / 1 / recent-for-agent | 3.008 / 5.888 | 3.072 / 5.632 | 2.624 / 4.032 | 0.937 / 1.213 |
| 1 / 1 / remember | 573.440 / 6291.456 | 589.824 / 4849.664 | 1245.184 / 11272.192 | 0.937 / 1.066 |
| 1 / 1 / update | 589.824 / 5767.168 | 557.056 / 3014.656 | 1245.184 / 11010.048 | 0.971 / 0.547 |

| Index / durable | Main end sync+flush+verify ms | PH3 ms | LMDB main-control ms | LMDB candidate-control ms |
| --- | ---: | ---: | ---: | ---: |
| 0 / 0 | 97.138 | 98.131 | 63.305 | 64.021 |
| 0 / 1 | 80.310 | 85.203 | 23.443 | 23.505 |
| 1 / 0 | 56.950 | 58.245 | 50.211 | 50.737 |
| 1 / 1 | 112.325 | 100.016 | 22.404 | 22.470 |

### Write confirmation

A second five-pair block uses the same active database path for both labels. It confirms fast indexed insert/update controls and repeats the uncertain unindexed durable case. The original run is retained in full; neither run replaces the other.

| Index / durable / stage | Confirm main p50/p99 | Confirm PH3 p50/p99 | LMDB p50/p99 | Adjusted PH3/main p50/p99 |
| --- | ---: | ---: | ---: | ---: |
| 0 / 1 / remember | 557.056 / 5636.096 | 557.056 / 3932.160 | 1245.184 / 9699.328 | 1.013 / 0.902 |
| 0 / 1 / update | 557.056 / 3538.944 | 573.440 / 3932.160 | 1245.184 / 11272.192 | 1.002 / 1.140 |
| 1 / 0 / remember | 0.608 / 4.736 | 0.592 / 4.736 | 5.888 / 9.472 | 0.974 / 1.000 |
| 1 / 0 / update | 0.848 / 5.504 | 0.816 / 5.248 | 6.272 / 8.704 | 0.962 / 0.953 |

The fast indexed update slowdown does not reproduce: confirmation p50/p99 improves 0.848/5.504 to 0.816/5.248 us. Durable unindexed insert p99 changes direction (original 3.080 to 7.078 ms; confirmation 5.636 to 3.932 ms). Durable update confirmation p50 stays close (557 to 573 us), but p99 remains higher (3.539 to 3.932 ms; adjusted ratio 1.140). The host data do not establish a durable-write p99 no-regression guarantee. All stored values and final 20k-record states verify. This uncertainty is separate from the passing warm read target and passing correctness gates.

[Write trials and ranges](ph3-write/SUMMARY.md), [confirmation and ranges](ph3-write-confirm/SUMMARY.md).

## Profiling

Round 2 already rejected verified mapped points: bulk/32 improved 3.840/6.528 to 2.432/3.584 us and normal/32 4.096/7.296 to 2.688/4.352, but all remained behind LMDB; normal/512 p50 rose 2.368 to 2.688 us. Its four-reader cold mapped p99 was 2949.120 us versus pread 589.824 us in the same prototype. Those prior results use CPU 3 and are historical, not comparable absolute timings for this CPU-6 campaign. This implementation retains full verified heap backing when the dataset fits, uses resident exact locators when it does not, and checks every returned external value rather than caching a per-block verification bit. The new cold/larger-cache matrices are required to judge the old tradeoffs. See [round-2 rejection](../read-path-results/round2/REJECTED.md).

The original 32 MiB sorted path used about 9,916/10,432 user cycles and 13,881/15,447 instructions per bulk/normal operation, versus LMDB 5,154/5,614 cycles and 7,893 instructions. Exclusive samples placed about 13% in cache insertion, 8–13% in block loading and 5% in block CRC, with routing/index/seek work in the inlined getter. See baseline-profile and profile-summary.json.

The rejected PH2-derived path already used per-value CRCs, derived at open. Its remaining mapped profile put 18–22% of samples in memmove and about 2–3% in CRC, with locator reconstruction in the inlined getter. Normal routing sampled at 11.86%. PH3 removes key reconstruction, and the L0 prefix removes a topology-sized filter walk. These samples identify hot code; they are not exact nanosecond attribution. DWARF reconstruction was incomplete and perf lost samples; raw coverage/loss receipts are preserved. User-only counters exclude kernel syscall instructions.

Full-path final counters and stacks: ph3-pread-profile and ph3-mapped-profile. Syscall interposition is diagnostic only: it counts positioned-read calls/bytes/time inside the perf FIFO operation boundary and includes clock overhead. It is never enabled for latency trials. The isolated component results overlap and must not be added into a full-read prediction.

| Final profile / mode | Loop mean us | User cycles/op | Instructions/op | LLC misses/op | Branch misses/op |
| --- | ---: | ---: | ---: | ---: | ---: |
| before-bulk-kiban / pread | 3.647 | 9744.0 | 13881.2 | 4.741 | 5.959 |
| before-bulk-lmdb / pread | 1.403 | 5099.3 | 7893.4 | 12.099 | 10.069 |
| before-normal-kiban / pread | 3.160 | 8156.9 | 15559.1 | 2.622 | 13.094 |
| before-normal-lmdb / pread | 1.227 | 4718.3 | 7893.4 | 9.827 | 10.041 |
| ph3-bulk-kiban / pread | 1.245 | 3052.6 | 6429.6 | 2.036 | 1.983 |
| ph3-bulk-lmdb / pread | 1.393 | 5143.0 | 7893.4 | 12.034 | 11.139 |
| ph3-normal-kiban / pread | 1.833 | 5035.8 | 7349.4 | 3.590 | 5.705 |
| ph3-normal-lmdb / pread | 1.353 | 5967.3 | 7893.4 | 13.681 | 11.864 |
| before-bulk-kiban / mapped | 2.775 | 12003.2 | 13168.1 | 41.563 | 5.161 |
| before-bulk-lmdb / mapped | 1.382 | 5988.0 | 7893.4 | 14.294 | 10.089 |
| before-normal-kiban / mapped | 2.245 | 9615.3 | 14503.8 | 40.913 | 10.070 |
| before-normal-lmdb / mapped | 1.328 | 5867.5 | 7893.4 | 13.736 | 10.070 |
| ph3-bulk-kiban / mapped | 0.864 | 3839.8 | 5641.5 | 8.154 | 1.988 |
| ph3-bulk-lmdb / mapped | 1.294 | 5751.3 | 7893.4 | 12.864 | 11.083 |
| ph3-normal-kiban / mapped | 1.012 | 4485.5 | 6260.9 | 8.434 | 3.235 |
| ph3-normal-lmdb / mapped | 1.435 | 5387.8 | 7893.4 | 13.627 | 11.768 |

| Syscall probe / mode | pread calls / million | Returned bytes / million | Diagnostic pread ns/call |
| --- | ---: | ---: | ---: |
| before-bulk-kiban / pread | 896514 | 3777974885 | 1532.9 |
| before-bulk-lmdb / pread | 0 | 0 | 0.0 |
| before-normal-kiban / pread | 916538 | 3861264684 | 1981.6 |
| before-normal-lmdb / pread | 0 | 0 | 0.0 |
| ph3-bulk-kiban / pread | 1000000 | 296732188 | 519.3 |
| ph3-bulk-lmdb / pread | 0 | 0 | 0.0 |
| ph3-normal-kiban / pread | 1000000 | 296732188 | 456.3 |
| ph3-normal-lmdb / pread | 0 | 0 | 0.0 |
| before-bulk-kiban / mapped | 0 | 0 | 0.0 |
| before-bulk-lmdb / mapped | 0 | 0 | 0.0 |
| before-normal-kiban / mapped | 0 | 0 | 0.0 |
| before-normal-lmdb / mapped | 0 | 0 | 0.0 |
| ph3-bulk-kiban / mapped | 0 | 0 | 0.0 |
| ph3-bulk-lmdb / mapped | 0 | 0 | 0.0 |
| ph3-normal-kiban / mapped | 0 | 0 | 0.0 |
| ph3-normal-lmdb / mapped | 0 | 0 | 0.0 |

| Isolated unchanged-path component | ns/op |
| --- | ---: |
| bloom | 62.6 |
| index | 278.6 |
| verify-block | 600.5 |
| cache | 329.8 |
| hash | 30.1 |
| block-seek | 680.3 |
| copy | 25.2 |
| crc-value | 19.3 |

## Rejected experiments and gates

The private PH2-derived locator candidate is preserved only as experiments/ph2-locators.patch. Its final pread paired K/LMDB point p50 ratios were 1.065 bulk and 1.532 normal, missing the stricter target. Earlier bulk pilot p50/p99 improved 3.712/5.504 to 1.536/2.176 us, versus LMDB 1.344/1.824; near parity was explicitly rejected. Historical final-* directories, development receipts and old source hashes refer to that experiment, not this PH3 source.

The first frozen PH3 candidate, with the Bloom precheck retained, missed the full normal target: paired p50/p99 ratios 0.930/0.864, despite bulk 0.800/0.759. The pipeline stopped before cold/writes; all 240 warm commands remain in ph3-before-bloom-read. The direct no-Bloom pilot against that source reduced bulk paired p50 K/L from 0.857 to 0.732 and normal from 0.829 to 0.791. Its p99 ratios remain below LMDB, and all trials are retained in no-bloom-pilot. The earlier PH3 mapped pilot before the L0 change measured 1.152/1.536 us bulk versus LMDB 1.504/2.240 and 1.248/1.600 us normal versus LMDB 1.440/2.176; paired p50 ratios 0.795/0.886. It was superseded by the frozen full matrix above. A missing-SST classification failure in the earlier candidate was fixed at the admission prepass root, preserving DbError::Corrupt. It was a caught development regression, not an existing-engine integrity bug. decisions.tsv records the parent clarification and fix.

All 11 final-source gates passed: fmt, clippy -D warnings, debug and release workspace tests (353 library tests), fixed crash corpora, loom, site facts, readbench checks, and both 20,000-seed suites. New tests cover mixed PH2/PH3 flags, snapshots, tombstones, binary/empty keys, address widths, checksum metadata corruption with recomputed section CRC, pre-open backing corruption, post-open repeated mapped/pread value corruption, short pread, concurrent reads after unlink, address-space eligibility, and exact reservation release. See [gate receipts](ph3-checks/status.json), [source hashes](ph3-checks/source-hashes.json), [provenance](ph3-provenance.json), [design](DESIGN.md), and [reproduction](VERIFY.md).

Recommendation: keep mmap_max_level opt-in. Warm mapped point targets pass, but normal/32 single-reader file-page-cold p99 rises 557.056 to 720.896 us versus main (about 29%), while its four-reader p99 improves 2686.976 to 2097.152 us. Both remain below their LMDB controls. The single-reader candidate reads 172,793,856 storage bytes versus main 146,546,688; admission retention and changed SST topology differ, so this is an observed tradeoff rather than an isolated causal attribution. Across modes, the candidate four-reader cold p99 is also higher with mmap: bulk 1540.096 us versus pread 434.176 us, normal 2097.152 us versus pread 491.520 us (about 3.5x and 4.3x). The existing option lets deployments choose this warm-page-cache advantage explicitly, and the report separately measures pread. Open-time validation, mixed old tables, and snapshot-dependent admission also argue against a default change. The complete acceptance decision and any other material controls are recorded in decisions.tsv.
