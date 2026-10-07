# Kiban v0.1.0 release procedure

This release preserves the corrected engine and storage formats. All required
checks must pass on the final commit before tagging. Publication is a separate
step: a local tag, remote tag, GitHub Release and crates.io version are distinct
states. Do not treat a dry run as publication.

## Benchmark reproduction

```sh
python3 validation/release_bench.py \
  --output target/release-baseline \
  --tmpdir target/release-benchmark-tmp
```

Use new output paths. The script runs the existing full `basic`, `steady_state`
and `phase_changing` benches serially, refuses the quick-mode environment flag,
and explicitly redirects TMPDIR to the selected disk filesystem. On this host
`/tmp` is tmpfs, which is inappropriate for storage measurements. It records
commit, compiler, profile, OS, CPU, memory, filesystem, source/configuration
snapshots, commands, exit status and all existing throughput/latency/WA output.
The files under `validation/release-results` are excluded from the crate package.

The basic bench's memtable-only control previously gave overlapping writers
independent sequence ranges, violating the memtable's strictly ascending update
contract. It now assigns global sequences under the existing write lock. Key
selection, values, read/write counts, threads and sampling are unchanged. The
other workloads are unchanged. No engine optimization was made.

Recorded baselines are single trials on a shared, uncontrolled host. Disk-backed
benchmarks still benefit from the Linux page cache; they do not establish
larger-than-RAM or cold-storage performance. WA counts the bytes supported by the
existing benches, not complete physical device writes.

## Post-correctness candidate baseline

Recorded on `095fe8432064e494cef598ad54a3e6babdf76e5d` (2026-10-07).
[Raw output and metadata](release-results/candidate/metadata.json) preserve the
complete full-suite trial and configuration sources. Compiler: rustc 1.98.1,
bench/release defaults, Fedora 44 Linux 7.2.8, Intel Core Ultra 9 185H (22 logical
CPUs), 32,258,724 KiB total RAM, encrypted Btrfs on NVMe with filesystem zstd
compression enabled. No engine compression is implied. The host is not isolated.

The steady-state cases use 300,000 writes and 300,000 reads, four writers and
four readers, 20,000 preloaded hot keys (64-byte values), 80-byte write values,
64 KiB write buffers/target SSTs, 256 KiB base level, multiplier four, L0 triggers
4/16 and 2 MiB block cache. The base inherits other defaults; each scheduler,
batch and pacing override is in the retained sources. The phase suite uses the
same tight maintenance configuration, six six-second phases, and two additional
20-second deep-debt windows. The basic suite uses five median samples, 20,000
write operations and 100,000 read operations for its standard cases, with
explicit case-specific sizes and threads retained in its source.

Measured default-governor results (throughputs below are per operation class,
not their sum):

| Workload | Writes/s | Reads/s | GET p99 / p99.9 µs | PUT p99 / p99.9 µs | WA | Avg SST probes/read |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Steady-state churn | 18,383 | 18,383 | 7.00 / 33.33 | 408.04 / 67,969.37 | 8.26× | 4.21 |
| Sequential | 5,714 | 5,714 | 7.08 / 31.42 | 425.18 / 37,824.27 | 29.05× | 3.61 |

Phase-changing governor: 7.81× WA, 2.83 average SST probes/read, L0 maximum eight,
2,102 write stalls, nine mode switches. In read-heavy recovery, GET p99 was
9.57 µs and PUT p99 was 3,052.30 µs. Full p50/p95/p99/p99.9 output is retained
where each existing bench supports it. The deep-debt case ended at 27.09× its
level budget: this trial does not establish debt convergence. Phase RSS includes
the benchmark's accumulated latency vectors, not just the engine.

Observations: read probes and hot-read throughput show no obvious collapse in
these workloads. Maintenance stalls and write tails remain substantial. The
scored batching control used 15.97× WA on sequential writes versus the governor's
29.05×; the governor is not universally better. No causal bottleneck attribution
or cold-storage/large-scale performance claim is made. Historical tmpfs runs
are not directly comparable to this disk-backed baseline.

A quick debug smoke benchmark briefly overlapped the initial candidate basic
controls; do not use those control timings for comparative claims. The remaining
candidate suites ran without that competing benchmark. The tagged commit must
be benchmarked again serially after the complete gate. Store its exact-SHA run
under `target/release-final-baseline` and gate under `target/release-final-gate`.
These separate final-run records avoid attempting to embed a commit's own SHA
inside its Git tree. The committed candidate trial is not mislabeled as a
measurement of the later documentation commit.

## Release gate

The local evidence runner executes the commands below and then the existing
1,000-cycle simulated and SIGKILL/reopen campaigns (seed 20261006, four threads,
snapshots enabled), stopping at the first failure:

```sh
python3 validation/release_gate.py \
  --output target/release-gate \
  --data target/release-gate-data
```

Run benchmarks separately from the gate to avoid competing workloads. Do not
rebuild example binaries during process torture: children spawn the running
executable by path. Use fresh data/output directories.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features loom -- -D warnings
cargo test
cargo test --examples
cargo test --features loom --lib -- --test-threads=1 loom_
cargo test --release --lib -- --ignored --nocapture
cargo run --example basic -- target/release-example
cargo package
cargo package --list
cargo publish --dry-run
```

Preserve package source, examples and required tests. The recovery integration
tests import torture modules under examples; keep those modules in the crate.
Generated validation artifacts are intentionally excluded, not deleted from the
repository. CI runs the correctness/package gate on Linux. Full benchmark and
1,000-cycle torture evidence are recorded locally with explicit configurations.

No v0.1.0 tag may already exist. Check local and remote tags and crates.io name
status, then create the annotated tag only against the validated final commit:

```sh
git tag -a v0.1.0 -m "Kiban v0.1.0"
git rev-parse v0.1.0^{}
git push origin v0.1.0
gh release create v0.1.0 --verify-tag --title "Kiban v0.1.0" --notes-file RELEASE_NOTES.md
cargo publish
```

Do not force-move a tag or retry a failed publication without inspecting it.
GitHub and crates.io require working authentication. After publication, test
`cargo add kiban`, `cargo check` and the documented minimal program in a clean
temporary project. Announce registry installation only after that succeeds.

## Remaining limits

See CORRECTNESS.md: MANIFEST has no checksum; snapshots retain captured state
conservatively; independent owners of one directory are not validated; SIGKILL
is not machine power loss; device and concurrency models have limited coverage.
API and format compatibility are not promised before 1.0. No production-readiness
or formal-correctness claim is made.
