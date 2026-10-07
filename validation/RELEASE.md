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

## Release gate

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
