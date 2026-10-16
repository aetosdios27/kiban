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

## One-command campaign

On Linux x86_64 with Git, Python 3, a C toolchain/OpenSSL development headers and
Rust installed, run from the repository root:

```sh
python3 tools/agentbench/run.py --install-helix \
  --output target/agentbench-results/first --n 10000 --operations 10000
```

This downloads the official Helix CLI v2.3.5 release binary, verifies its SHA-256,
clones the matching source tag and checks commit
`17e7ecf764aecd553e1f54ca25320d654153a9aa`, compiles the checked-in HelixQL with
that CLI, and builds the server with `cargo build --locked --release`. It uses
upstream's release profile (opt-level 2, LTO, one codegen unit), versus Kiban's
release opt-level 3/no LTO. It uses the official CLI's standalone binary/source
workflow rather than requiring Docker, Podman or a cloud login. In this CLI,
`helix build --bin` invokes a debug Cargo build; the installer explicitly selects
release. All commands and build logs are retained. Initial builds may take
several minutes. No writes go to the Kiban core or `main`.

Installation lives under `target/agentbench-install`. Subsequent campaigns can
omit `--install-helix`; source-query and binary hashes must still match the
installation manifest. A changed HelixQL query requires installing/building
again. The root Kiban dependency is a local path, intentionally measuring the
checkout rather than the historical Kiban revision pinned by `tools/compare`.
Cargo.lock pins the standalone harness dependency graph. No dependencies are
added to the published `kiban` crate; `cargo deny check` still checks the main
crate separately.

The runner refuses an existing output directory or occupied port (default
16969). It leaves all databases under the results directory; nothing is deleted
automatically. It performs five serial repetitions in Kiban/Helix alternating
order. Each side/trial first runs the entire suite on a separate warmup database
(default 100 records and calls per stage), which is excluded from measurements.
The measured databases start empty; W2/W3 use the records populated by W1, so
those are warm, RAM-fitting reads. W4/W5 operate on the same population. A fresh
client and server are created for each trial. Connection readiness makes a
validated Noop request before timing; HTTP keep-alive connections are reused
within each process. The floor makes one additional validated Noop on its own
client connection before timing. Concurrency is one; no pipelining, batching, retries or
client-side recent-list sorting occurs.

`--n` controls population/W1 inserts; `--operations` controls each of W2–W5 and
the transport floor. Operations must be a multiple of ten, giving W5 exactly
90% reads/10% overwrites with random existing IDs. The seed, record IDs, agent
assignment and access trace are the same in all trials and backends. For a
roughly one-minute suite, first run a small explicitly preliminary pilot, inspect
the sum of stage `wall_seconds` for the slower path, and scale both counts by
`60 / observed_seconds`. Keep counts equal across systems. This is an operation
budget, not a claim that embedded and HTTP paths each take a minute. Increase
`--operations` for slow-tail study; at least 10,000 samples are needed for p999.
Use `--runs` to increase repetitions, never below five.

Load is sampled once per second and at command boundaries. The default
`--load-window all` uses the maximum 1/5/15-minute average. To gate on current
one-minute load while historical averages decay, pass `--load-window 1m`.
If the selected initial load is >=2, timing is refused unless
`--allow-preliminary` is explicit. If any selected load sample during foreground
work is >=2, the entire report is labeled preliminary. All three averages are
retained and reported regardless of the selected gate. Builds finish before measured trials. CPU-bound
background jobs are not killed, paused or reniced. Suspend/clock discontinuities
above two seconds, per-command timeout, HTTP errors, malformed responses, missing
records and wrong values invalidate the campaign. The default command timeout is
3,600 seconds, including builds, and can be changed with `--timeout`.

After each suite, Kiban is flushed/synced and both backends verify all final
records plus every agent's newest list. The embedded process then exits and
reopens; the Helix server is stopped and restarted on the same data directory.
All final records/lists are verified again in a separate process. This checks
persistence across process restart; it is not a sudden-power-loss test. The
server normally receives SIGTERM, with bounded cleanup. Per-write read-back
checks catch even an intermediate overwritten value being stored incorrectly.

## Fairness and query shape

Embedded and transport latency are separate rows, with no combined ranking or
speedup. The generated report includes a **Kiban server: not implemented**
placeholder. All local data directories share the host filesystem, whose mount
options and devices are recorded. Hardware, kernel, memory/swap, Rust versions,
Git state, compiled-source snapshots, binary/lock hashes, exact command arrays,
parameters and load samples accompany each campaign. No global cache clearing,
CPU governor changes or CPU isolation are performed. Helix includes its native
runtime/index overhead and default telemetry settings; these are not stripped
from the server.

