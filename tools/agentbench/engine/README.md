# In-process engine head-to-head

This optional, standalone crate links Kiban, Helix v2.3.5 and the same `heed3`
0.22.0 / `lmdb-master3-sys` 0.2.5 used by that Helix release. It adds no core
Kiban dependencies. It exercises W1–W5 from the HTTP harness with the same
generator, seed, record fields, oracle, latency histogram and persistence checks.
It never starts an HTTP server, creates a socket or invokes a router.

The claim under test is whether these storage/traversal paths differ on the
specified single-client workloads at the same durability, dataset and CPU
affinity. The comparison includes physical data-layout costs. It cannot rank
general database capabilities, semantic retrieval, concurrent writers or cold
storage workloads.

## Reproduction

On this Intel hybrid-core Linux host, from the repository root:

```sh
python3 tools/agentbench/engine_run.py --install-helix \
  --output target/enginebench-results/h2h \
  --sizes 10000 --operations 10000 \
  --runs 5 --cpus 1-4 --driver-cpus 12-21
```

Omit `--install-helix` to reuse the verified installation from the HTTP study.
Installation uses the existing official CLI/source installer and does not run
the server. Its standard location, `target/agentbench-setup/helix-src`, is the
crate's path dependency. The driver refuses a different source commit or any
source/schema/query hash change. The source pin is
`17e7ecf764aecd553e1f54ca25320d654153a9aa` (HelixQL v2.3.5, internal crate
version 1.3.3). This is not a Helix v3 comparison.

The checked-in lockfile retains the pinned Helix dependency versions. The
driver builds with `--release --locked`, checks the generated adapter, snapshots
all measured inputs, and writes raw JSONL, process resources, load samples,
commands, metadata and `REPORT.md`. Every output directory and database is
fresh. Databases and full logs stay in ignored `target/`; compact results can
be archived separately without changing raw values. Regenerate a report with:

```sh
python3 tools/agentbench/engine_report.py target/enginebench-results/h2h
```

Each trial first loads N base records untimed, verifies all of them, closes the
engine, and verifies them again after reopening. Kiban uses WriteBatch chunks
of 10,000 records; Helix and LMDB each use one large write transaction, then an
explicit final sync. Bulk loading uses relaxed per-call durability. It never
acknowledges one million separate durable writes.

Timed W1 inserts fresh IDs starting at N. W2/W3/W5 run the configured operation
count; W4 uses the timed write count. In durable mode W1 and W4 are capped at
10,000 calls each; a 100,000-call mixed workload contains 10,000 writes. Fast
mode uses the full count for W1/W4. Thus the final population is N+10,000 for
the standard durable runs, N+100,000 for large fast runs, and 20,000 for both
small modes. Recent lists and random reads use that full resulting population.
The population and operation counts are stated beside each results table.

Small N has five alternating trials; large N has three, as directed to bound
the wall-clock budget. The smaller repetition count limits large-N confidence.
Helix W3 still visits and sorts all memories for the selected agent; the large
matrix is calibrated before starting rather than silently reducing that work.
The default per-process deadline is six hours; change `--timeout` if needed.

For staged delivery, finish small N first, then reuse the identical binary:

```sh
python3 tools/agentbench/engine_run.py --output target/enginebench-results/small \
  --sizes 10000 --operations 10000
python3 tools/agentbench/engine_run.py --output target/enginebench-results/large \
  --sizes 1000000 --operations 100000 --large-runs 3 --engines kiban lmdb \
  --reuse-binary-from target/enginebench-results/small
```

The first command builds once and records the executable hash. The second
checks that hash and all compiled inputs, snapshots the current driver/report scripts, and does not run a build.
Every engine and every trial uses the same executable. The large-N subset omits memory-blocked Helix; see the reproduction below.

## Included and excluded layers

| Layer | Kiban | Helix in process | Raw LMDB floor |
| --- | --- | --- | --- |
| Typed benchmark argument/result | Included | Included | Included |
| HTTP, routing, request body, response JSON | Excluded | Excluded | Excluded |
| Stored record serialization/deserialization | JSON codec | Helix native node/edge/property codec | Same JSON codec as Kiban |
| Index/key layout | Agent + descending immutable creation sequence primary key | Indexed Agent and Memory nodes, Remembers edge, adjacency and newest-first sort | Same ordered primary key as Kiban |
| Transactions and durability work | Per call | Per call, as generated | Per call |
| Engine allocation and caches | Included | Included, including per-query Bump arena | Included, mmap resident pages |
| Graph UUID generation and index/edge maintenance | Not applicable | Included | Not applicable |
| Read result materialization | Owned Memory records | Owned Memory records from native properties | Owned Memory records |
| Oracle and separate write read-back | Outside call latency; inside verified throughput | Same | Same |

