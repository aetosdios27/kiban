Each engine patch applies independently to `0367f8d`, the cache-budget
commit. They are alternative implementations, not a patch stack. The retained
engine, if any, is identified by the final report; a patch's presence here
does not mean it shipped.

Rebuild a candidate in this worktree after saving other WIP:

```sh
git apply validation/read-path-results/round2/experiments/<candidate>.patch
taskset -c 2-5 cargo build --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
```

Frozen binaries remain under `target/read-path-tools/round2-*`, with SHA-256
identities in every benchmark group's `metadata.json`. Payload validation,
operation/record counts, CPU affinity and complete-command counts are audited
by `tools/agentbench/readpath/round2_report.py`. Timed readers use CPU 3;
builds and gates use 2–5. Avoid overlapping local builds/tests and measurements.

`reuse-slots.patch` is an unmeasured sketch of binding only on cache reuse. It was saved before switching to the mapped experiment; no result is claimed for it.
