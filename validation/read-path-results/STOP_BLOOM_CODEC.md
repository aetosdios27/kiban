Historical stop report, captured before commit 3606fa0. The user ruled this issue harmless to stored data and authorized the fix and continuation. The reproduction now passes on the fixed source.

# Read-path investigation stopped on a correctness bug

The requested correctness stop condition triggered before profiling completed.
No engine change, performance fix, commit, push, or PR was made. Branch
`perf/read-path` starts at `082858f`, the main commit containing PR #10.
Benchmark tooling and evidence remain uncommitted for inspection.

## Reproduced Bloom filter codec bug

`BloomFilter::empty().encode()` cannot round-trip through
`BloomFilter::decode()`. The constructor allocates eight bytes for 64 bits,
while the decoder requires nine bytes for the same bit count. The normal
`build_hashes` path allocates nine bytes too.

- Constructor: `src/bloom.rs:51`, especially line 53.
- Decoder: `src/bloom.rs:94`, especially its exact-length check.
- Reference construction: `src/bloom.rs:36`, `num_bits / 8 + 1` bytes.

The public low-level constructor and codec disagree. SST construction uses
`build_hashes`, so this reproduction does not establish a database data-loss
bug or explain the read-performance gap. Engine code is unchanged.

Run from the repository root:

```sh
bash validation/read-path-results/reproduce-bloom-empty.sh
```

Observed output, from the unchanged release library:

```text
empty(): 17 encoded bytes; decode succeeds: false
build(empty): 18 encoded bytes; decode succeeds: true
assertion `left == right` failed: a valid empty filter must round-trip through its public codec
  left: None
 right: Some(BloomFilter { bits: [0, 0, 0, 0, 0, 0, 0, 0], num_bits: 64, num_probes: 7 })
```

The assertion exits unsuccessfully. [Reproduction source](read-path-results/bloom-empty-repro.rs)
and [captured output](read-path-results/bloom-empty-repro.log) are retained.
No fix is included because the task explicitly requires stopping on a
correctness bug.

## Completed baseline work

The 1M normal layout uses individual writes and flushes at the default
write-buffer budget. `Kiban::flush` runs its existing compaction policy.
It produces one L1 table, six L2 tables, and 77 L3 tables. The bulk layout
has one L0 table. At 10k, both methods fit in one L0 table. Every prepared
database was closed, independently reopened, and fully verified.

The new Helix-free driver reuses the existing W2/W3 backend calls, payloads,
verifier, random sequence, histogram, and release profile. Its dependencies
match the original engine harness lockfile, including `heed3 0.22.0`,
`lmdb-master3-sys 0.2.5`, `mimalloc 0.1.48`, `libmimalloc-sys 0.1.44`, and
`serde_json 1.0.143`. These details matter because an initial preload made
with a newer LMDB dependency was rejected by the pinned library with
`MDB_VERSION_MISMATCH`. Those LMDB databases were recreated. The aborted
setup trial is retained under `invalid-new-lmdb-format` and excluded.

Latency runs alternate Kiban and LMDB on CPU 3. Builds and correctness
checks use CPUs 2–5. The existing external crash sweep remains on E-cores
12–21 and was not modified. Load was about 22. CPU 3 was observed at
3.3 GHz during measurement, so absolute times differ from the earlier
head-to-head report. Comparisons use contemporaneous controls.

The read baseline completed 80 trials: five repetitions per engine for
each population, loading method, and cache budget. Each trial has 100,000
W2 calls and 100,000 W3 calls, zero timed writes, full payload checks, and
full verification before and after measurement. The driver also checks
that SST topology does not change. [All medians, ranges, and p99 values](read-path-results/BASELINE.md)
are generated from the [raw trial files](read-path-results/baseline).

This baseline does not show a win over LMDB. No before/after speedup or
write-regression claim is made. Hardware profiling was interrupted during
its first workload and its partial files must not be used as results.
The ranked cost attribution, flame graphs, microbenchmark results, proposed
fix rankings, write checks, and implementation remain unfinished because
of the explicit correctness stop condition.

## Baseline correctness gates

Before any engine change, all requested gates passed:

- `cargo test` and `cargo test --release`.
- Workspace Clippy with warnings denied, and `cargo fmt --check`.
- Both fixed crash corpora, including explicit release runs.
- The direct and shared long suites, each with seeds `0..20000` in release
  mode and one test thread. Together they checked 160,000 simulated crashes.
- The existing loom models, with one test thread.
- `python3 tools/site_facts.py validate`.
- The separate readbench tests, Clippy, and formatting. Its tests check
  real values after reopen and reject altered revisions for both engines.

[Gate commands and statuses](read-path-results/profile-checks/status.json),
[source hashes](read-path-results/profile-checks/source-hashes.json), and
logs are retained. The root source hashes still match the tested baseline.
The existing test suite passes despite the codec reproduction above.