| System/path | Acknowledgement/durability | Record/query representation |
| --- | --- | --- |
| Kiban embedded | `Kiban::put` + `sync` after every write, default engine options | serialized record; primary key `(agent, inverted creation sequence)`; range `take(K)` |
| Kiban server | **not implemented** | placeholder only |
| Helix HTTP | one HelixQL request/write transaction, LMDB default synchronous commit; no `NO_SYNC`/`NO_META_SYNC` flags | indexed memory ID; indexed Agent node with Remembers edges; adjacency + descending sort + range |
| Helix transport floor | no writes | `POST /Noop {}` on the same host, port, HTTP client and server |
| Hydra hosted | not measured; durability not established | public API capability limitations below |

The Helix floor includes JSON, routing and an empty read transaction; it is an
upper-bound proxy for pure transport overhead, not a kernel-only measurement.
It returns a checked constant and does not access graph records. Memory writes
also return a constant acknowledgement, with the full-value read-back outside
the write timer. Never subtract
percentiles to estimate engine time. Network/server serialization and query
processing remain part of the competitor client path.

Kiban's key layout serves both requested access patterns; Helix maintains an ID
index plus agent adjacency. The 64 Agent nodes are initialized outside timing
and retained in storage; each memory insertion creates its edge in the same
transaction. Helix's natural descending sort visits that agent's memories,
whereas Kiban stops its ordered range after K values. This difference is real
and visible, not an equal-algorithm guarantee. Timestamps are unique logical
creation sequences; updates leave them unchanged. ID encodes creation/agent, so
these are structured-key ID reads rather than opaque UUID secondary-index reads.

The initial direct non-unique `agent` index query failed to compile in v2.3.5:
its code generator produced a scalar return wrapper around a Vec. The graph
adjacency query avoids this without changing the compiler or substituting an
unindexed full scan. The checked-in queries implement W1–W4; W5 reuses Recall
and Update at the client, exactly 90/10, without an extra server-side loop.
Vector search, embeddings, semantic-memory quality, multi-client concurrency,
large/out-of-cache data, and throughput under concurrent requests are out of scope.

**Version scope:** v2.3.5 is deliberately the pinned HelixQL release. Current
Helix v3 uses a newer query/storage architecture and is not measured here.
These results must never be presented as current-v3 performance. The source
package internally calls itself helix-db 1.3.3; the release/CLI tag is v2.3.5.

## HydraDB capability slot (checked 2026-10-16)

No account was created and no credentials were entered. The optional backend
is gated by **`HYDRADB_API_KEY`**, with **`HYDRADB_ENDPOINT`** defaulting to
`https://api.hydradb.com`. When unset it records a skip without any request.
When set it performs only the documented authenticated `GET /databases`
capability probe (`Authorization: Bearer …`, `API-Version: 2`); response bodies
and keys are not logged. It still skips all workload timing for the following
specific API gaps. This is a capability slot, not a completed CRUD adapter.

The hosted v2 API/SDK is public: Python `hydradb-sdk>=2,<3`, TypeScript
`@hydradb/sdk@^2`, and HTTP. Memory ingest is asynchronous, and read/query
availability may require waiting for indexing. `POST /context/list` accepts IDs
and pagination but documents no explicit created-at descending sort contract;
`PATCH /context/{id}/metadata` updates metadata, not memory text. The published
spec has no documented in-place memory-text overwrite endpoint. Consequently,
W1 acknowledgement would not match the local durability/readiness contract,
W3 ordering is unestablished, and W4/W5 overwrite semantics cannot be faithfully
implemented. No documented same-host trivial Noop is exposed either. We do not
silently replace updates with delete/reinsert or exact reads with semantic
retrieval. A future faithful adapter can use the Backend contract once those
contracts are documented.

Signup asks for first name, last name, email, workspace subdomain and agreement
to terms/privacy. The signup page says creating a workspace is free and needs
no credit card. The marketing pricing section separately lists Cloud Ship at
$25/month minimum usage and $0.50/GB-month, Scale at $300/month minimum and
$0.25/GB-month, and graph construction at $5 per million ingested tokens. Do not
infer that a free workspace includes free benchmark capacity. The site now also
advertises an open-source self-hosted offering, but that is distinct from the
hosted API slot requested here; it was not installed or measured. The hosted
service also cannot meet the same-machine requirement.

Sources (primary, accessed 2026-10-16):