Both KV backends derive the agent and creation sequence from the ID, so their
one ordered primary key supports both ID reads and W3. Updating a value leaves
its immutable creation key unchanged. No artificial secondary index is added.
Helix retains its natural graph representation, with an edge inserted for every
memory. Thus raw LMDB is a natural KV floor, not Helix without only its function
call overhead. Graph layout and traversal versus ordered KV storage are part of
the experiment; the two are not interchangeable engine internals.

Helix's public generated handlers accept `HandlerInput { request, graph }` and
unconditionally deserialize and serialize request/response JSON. Calling the
handlers unchanged would retain that layer. The authorized alternative is
implemented by `generate.py`: extract their arena, transaction, traversal and
commit statements verbatim, and replace only typed arguments and result
projection. It fails if the expected compiler output shape changes.

The original generated W3 storage body is retained exactly:

```rust
let memories = G::new(&db, &txn, &arena)
    .n_from_index("Agent", "agent_num", &data.agent)
    .out_node("Remembers")
    .order_by_desc(|val| val.get_property("created_at").cloned().unwrap_or(Value::Empty))
    .range(0, data.k.clone()).collect::<Result<Vec<_>, _>>()?;
```

Whitespace above is formatted for reading. `equivalence.json` retains the
byte-exact source segments and hashes for all five handlers. `src/generated.rs`
contains the unformatted extracted statements and the compiler's original
`config()` including schema, unique indexes and BM25/vector defaults. The adapter then
explicitly disables BM25 in the storage configuration: node insertion otherwise
automatically indexes all properties, although W1–W5 never query full-text search.
This documented tuning removes an unused capability from the comparison; no
required schema/index/query is changed. The earlier HTTP study kept the generated
BM25 default on, so its measurements also differ in this setting. Regenerate
or verify against the actual pinned CLI output:

```sh
python3 tools/agentbench/engine/generate.py
python3 tools/agentbench/engine/generate.py --check
```

For reads, projection clones the five benchmark fields to owned `Memory`
records before the generated read transaction commits. The internal graph UUID
and label used only by the JSON response are not formatted or returned. For
writes, the constant JSON `"ok"` acknowledgement is replaced by `()`. The
generated node creation, unique index lookup, edge creation, update traversal
and transaction commit are unchanged. The generator removes the request's Arc
clone as well. There is no task queue, async runtime or server concurrency in
this entry point. Helix's library still has gateway dependencies, but the bench
does not call them. This is a transport-free generated query core, not a public
Helix embedding SDK or a patched engine.

All three backends are selected at runtime in **one binary**. Helix's library
declares the global MiMalloc allocator, so it is shared by all three. The common
release profile uses upstream Helix settings: opt-level 2, LTO, one codegen unit,
panic abort, with debug information retained for diagnostics. Different compiler
profiles or allocators are not hidden in the backend comparison.

## Durability

| Mode | Kiban | Helix | Raw LMDB |
| --- | --- | --- | --- |
| durable | Default options; `put` followed by `sync()` inside every write timer | Default synchronous `graph_env` flags; generated write transaction commits once per call | Default synchronous environment; one write transaction/commit per call |
| fast | Default options; `put` without per-call `sync()` | `NO_SYNC | NO_META_SYNC`, set through the public environment API before foreground calls | `NO_SYNC | NO_META_SYNC`, set before foreground calls |

Kiban's `sync()` flushes its WAL userspace buffer and calls `sync_data`
(`fdatasync` on Linux) for the active and any frozen WAL. Its maintenance can
still synchronize files in fast mode; fast does not promise zero sync syscalls.
With default flags, LMDB synchronizes dirty data pages and writes commit metadata
through a synchronous descriptor. These durable paths pay persistence work on
every acknowledged call, without cross-call batching; they do not require the
same number or kind of `fsync` syscall.

