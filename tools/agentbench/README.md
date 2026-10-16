# Agent-memory latency harness

This unpublished, standalone crate exercises Kiban through its in-process API.
It adds no dependencies to the core crate and is already excluded by the root
package's `tools/**` rule. It uses the working tree's Kiban, with source hashes
and the exact Git commit recorded by the campaign driver.

`tools/compare/src/backend.rs` is an engine-specific enum, not a backend trait.
Its maintenance counters, settlement rules, byte-oriented workload and RocksDB
build are tied to the embedded comparison. Network query systems need structured
records, server lifecycle and a transport-floor probe, so this is a sibling
rather than changing that historical harness. It reuses the comparison's bounded
histogram (`examples/scale/metrics.rs`) and deterministic PRNG.

The binary accepts a JSON config file. Build with:

```sh
cargo build --release --locked --manifest-path tools/agentbench/Cargo.toml
cargo test --release --locked --manifest-path tools/agentbench/Cargo.toml
```

Config fields: `backend` (`kiban`), fresh `db` path, `endpoint` (empty for Kiban),
`n` (at least 64), `operations`, `k`, `seed`, new JSONL `output` path, and
`verify_only` (false for the workload, true after reopening the same database).
The stages are insert, random ID reads, newest-for-agent, overwrite and 90/10
read/write. Values include 200 ASCII text bytes plus identity, agent, creation
sequence and revision. Creation order stays fixed across overwrites. The primary
key encodes agent and descending creation sequence; point IDs encode that same
sequence. No secondary index, vector search or embedding generation is measured.

Every write does put+sync, followed by a separate full-value read-back. Every
foreground read/list is checked; final checks verify the complete population and
all agents' newest lists. A mismatch aborts. A second process can verify the final
state after reopening. Foreground call latency excludes the separate write
read-back. Verified throughput includes it and all oracle/generator work;
call-only throughput divides call count by accumulated call duration. Post-run
flush/sync/verification is recorded separately. Raw stage JSONL retains p50, p99,
p999, exact maximum, counts and read/write distributions. p999 is absent below
10,000 samples. Percentile histogram bounds have <3.125% bucket error above 32 ns.