- [Helix v2.3.5 installation/HelixQL guide](https://github.com/HelixDB/helix-db/tree/v2.3.5#readme)
- [Pinned CLI binary/source build behavior](https://github.com/HelixDB/helix-db/blob/17e7ecf764aecd553e1f54ca25320d654153a9aa/helix-cli/src/commands/build.rs)
- [Pinned Helix LMDB environment flags](https://github.com/HelixDB/helix-db/blob/17e7ecf764aecd553e1f54ca25320d654153a9aa/helix-db/src/helix_engine/storage_core/mod.rs)
- [Current Helix API and local modes](https://github.com/HelixDB/helix-db#readme)
- [Hydra quickstart, API key and asynchronous indexing](https://docs.hydradb.com/get-started/v2/quickstart)
- [Hydra public OpenAPI v2 contract](https://docs.hydradb.com/api-reference/v2/openapi.json)
- [Hydra signup fields and no-credit-card notice](https://dashboard.hydradb.com/sign-up)
- [Hydra pricing and self-hosting](https://hydradb.com/#pricing)

## Output and validation

`REPORT.md` contains per-trial-metric medians with min–max ranges, separate
system/transport rows, p50/p99/p999/max and both throughput denominators.
`rN-SYSTEM.jsonl` is the raw per-trial stage data, including counts, latency
summaries, read/write distributions, verification time and completion markers.
`*-reopen.jsonl` records the separate full persistence verification. Warmup
JSONL/logs are explicitly named and excluded. `status.json` records invalidity,
errors and completed runs. No incomplete campaign is given a comparison table.
`commands.jsonl`, `load.jsonl`, `metadata.json`, generated Helix code/lock and
`sources/` retain reproduction evidence. Regenerate the table with:

```sh
python3 tools/agentbench/report.py RESULTS_DIRECTORY
cargo fmt --manifest-path tools/agentbench/Cargo.toml --check
cargo clippy --release --locked --manifest-path tools/agentbench/Cargo.toml --all-targets -- -D warnings
cargo test --release --locked --manifest-path tools/agentbench/Cargo.toml
python3 -m unittest discover -s tools/agentbench -p 'test_*.py'
cargo deny check
cargo package --list --allow-dirty
```

Tests drive the real Kiban suite/reopen path, deliberately corrupt a stored
value and require rejection, check latest ordering, and exercise child failure,
timeout cleanup and invalid-report suppression. Imported PRNG/histogram tests
also run. The harness is not a production database client library.

Externally interrupted campaigns can retain completed measurement units:

```sh
python3 tools/agentbench/run.py --resume --allow-preliminary --output RESULTS_DIRECTORY
```

Resume uses the original workload parameters and checks the exact benchmark
binaries, compiled sources and Helix inputs. It preserves original metadata and
source snapshots, records the resumption and current driver separately, skips
registered trials, and performs any missing persistence verification before
accepting a completed workload. It refuses partial workload files or campaigns
that recorded an actual failure; it never silently reruns or discards them.
An exclusive supervisor lock prevents duplicate resumed workers. Inspect and
stop any surviving original supervisor before resuming an older campaign that
predates the lock. Load-sampling gaps caused by the interruption remain gaps.
The report and `per-run-load.json` include observed 1/5/15-minute load ranges,
sample counts and coverage for every timed backend/floor run.

## Recorded campaign

The [first-results report](../../validation/AGENT_MEMORY.md) records the fresh
`final-10000-clean` attempt, its per-call durability costs, all five repetitions
and load during each run. It remains preliminary because measured load exceeded 2. [Compact raw evidence](results/final-10000-clean/) includes
per-run JSONL, configs, metadata, command/load logs, verification markers and
source/binary hashes. Databases, binaries, build logs and full source snapshots
remain under `target/agentbench-results/final-10000-clean`; they are not committed.
The earlier loaded-host development campaign remains under
`target/agentbench-results/first-10000` and is not pooled into these results.

The earlier preliminary raw evidence remains in [results/final-10000/](results/final-10000/).
[Results index](results/README.md) keeps the attempts separate; their numbers are never pooled.

## In-process engine comparison

The optional [engine harness](engine/README.md) links the pinned Helix query core
and raw heed3/LMDB alongside Kiban, with durable and fast modes, fixed P-core
affinity, and separate datasets. It has its own crate/lockfile and never starts
an HTTP server. Its results use a separate report and load rule; they are not
pooled with the HTTP study.

## Helix memory growth repro

The million-record engine study excludes Helix after bounded direct and HTTP
memory diagnostics reproduced growth outside the benchmark runner. The
[standalone client, exact query and setup instructions](engine/README.md#helix-memory-growth-repro)
include the BM25 configuration difference and links to raw RSS/query-count
samples. This HTTP diagnostic is separate from the in-process latency study;
no Helix engine patch contributes published numbers.

The [large-N cache-budget diagnosis](engine/README.md#large-n-cache-budget-diagnosis)
uses one separately frozen binary for Kiban's four cache sizes and a raw-LMDB
reference, with all foreground operations reading. Its W5 key-sequence projection
and separate SharedKiban counter replay are explicitly labelled.