Helix `HelixGraphStorage::new` uses `EnvOpenOptions` without relaxed flags. It
does not expose these flags in the HelixQL configuration. Its public
`graph_env: heed3::Env` allows the benchmark to enable `NO_SYNC | NO_META_SYNC`
through `Env::set_flags` without editing Helix source. This is a benchmark-level
environment setting, not a claim that the normal Helix server offers a fast-mode
switch. No transaction or worker is active when the exclusively owned environment
is configured. The exact runtime flag bits and map size are recorded in every
process's resources JSON and verified against the requested mode. Neither
WRITEMAP nor MAP_ASYNC is enabled. Both LMDB paths use a 10 GiB map, 200 database
slots and 200 reader slots, matching the generated Helix storage defaults.

After foreground work, each backend explicitly synchronizes before final full
verification and orderly close. This one-time sync, Kiban flush and complete
post-run/reopen checks are outside all foreground latency/throughput numbers.
The fast-mode reopen check proves orderly persistence after that final sync; it
does not establish a power-loss guarantee. No power-loss test is part of this
experiment.

## Affinity, load and verification

This Core Ultra 9 185H exposes P-core CPUs `0-11` through
`/sys/devices/cpu_core/cpus`, and E/LP-E CPUs `12-21` through `cpu_atom/cpus`.
`lscpu -e` identifies CPUs 1/2 as physical core 1 and 3/4 as physical core 2.
Every engine and its reopen verifier runs under `taskset -c 1-4`; the Python
driver is restricted to `12-21`. Threads inherit affinity. Metadata retains the
topology and actual benchmark affinity; these are two physical P-cores including
both SMT siblings. Choose suitable sets for another host. This does not reserve
those cores or change other desktop processes' affinity, frequency or priority.

The order is Kiban, Helix, LMDB for every mode/trial. Durable and fast triples
alternate, for five repetitions at small N and three at large N. No engines run
concurrently. Before the first trial the driver waits up to 120 seconds for
one-minute load below 3 (`--wait-seconds` configures this startup bound). Acceptance uses **each trial's median** one-minute load
below 3, with complete sampling; maxima are still reported. A maximum above 3
does not itself fail the trial. If any trial median or coverage fails, the
affected set and campaign remain preliminary, with every trial retained.
Otherwise the label is **measured on a shared desktop, pinned cores**.

Load samples are recorded every 200 ms, plus actual engine-side boundary samples
for short fast-mode trials. Engine timestamps exclude the discarded
100-record/100-call warmup database; the load interval includes the foreground
suite and final in-process verification. The engine open and separate reopen
process are outside that interval. Gaps above 2.5 seconds fail coverage; clock
discontinuities, timeouts, command failures, wrong flags/affinity or any wrong
value invalidate the campaign. No comparison table is produced for invalid or
incomplete campaigns. A completed loaded trial is not silently replaced.

Every timed write is immediately read back field-for-field. Every untimed base
record is verified before closing and again after reopening. All foreground reads and
ordered lists are checked. The final population and all 64 agents' newest lists
are verified before closing and again in a new process. The seed is 20261016,
values contain 200 ASCII text bytes plus the original metadata fields, and K=10.
Larger N exceeds the default 4 MiB Kiban memtable and 32 MiB block cache; it can
still fit in this host's RAM and is not a cold/out-of-RAM experiment.

RSS is measured through `/proc/self/status`: end RSS and kernel high-water RSS
for the entire benchmark process, including bulk preload, the oracle revision vector,
allocator and resident mmap pages. Shared kernel page cache outside the process
is not included. Disk apparent and allocated bytes (`st_blocks * 512`) are
recorded after orderly close, before reopen. Btrfs compression affects allocated
space. No global caches are dropped.

Externally terminated supervisors can resume completed units with `--resume`
and the original output/CPU sets. An exclusive lock prevents duplicate drivers.
Source/binary hashes must still match. Completed raw data may receive a missing
reopen check; partial trials are retained and refused rather than automatically
rerun. A sampling gap still fails coverage. Do not use resume to hide a failed
measurement.

## Checks and excluded diagnostics

```sh
cargo test --release --locked --manifest-path tools/agentbench/engine/Cargo.toml
cargo clippy --release --locked --manifest-path tools/agentbench/engine/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path tools/agentbench/engine/Cargo.toml --check
python3 -m unittest discover -s tools/agentbench -p 'test_*.py'
```

Rust tests drive all three engines in both modes, verify after reopen, then
deliberately corrupt a value and require rejection. They exercise the bulk-load/reopen/timed-operation
plan in both modes. The HTTP harness still uses
the same runner, with its transport floor moved into a separate module. Its
historical result files are unchanged. Python tests exercise the load median,
warmup exclusion, affinity parser, error/timeout paths and invalid-report rule.

Separate syscall diagnostics and runtime calibration use:

```sh
python3 tools/agentbench/engine_probe.py --sync-profile \
  --output target/enginebench-probes/sync
python3 tools/agentbench/engine_probe.py --n 1000000 --operations 1000 \
  --engines helix --modes fast --output target/enginebench-probes/large-calibration
```

`sync_profile.c` interposes fsync/fdatasync and pwrite/pwrite64, counts calls and
time, and identifies writes through O_DSYNC/O_SYNC descriptors. It observes the
whole diagnostic process, including initialization, warmup and final sync. It
is not a full syscall tracer or CPU stack profiler, and can miss direct syscalls
that bypass those libc symbols. It helps confirm the durability path without
changing the engine. `LD_PRELOAD` diagnostics and their latency values are
excluded from the comparison campaign. Diagnostic rows must never be pooled
with the five uninstrumented measurement trials.

## Recorded results

[ENGINE_H2H.md](../../../validation/ENGINE_H2H.md) contains the small-N study,
all percentiles, throughput, RSS/disk, load conditions and interpretation.
[Raw small-N evidence](../results/engine-small-10000/) is kept separately from
the HTTP study. The large-N report records the Kiban/LMDB subset and the evidence for memory-blocked Helix separately.

## Helix memory growth repro

The large-N Helix calibration was stopped at a 6 GiB RSS bound, after the five
1,000-operation stages but during final verification. It is invalid as a
comparison trial. Before excluding Helix, a separate HTTP reproduction completed
1,000 W3 requests against the same pinned v2.3.5 / `17e7ecf` engine and a copy of
that 1,001,000-record database. Server RSS increased from 74.45 to 4,765.00 MiB;
the standalone Python client's RSS settled at 20.86 MiB. Every response was
checked and discarded. The growth therefore also occurs without our in-process
adapter, oracle, histograms or benchmark runner. No Helix engine patch was used.

The exact query is in [queries.hx](../helix/queries.hx):

```text
QUERY Recent(agent: U32, k: U32) =>
    memories <- N<Agent>({agent_num: agent})::Out<Remembers>::ORDER<Desc>(_::{created_at})::RANGE(0, k)
    RETURN memories
```

The minimal request/RSS client is [helix_memory_repro.py](../helix_memory_repro.py).
It uses only Python's standard library, works against an independently running
Helix server, keeps one response at a time, and streams query-count/RSS pairs.
It checks IDs, agent, creation sequence, revision type, text size, list count
and ordering; it is a memory diagnostic, not another latency trial or a complete
benchmark value oracle. Its 6 GiB RSS limit stops requests; it never kills an
externally owned server. Stop that server yourself when the client exits.

A bounded seed can be made with the already built binary (no rebuild):

```sh
python3 tools/agentbench/engine_probe.py --n 1000000 --operations 1 \
  --engines helix --modes fast --output target/helix-memory-seed
mkdir -p target/helix-memory-http/data
cp -a --reflink=auto target/helix-memory-seed/data/fast-helix \
  target/helix-memory-http/data/user
```

That seed has 1,000,001 final records; use `--population 1000001` below. The
archived diagnostic used 1,000 insert operations instead and therefore has
1,001,000 records. Bulk load is one relaxed transaction plus final sync; all
seed values are verified before and after reopen. Keep the seed process within
available RAM, and use a fresh destination for every attempt.

The server appends `user/` to `HELIX_DATA_DIR`. Its generated default config has
BM25 enabled; opening this non-BM25 seed triggers a full BM25 rebuild and the
stock 10 GiB map fills before requests can run. For this diagnostic only, use
the same `bm25: Some(false)` setting as the engine study in the generated
`helix-container/src/queries.rs`, then build a separate server executable. This
changes one application configuration field, not Helix engine or query logic.
Preserve the installed executable and restore the generated file afterwards:

```sh
repo=target/agentbench-setup/project/.helix/dev/helix-repo-copy
cp "$repo/helix-container/src/queries.rs" target/helix-memory-http/queries.original.rs
cp "$repo/target/release/helix-container" target/helix-memory-http/server.original
python3 - <<'PY'
from pathlib import Path
p=Path('target/agentbench-setup/project/.helix/dev/helix-repo-copy/helix-container/src/queries.rs')
s=p.read_text()
assert s.count('bm25: Some(true)') == 1
p.write_text(s.replace('bm25: Some(true)','bm25: Some(false)'))
PY
cargo build --release --locked --manifest-path "$repo/helix-container/Cargo.toml"
cp "$repo/target/release/helix-container" target/helix-memory-http/server.bm25-off
cp target/helix-memory-http/server.original "$repo/target/release/helix-container"
cp target/helix-memory-http/queries.original.rs "$repo/helix-container/src/queries.rs"
sha256sum target/helix-memory-http/server.bm25-off
HELIX_DATA_DIR="$PWD/target/helix-memory-http/data" HELIX_PORT=16971 \
  taskset -c 1-4 target/helix-memory-http/server.bm25-off \
  >target/helix-memory-http/server.log 2>&1 &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true' EXIT
# Wait for POST /Noop {} to return agentbench-noop, then:
taskset -c 12-21 python3 tools/agentbench/helix_memory_repro.py \
  --server-pid "$server_pid" --population 1000001 --queries 1000 \
  --output target/helix-memory-http/client
kill "$server_pid"
wait "$server_pid" || true
trap - EXIT
```

The configured server is outside the frozen benchmark binary and never supplies
published engine latency numbers. The engine source tree, generated W3 core,
query text, config delta, server hash, all 1,001 RSS samples and server logs are
retained in [helix-memory-diagnostic](../results/helix-memory-diagnostic/).
Its HTTP W3 storage/traversal core was compared byte-for-byte to the frozen
in-process adapter. CLI-generated schema JSON/function ordering can differ;
the archive retains the actual HTTP source rather than assuming byte identity
of the entire generated file.

The direct adapter creates/drops its local Bump arena and traversal result Vec
on every call (`engine/src/generated.rs`, `recent`). It returns K owned Memory
values. `src/runner.rs` consumes `many` in the iteration's `if let` and drops it
before the next iteration. Only fixed-size revision/recent-ID oracles and
histograms persist; logs contain one row per stage, not response records.
The HTTP client explicitly deletes each decoded result and retains no list of
responses. Its stable RSS and independent server growth exclude accumulation
in those harness Vecs/logs as the explanation.

The likely allocation mechanism is pinned Helix's `ImmutablePropertiesMap`:
it is Copy, stores raw `Value` pointers allocated with `Bump::alloc_layout`,
and writes owning values with `values.add(index).write(value)`. `Value::String`
owns a heap String, while the map has no Drop implementation and Bump does not
run destructors for raw arena allocations. W3 decodes all roughly 15,640
neighbors before sorting/taking ten. This is source-supported attribution, not
a heap-profiler proof that every retained byte has that origin. Mapped pages
and allocator overhead also contribute to RSS, especially early in the curve.
Do not patch Helix or replace the query when reproducing the published study.

Large-N Helix is reported as **memory-blocked** in both modes, with no valid
latency/RSS/disk trial cells. The only direct calibration was fast mode; W3 is a
read-only query with the same allocation path in durable mode. A separate
100,000-call W3 projection was roughly 27 minutes per Helix trial, over 163
minutes across six trials before other work, independently exceeding the
90-minute full-matrix budget. Calibration timings are not comparison results.
The approved remaining large-N command is:

```sh
python3 tools/agentbench/engine_run.py --output target/enginebench-results/large-1000000-kv \
  --sizes 1000000 --operations 100000 --large-runs 3 --engines kiban lmdb \
  --reuse-binary-from target/enginebench-results/small-10000
```

A backend subset is explicitly recorded in metadata; a successful subset does
not imply Helix completed. Binary reuse checks every compiled Rust input,
manifest/lockfile, query schema and equivalence record against the first build.
Driver/report-only changes are snapshotted separately; no measurement binary
was rebuilt. Archives preserve the scripts/hashes used at measurement time,
while the auditor verifies those snapshots even if reporting scripts evolve.
