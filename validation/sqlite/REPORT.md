# Why not just use SQLite?

Kiban has a performance case for this warm, single-worker memory API. With the
opt-in point index, its paired point-read p50 is 1.55–2.03× faster than SQLite's
mmap row across the four read groups. SQLite wins memory use and indexed-Kiban
disk footprint. Durable update performance gives no consistent Kiban win.
These are shared-host measurements, not exclusive-core or cold-disk results.

This experiment changes the readpath harness only. Kiban's engine `src/` stays at
`06a457db` with `KibanOptions::point_index` both disabled and enabled.

## Workload and timing boundary

The experiment reuses the existing `tools/agentbench` memory model, `random::mix`,
JSON encoding and decoding, public backend API, full-value oracle, and histogram.
Seed is **20261016**, with 64 agents and 200 ASCII text bytes per record. Each
record also stores identity, agent, immutable creation sequence, and revision.
IDs determine their agent and sequence in this model. Kiban and LMDB use the
same agent-prefixed descending-sequence key bytes. SQLite uses integer keys,
which the brief permits, and stores exactly the same JSON bytes in `value BLOB`.
The clustered table can derive `(agent, ts)` from an ID without a secondary
lookup because the other backends derive their composite key the same way.
This does not model arbitrary IDs with an independent agent assignment.

Read scenarios start with 1,000,000 records. Each command performs 100,000
`recall-by-id` and 100,000 `recent-for-agent` calls, returning the latest ten
records in descending creation order. Both Kiban bulk and normal layouts are
included. Bulk produces one L0 SST. Normal performs individual writes and
flushes at the default 4 MiB write-buffer budget, allowing normal compaction. The governor can produce different topologies on another host or run. Each recorded fixture stays fixed across all five trials.
Actual final SST levels and counts are recorded in preparation receipts.
SQLite and LMDB fixtures are identical between those two comparison groups.

Write scenarios start with 10,000 records from a fresh private fixture copy.
The shared five-stage plan performs 1,000 insertions, 1,000 point reads,
1,000 latest-ten reads, 1,000 updates, and 1,000 mixed calls. Mixed calls are
exactly 900 reads and 100 individually committed updates. Read, update, and
mixed IDs range over the resulting 11,000-record population. Overwrites retain
creation order. These short writes fit in Kiban's memtable. They do not measure
steady-state LSM compaction throughput at 1M records.

Each timer covers one backend API call, including key construction where
needed, SQL binding and execution, serialization for writes, owned result
materialization, JSON decoding for reads, and the selected commit durability.
Every result is checked against the full deterministic payload outside its
timer. Each write is read back separately outside the timer. Full population
and latest-ten verification run before and after each command. Write commands
also close and reopen the database and verify the final population again.
SQLite performs `integrity_check` after preparation and final write checkpoint.
Any failure prevents a command receipt from being written and aborts the driver.

`Call ops/s` divides call count by accumulated per-call duration. It is not
continuous application throughput. `Verified ops/s` includes oracle generation,
per-call checks, histogram work, progress logging, and write readback within
the stage. Neither includes final sync, flush, checkpoint or full verification.
The final completion duration is retained separately. Histogram percentiles
are upper bucket bounds with less than 3.125% relative bucket error above 32 ns.
Returned `Memory` destruction is outside the timer. Temporary backend buffers
are destroyed inside their backend call. SQLite decodes directly from a borrowed
column BLOB, LMDB from a borrowed mapped value, and Kiban from its returned owned
byte vector. No extra SQLite copy was added to equalize an API limitation.

## SQLite configuration and layouts

Bundled SQLite **3.53.2**, `rusqlite 0.40.2`, `libsqlite3-sys 0.38.2`.
Runtime source ID is
`2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24`.
Every SQLite process logs its complete compile options, schema, effective pragmas,
and `EXPLAIN QUERY PLAN` output. The lockfile preserves `heed3 0.22.0`,
`lmdb-master3-sys 0.2.5`, `mimalloc 0.1.48`, and `serde_json 1.0.143`.
Adding SQLite updates the C build helper `cc` and its `shlex` dependency, not
Kiban's Rust runtime dependencies. All final engines run in the same binary.

The rowid candidate uses creation sequence as its integer primary key:

```sql
CREATE TABLE memories (
    id INTEGER PRIMARY KEY,
    agent INTEGER NOT NULL,
    value BLOB NOT NULL
);
CREATE INDEX memories_recent ON memories(agent, id DESC);
SELECT value FROM memories WHERE id = ?1;
SELECT value FROM memories WHERE agent = ?1 ORDER BY id DESC LIMIT ?2;
UPDATE memories SET value = ?3 WHERE id = ?1 AND agent = ?2;
```

The clustered candidate stores recent records next to each other:

```sql
CREATE TABLE memories (
    agent INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    value BLOB NOT NULL,
    PRIMARY KEY(agent, ts DESC)
) WITHOUT ROWID;
SELECT value FROM memories WHERE agent = ?1 AND ts = ?2;
SELECT value FROM memories WHERE agent = ?1 ORDER BY ts DESC LIMIT ?2;
UPDATE memories SET value = ?3 WHERE ts = ?1 AND agent = ?2;
```

All statements use a 16-entry prepared-statement cache. A connection belongs to
one worker and opens with `SQLITE_OPEN_NO_MUTEX`. There is one connection in
these single-worker trials. Each SELECT is one implicit read transaction.
Each INSERT or UPDATE is its own implicit write transaction and returns only
after its commit. Updates touch only the BLOB because keys are immutable.
Preload is one transaction, outside timing. Both tables load in primary-key
order, the rowid secondary index is built afterward, and `ANALYZE` runs before
measurement. Query plans confirm indexed point lookup and ordered agent lookup,
without a scan or temporary sort.

Initial setup inserted clustered rows in ID order and produced a 597 MiB
1M-record file. Loading in primary-key order reduces it to 326 MiB. The original
setup receipts remain in [preparation-original](preparation-original/NOTICE.md)
and never enter latency summaries. This avoids choosing SQLite's schema from
an unnecessarily fragmented starting point. See SQLite's explanation of the
[WITHOUT ROWID tradeoffs](https://www.sqlite.org/withoutrowid.html).

The generic `durable` option controls Kiban and LMDB. SQLite durability is
controlled by `sqlite_sync`; its effective pragma is authoritative, including
in the separate sync-probe configurations.

Every connection explicitly sets:

```sql
PRAGMA page_size = 4096;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = OFF;    -- or NORMAL or FULL, as labeled
PRAGMA cache_size = 131072;  -- 512 MiB; default-budget trials use 109626
PRAGMA mmap_size = 0;        -- separate mmap rows use 1073741824
PRAGMA wal_autocheckpoint = 1000;
PRAGMA temp_store = MEMORY;
```

Locking mode stays `normal`, auto-checkpointing remains enabled, and no enclosing
read transaction removes per-call snapshot/locking costs. After the timed write
stages, `finish()` sets synchronous to FULL and checkpoints with TRUNCATE, then
checks integrity. This final persistence work is outside operation latency and
inside completion time. It does not retroactively make a relaxed acknowledgment
durable at the moment it was returned.

## Durability and memory fairness

| Row | Acknowledgment and crash guarantee |
|---|---|
| Kiban durable | Each put is followed by `Kiban::sync()`, which flushes the WAL buffer and calls `fdatasync` before returning. Accepted writes survive a crash under the engine's sync contract and an honest filesystem/device. |
| SQLite WAL FULL | Each commit synchronizes the WAL before returning. This is the SQLite comparison for Kiban durable. |
| SQLite WAL NORMAL | Commits do not sync individually. Recent committed transactions can be lost after an OS crash or power loss. WAL consistency is preserved; an application-process crash alone preserves committed transactions. This row is not per-commit durable. |
| SQLite WAL OFF | Syncs are disabled during the timed stages. No power-loss durability or corruption-resistance promise. This is a relaxed speed row. |
| Kiban fast | `put` returns without sync. WAL buffering can lose acknowledged writes even on process death. Recovery's durability floor is the last completed sync. It has a weaker acknowledgment than SQLite NORMAL. |
| LMDB durable | Normal LMDB flags, one committed write transaction per mutation, with default synchronous commit. |
| LMDB fast | `NO_SYNC | NO_META_SYNC`, one transaction per mutation, with no per-commit durability promise. No WRITE_MAP. |

SQLite's [synchronous guarantees](https://www.sqlite.org/pragma.html#pragma_synchronous)
and [WAL checkpoint behavior](https://www.sqlite.org/wal.html) support these
labels. These are documented contracts and implementation checks, not new
power-cut tests. No relaxed-mode win is counted as a durable win.

Explicit Kiban cache size is 536,870,912 bytes. Its actual default on this host
is 449,030,592 bytes, or 428.229 MiB, from the minimum physical/cgroup memory
budget divided by 64. SQLite gets 131,072 pages for 512 MiB or 109,626 pages
for 449,028,096 bytes, only 2,496 bytes below the Kiban default. Effective
settings are checked at open, not inferred from requested configuration.

This matches raw cache capacity, not total process memory. Kiban metadata and
allocator overhead are outside its byte budget. SQLite page objects and
allocator overhead also consume memory. The OS page cache is shared by all
engines. LMDB's 10 GiB map is a virtual address reservation, not a comparable
10 GiB resident cache. Peak RSS is measured for the entire process, including
open, verification, and reopen. It does not include unmapped OS page cache.

Both requested cache budgets fit these roughly 315 MiB base datasets. This experiment does not establish behavior under small-cache pressure or for disk-resident random reads.

SQLite mmap rows keep the same page-cache setting and additionally allow a
1 GiB read mapping, covering the entire fixture. They are a separately labeled
fast-read configuration with additional mapped memory, not a matched total-RSS
comparison. SQLite documents [mmap's separate I/O path](https://www.sqlite.org/mmap.html).
No global cache drop or claim of cold-disk behavior is made. Full pre-verification
warms each engine before timing.

## Host, scheduling and reproducibility

Intel Core Ultra 9 185H, 22 logical CPUs, WD PC SN740 NVMe, Btrfs on LUKS with
`compress=zstd:1`. Rust 1.98.1, LLVM 22.1.8. The readpath release profile uses
opt-level 2, one codegen unit, LTO, panic abort, and MiMalloc for Rust. SQLite's
C library uses its bundled system allocator. Exact host details are in
[host.json](host.json), and binary/source hashes in [raw metadata](raw/metadata.json).

All benchmark and preparation children are pinned to **CPU 10 only**. No work
from this task is assigned to its hyperthread sibling CPU 11 or to CPUs 3/4,
6/7 or 8/9. Build and test commands use `taskset -c 12-21 nice -n 10`, and finish
before any latency screening. Coordinators stay on CPUs 12–21. Five rounds rotate
engine order for each scenario, with an LMDB control in every round. Every final
five-entry scenario puts each engine in each order position exactly once.
Other tasks and desktop processes remain active on the host, sharing power,
LLC, memory and storage. Per-command host load and CPU 10/11 tick deltas are
retained. CPU affinity reserves no exclusive OS core; sibling activity is
reported with the completed audit below.

Logical disk size sums all regular database files after process exit, including
WAL/SHM if present. Allocated size is `st_blocks * 512`. Btrfs compression and
reflink sharing mean this is not a measurement of unique physical extents.
The write figures are post-finish footprints, not peak WAL space or write
amplification. `/usr/bin/time -v` measures whole-process peak RSS.

The following commands reproduce the experiment from the repository root.
Start with fresh `target/sqlite/data` and output directories. The commands below use a new result folder rather than the committed raw evidence. Never run the
build/check block concurrently with any measurement. `--resume` checks frozen
binary/source identity and receipts, and resumes completed phases without
repeating successful commands.

```sh
taskset -c 12-21 nice -n 10 cargo test --locked --manifest-path tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 nice -n 10 cargo clippy --locked --manifest-path tools/agentbench/readpath/Cargo.toml -- -D warnings
taskset -c 12-21 nice -n 10 cargo fmt --check --manifest-path tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 nice -n 10 cargo build --release --locked --manifest-path tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_bench.py prepare --out target/sqlite/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_bench.py screen --out target/sqlite/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_bench.py select --out target/sqlite/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_bench.py measure --out target/sqlite/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_report.py target/sqlite/reproduction/raw
```

The layout selection rule is fixed before screening: minimize the equal-workload
geometric mean of the five-trial medians of mean call duration, selecting once
for ordinary reads, once for mmap reads, and once for writes. The write score includes remember, update and mixed, in fast and durable
scenarios. The read score includes both calls and both cache budgets across the
bulk/normal comparison groups. Selection never mixes rowid point samples and
clustered scan samples into an impossible single-schema result. Every losing
layout's measurements remain in [SCREEN.md](SCREEN.md) and [MMAP-SCREEN.md](MMAP-SCREEN.md). The select phase runs the separate mmap screening before freezing its choice. All final mmap rows use that independently selected layout.

## Results

The audit passes **325 benchmark commands and 44,525,000 timed calls with zero
errors**: 90 ordinary screening commands, 60 mmap screening commands, 150 primary
comparison commands, and 25 durable sync-tuning commands. The ten CPU-counter
loops and eight sync probes are diagnostic and excluded from those totals.
Every recorded benchmark process reports `Cpus_allowed_list: 10`. Ten readpath
Rust tests pass, including both SQLite schemas, all three synchronous modes,
mmap, duplicate/missing mutation rejection, reopened writes and wrong-payload
rejection. Clippy, formatting, Python compilation and the sync-probe smoke test
pass. [Check logs](checks), [audit.json](audit.json), [TRIALS.csv](TRIALS.csv),
[summary.json](summary.json), and [decisions.tsv](decisions.tsv) retain the evidence.

The rowid schema wins the declared geometric score in all three families:

| Family | Rowid score µs | Clustered score µs | Selected |
|---|---:|---:|---|
| Ordinary reads | 6.251 | 6.616 | rowid |
| mmap reads | 4.761 | 4.946 | rowid |
| Writes | 91.874 | 109.532 | rowid |

These are aggregate scores, not the latency of a public operation. The modest
read-score margins do not establish a universal schema winner. Clustered
SQLite has faster latest-ten calls; rowid has faster points. An application
that weights latest-ten more heavily can reasonably choose clustered storage.
Every candidate, including mmap on both schemas, is retained in
[SCREEN.md](SCREEN.md) and [MMAP-SCREEN.md](MMAP-SCREEN.md), with RSS and disk
in the corresponding `*-RESOURCES.md` files. The final comparison uses one
complete selected schema, not a blend of different schemas' best operations.

Cells are **p50 µs / p99 µs / call kops/s**, each the median of five trials. All ranges, verified throughput and paired ratios are in MEASURE.md, FDATASYNC.md and summary.json.

### 1M reads

| Layout/cache | Operation | Kiban index off | Kiban index on | SQLite page cache | SQLite mmap | LMDB |
|---|---|---:|---:|---:|---:|---:|
| bulk/512 | recall-by-id | 1.728 / 2.560 / 578.7 | 0.928 / 1.248 / 1073.5 | 2.176 / 3.008 / 463.1 | 1.856 / 2.496 / 526.0 | 1.376 / 1.856 / 725.2 |
| bulk/512 | recent-for-agent | 2.816 / 4.736 / 347.1 | 2.752 / 4.352 / 360.5 | 9.216 / 13.312 / 108.8 | 8.704 / 12.800 / 112.5 | 2.240 / 3.584 / 437.1 |
| bulk/default | recall-by-id | 1.728 / 2.560 / 576.3 | 0.960 / 1.248 / 1042.2 | 2.368 / 3.328 / 412.2 | 2.016 / 2.752 / 486.1 | 1.440 / 1.920 / 686.0 |
| bulk/default | recent-for-agent | 3.008 / 5.120 / 322.7 | 2.944 / 4.608 / 331.2 | 9.984 / 15.360 / 93.7 | 9.984 / 15.360 / 96.9 | 2.560 / 4.480 / 373.1 |
| normal/512 | recall-by-id | 1.920 / 3.008 / 498.8 | 1.248 / 1.728 / 791.6 | 2.560 / 4.096 / 391.3 | 2.048 / 3.584 / 473.6 | 1.440 / 2.048 / 690.0 |
| normal/512 | recent-for-agent | 3.520 / 6.016 / 274.6 | 3.584 / 6.144 / 268.0 | 10.496 / 17.408 / 93.8 | 9.984 / 16.384 / 96.2 | 2.432 / 4.096 / 390.6 |
| normal/default | recall-by-id | 2.016 / 3.072 / 488.3 | 1.408 / 1.984 / 694.7 | 2.432 / 4.224 / 399.0 | 2.112 / 4.032 / 444.5 | 1.472 / 2.112 / 676.9 |
| normal/default | recent-for-agent | 3.776 / 6.528 / 255.3 | 3.712 / 6.912 / 241.7 | 11.264 / 19.968 / 81.2 | 10.240 / 17.920 / 91.8 | 2.624 / 4.608 / 370.6 |

### 10k writes

NORMAL is relaxed in both groups. FULL is the durable SQLite comparison. Fast Kiban acknowledgments can be lost on process death.

| Mode | Operation | Kiban index off | Kiban index on | SQLite OFF or FULL | SQLite NORMAL | LMDB |
|---|---|---:|---:|---:|---:|---:|
| fast | remember | 0.736 / 6.400 / 983.6 | 0.720 / 6.016 / 982.4 | 11.008 / 40.960 / 77.4 | 11.008 / 41.984 / 39.0 | 7.424 / 13.056 / 126.4 |
| fast | update | 0.848 / 6.272 / 755.1 | 0.816 / 6.912 / 907.7 | 6.784 / 10.752 / 121.7 | 6.656 / 10.752 / 44.8 | 7.680 / 11.520 / 127.3 |
| fast | mixed-90-10 | 1.184 / 2.368 / 819.3 | 0.832 / 1.408 / 1175.6 | 1.792 / 7.168 / 437.9 | 1.856 / 7.168 / 427.9 | 0.912 / 8.704 / 613.8 |
| durable | remember | 557.056 / 1179.648 / 1.7 | 540.672 / 1245.184 / 1.6 | 655.360 / 1605.632 / 1.5 | 10.496 / 44.032 / 39.3 | 1212.416 / 2686.976 / 0.8 |
| durable | update | 557.056 / 1212.416 / 1.7 | 557.056 / 1245.184 / 1.7 | 557.056 / 1703.936 / 1.7 | 6.656 / 9.728 / 43.6 | 1212.416 / 2555.904 / 0.8 |
| durable | mixed-90-10 | 2.368 / 589.824 / 17.3 | 1.632 / 573.440 / 17.0 | 2.432 / 622.592 / 16.8 | 1.856 / 7.168 / 429.2 | 1.664 / 1277.952 / 8.1 |

### Linux fdatasync follow-up

| Operation | Kiban index off | Kiban index on | SQLite FULL fsync | SQLite FULL fdatasync | LMDB |
|---|---:|---:|---:|---:|---:|
| remember | 573.440 / 2064.384 / 1.6 | 540.672 / 6815.744 / 1.3 | 655.360 / 3080.192 / 1.3 | 638.976 / 2818.048 / 1.3 | 1212.416 / 6946.816 / 0.8 |
| update | 573.440 / 2490.368 / 1.5 | 540.672 / 6029.312 / 1.4 | 557.056 / 1638.400 / 1.7 | 557.056 / 1736.704 / 1.6 | 1212.416 / 3932.160 / 0.7 |
| mixed-90-10 | 2.496 / 753.664 / 15.9 | 1.696 / 589.824 / 17.0 | 2.368 / 622.592 / 16.4 | 2.496 / 737.280 / 14.5 | 1.632 / 1441.792 / 7.5 |

### Paired 1M latency ratios

Each cell is **p50 ratio / p99 ratio** for Kiban divided by its same-round comparator. Below 1 favors Kiban.

| Layout/cache | Operation | Index | / SQLite page cache | / SQLite mmap | / LMDB |
|---|---|---|---:|---:|---:|
| bulk/512 | recall-by-id | 0 | 0.794 / 0.826 | 0.931 / 1.026 | 1.256 / 1.333 |
| bulk/512 | recall-by-id | 1 | 0.426 / 0.406 | 0.500 / 0.500 | 0.674 / 0.649 |
| bulk/512 | recent-for-agent | 0 | 0.307 / 0.349 | 0.324 / 0.385 | 1.250 / 1.250 |
| bulk/512 | recent-for-agent | 1 | 0.306 / 0.327 | 0.316 / 0.340 | 1.200 / 1.214 |
| bulk/default | recall-by-id | 0 | 0.736 / 0.776 | 0.869 / 0.905 | 1.209 / 1.273 |
| bulk/default | recall-by-id | 1 | 0.410 / 0.382 | 0.492 / 0.443 | 0.667 / 0.650 |
| bulk/default | recent-for-agent | 0 | 0.303 / 0.328 | 0.295 / 0.319 | 1.200 / 1.088 |
| bulk/default | recent-for-agent | 1 | 0.302 / 0.318 | 0.303 / 0.310 | 1.150 / 1.059 |
| normal/512 | recall-by-id | 0 | 0.750 / 0.742 | 0.937 / 0.839 | 1.378 / 1.485 |
| normal/512 | recall-by-id | 1 | 0.500 / 0.422 | 0.587 / 0.482 | 0.864 / 0.852 |
| normal/512 | recent-for-agent | 0 | 0.346 / 0.375 | 0.362 / 0.375 | 1.421 / 1.469 |
| normal/512 | recent-for-agent | 1 | 0.346 / 0.393 | 0.369 / 0.386 | 1.439 / 1.406 |
| normal/default | recall-by-id | 0 | 0.814 / 0.750 | 0.955 / 0.897 | 1.432 / 1.600 |
| normal/default | recall-by-id | 1 | 0.547 / 0.484 | 0.647 / 0.463 | 0.977 / 0.939 |
| normal/default | recent-for-agent | 0 | 0.351 / 0.354 | 0.363 / 0.386 | 1.487 / 1.545 |
| normal/default | recent-for-agent | 1 | 0.335 / 0.359 | 0.372 / 0.386 | 1.450 / 1.545 |

### 1M disk and memory

Each cell is **logical disk MiB / peak RSS MiB**, median of five process runs. Full resource ranges and allocated bytes are in MEASURE-RESOURCES.md.

| Layout/cache | Kiban index off | Kiban index on | SQLite page cache | SQLite mmap | LMDB |
|---|---:|---:|---:|---:|---:|
| bulk/512 | 314.564 / 433.609 | 391.176 / 422.598 | 314.559 / 337.504 | 314.559 / 316.891 | 799.305 / 812.055 |
| bulk/default | 314.564 / 433.590 | 391.176 / 422.523 | 314.559 / 338.289 | 314.559 / 316.215 | 799.305 / 812.324 |
| normal/512 | 314.573 / 430.105 | 391.197 / 453.223 | 314.559 / 337.926 | 314.559 / 316.027 | 799.305 / 812.172 |
| normal/default | 314.573 / 428.688 | 391.197 / 448.422 | 314.559 / 337.793 | 314.559 / 316.285 | 799.305 / 812.309 |

### Write completion and sync tuning

Durable updates are largely limited by persistence rather than lookup speed.
In the primary matrix, SQLite FULL and both Kiban rows have the same 557.056 µs
median update p50. In the follow-up, Kiban index-on is 540.672 µs and SQLite
FULL with fdatasync is 557.056 µs. Their trial ranges overlap. SQLite's update
call throughput is slightly higher in the primary matrix and higher in the
follow-up; p50 alone does not capture the fsync stalls in the tails.

The follow-up is deliberately separate. Vendor `sqlite3.c` aliases `fdatasync`
to `fsync` unless `HAVE_FDATASYNC` is enabled. A second bundled build uses
`LIBSQLITE3_FLAGS=-DHAVE_FDATASYNC=1` on Linux. It retains WAL and synchronous
FULL. The five rotated rounds include default SQLite FULL, tuned SQLite FULL,
both original Kiban rows, and LMDB. Binary hashes and exact flags are in
[fdatasync metadata](raw/fdatasync/metadata.json). The primary matrix is preserved.

Tuning does not establish a broad durable-write winner. Its SQL update p50 is
unchanged at 557.056 µs, and call throughput is 1,629/s versus default SQLite's
1,664/s in that group. Tail ranges are very wide: tuned SQL update p99 spans
1.573–11.534 ms, and Kiban index-on spans 1.311–11.796 ms. Ratio medians pair
rounds before aggregation; dividing displayed medians can give a very different
answer, particularly when stalls occur in different rounds.

| Whole-process diagnostic | fsync calls | fdatasync calls | Failed calls |
|---|---:|---:|---:|
| kiban-fast | 9 | 2 | 0 |
| lmdb-fast | 0 | 1 | 0 |
| kiban-durable | 9 | 2102 | 0 |
| lmdb-durable | 0 | 2101 | 0 |
| sqlite-off | 3 | 0 | 0 |
| sqlite-normal | 13 | 0 | 0 |
| sqlite-full | 2113 | 0 | 0 |
| sqlite-full-fdatasync | 0 | 2113 | 0 |

These counts include open, all 2,100 mutations, final flush/checkpoint and
reopen. Kiban durable adds exactly 2,100 fdatasync calls over fast. SQLite FULL
adds exactly 2,100 sync calls over NORMAL. NORMAL's remaining syncs are consistent
with checkpoint/finalization work. This validates the requested acknowledgment
policies without treating final flush as per-write durability. The probe counts
libc calls, not kernel tracepoints, and its own timings are never reported.

SQLite NORMAL and OFF can have similar p50/p99 yet different mean throughput.
For example, fast-scenario updates show 6.656/10.752 µs under NORMAL and
6.784/10.752 µs under OFF, but 44.8k versus 121.7k call ops/s. Automatic checkpoint
syncs and stalls below the top one-percent boundary affect the mean. Raw maximum,
p999 where defined, call duration, and wall duration remain in the JSONL files.
The probe shows additional NORMAL syncs; it does not assign every stall to a
specific checkpoint without a syscall time trace.

Final completion cost is separate from call throughput. It includes Kiban
sync/flush, LMDB force-sync, or SQLite FULL checkpoint/integrity check, plus
full-model verification. It excludes the later reopen check.

| Group | Engine | Completion seconds median [min,max] |
|---|---|---:|
| measure/durable | kiban-index0 | 0.0250 [0.0228, 0.0303] |
| measure/durable | kiban-index1 | 0.0246 [0.0221, 0.0260] |
| measure/durable | lmdb | 0.0136 [0.0127, 0.0140] |
| measure/durable | sqlite-full | 0.0370 [0.0307, 0.0384] |
| measure/durable | sqlite-normal | 0.0376 [0.0310, 0.0395] |
| measure/fast | kiban-index0 | 0.0256 [0.0233, 0.0286] |
| measure/fast | kiban-index1 | 0.0239 [0.0216, 0.0255] |
| measure/fast | lmdb | 0.0279 [0.0270, 0.1103] |
| measure/fast | sqlite-normal | 0.0369 [0.0355, 0.0401] |
| measure/fast | sqlite-off | 0.0437 [0.0419, 0.0451] |
| fdatasync/durable | kiban-index0 | 0.0292 [0.0249, 0.0413] |
| fdatasync/durable | kiban-index1 | 0.0275 [0.0226, 0.0343] |
| fdatasync/durable | lmdb | 0.0130 [0.0117, 0.0145] |
| fdatasync/durable | sqlite-full | 0.0337 [0.0326, 0.0345] |
| fdatasync/durable | sqlite-full-fdatasync | 0.0355 [0.0331, 0.0395] |

### CPU evidence and its limits

One unpaired diagnostic run per row, bulk/512, one million full API calls per
loop. FIFO control enables counters only inside the loop, excluding fixture
open, trace generation and both full verifications. These profile loops include
a small result digest and destruction of returned objects, unlike the latency
timer. Events are user-mode `cpu_core` counters, fully scheduled without
multiplex scaling. They are diagnostic evidence, not additional latency trials.

| Backend | Call | User cycles/op | Instructions/op | Branch misses/op | LLC misses/op | dTLB misses/op |
|---|---|---:|---:|---:|---:|---:|
| kiban-index0 | point | 7366.20 | 8477.16 | 3.68 | 10.64 | 0.27 |
| kiban-index0 | latest ten | 12396.48 | 49119.57 | 21.94 | 0.06 | 0.29 |
| kiban-index1 | point | 4198.26 | 5784.87 | 1.71 | 9.31 | 0.02 |
| kiban-index1 | latest ten | 12236.27 | 48449.13 | 22.74 | 0.04 | 0.32 |
| lmdb | point | 5599.30 | 8037.38 | 10.99 | 12.98 | 1.99 |
| lmdb | latest ten | 10255.01 | 42342.61 | 24.04 | 0.15 | 0.12 |
| sqlite-mmap | point | 5798.67 | 9869.98 | 16.35 | 7.50 | 1.11 |
| sqlite-mmap | latest ten | 36299.86 | 106122.88 | 69.40 | 0.25 | 0.52 |
| sqlite | point | 7089.48 | 9968.00 | 16.45 | 10.86 | 2.90 |
| sqlite | latest ten | 38103.06 | 107362.74 | 76.29 | 0.17 | 0.61 |

The point index reduces Kiban's measured user instructions from 8,477 to 5,785
per point call. SQLite mmap uses 9,870. Latest-ten SQLite mmap uses 106,123
instructions versus indexed Kiban's 48,449 and LMDB's 42,343. The extra SQLite
user work and branch misses are consistent with its generic statement/cursor
path costing more than these specialized APIs. This does not isolate a causal
percentage for the SQL VM, locks, JSON, or allocation. User counters exclude
kernel work. No profiler was active during the latency matrix.

### Where SQLite wins and what the verdict permits

SQLite wins the measured footprint comparison. Its mmap row uses roughly
316 MiB peak RSS versus 423–453 MiB for indexed Kiban, while ordinary SQLite
uses roughly 338 MiB. Its 314.559 MiB database is about 19.6% smaller than
indexed Kiban's 391.2 MiB. Unindexed Kiban is about the same logical size as
SQLite, so SQLite's disk advantage is specifically against the point-index
configuration. The existing LMDB control uses about 799 MiB on disk and
812 MiB RSS; that reflects its existing ID-order loader and layout, not a
claim that LMDB cannot be packed more tightly. Write-scenario SQL RSS is also
lower, about 10 MiB versus Kiban's 16–20 MiB.

SQLite also shows the higher durable-update call throughput in these runs.
The wide sync-tail ranges prevent a general durable superiority claim for
either engine. Its stronger relaxed-mode guarantee matters: NORMAL commits
survive an application-process crash, while Kiban fast can acknowledge data
still in a userspace WAL buffer. Treating that fast-mode gap as an equal-crash-
guarantee win would be wrong.

Kiban's supported case is narrow and useful: low warm-read latency for this
keyed memory API, plus very cheap buffered mutations when the application
accepts their weaker acknowledgment. The opt-in point index gives clear point
wins against SQLite mmap here, but costs about 24% more SST space. It beats
LMDB clearly in bulk, improves normal/512, and is close to parity in
normal/default. LMDB remains faster at latest-ten across all four groups.
There is no evidence here that an LSM is inherently better for agent memory.

If footprint, SQL facilities, or durable writes drive the application, these
results provide no reason to reject SQLite. If this exact warm keyed API is the
latency bottleneck, Kiban's specialized read path is a measured reason to choose
it. Application-visible benefit still depends on how much request time belongs
to storage. This experiment excludes embeddings, vector search, multi-process
contention, batched commits, cold starts, small-cache pressure, deletes, and
steady-state compaction under writes. The optional concurrent-reader/writer
case was not run. None of those gaps should be filled with a winner claim.

### Host noise and final verification

No benchmark task uses CPU 11, but it was **not exclusively idle**: background
busy time across the 150 primary process intervals has median 3.78%,
range 0.00–21.05%. These include verification, not just timed call
intervals, and short write commands make percentages coarse. Primary one-minute
load ranges from 3.30 to 6.33.
The requested CPU-10 affinity is satisfied; strict sibling-idle isolation is
not. Other tasks' affinities were not changed. Interpret the data as a rotated,
controlled shared-host comparison, and rerun on an isolated host for a strict
exclusive-core claim. The visible durable-tail drift reinforces this limit.

Engine source hashes match the frozen `06a457db` sources. No engine `src/`
file, core dependency, or engine option default changes. No push or PR is made.
The original setup archive, every successful trial, losing SQLite layouts,
raw controls, command times, effective pragmas, query plans, process status,
peak RSS and disk receipts remain local and committed with this report.

To reproduce the optional diagnostics, provide a local `perf` executable at
`target/sqlite/bin/perf`. The `cpu_core` event names match this Intel hybrid
host. Run all builds and checks separately from timed campaigns.

```sh
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_profile.py \
  --raw target/sqlite/reproduction/raw --out target/sqlite/reproduction/profile
taskset -c 12-21 nice -n 10 cc -O2 -Wall -Wextra -Werror -shared -fPIC \
  tools/agentbench/readpath/sqlite_sync_probe.c -ldl -o target/sqlite/bin/sync-probe.so
LIBSQLITE3_FLAGS=-DHAVE_FDATASYNC=1 \
  CARGO_TARGET_DIR="$PWD/target/sqlite/fdatasync-target" \
  taskset -c 12-21 nice -n 10 cargo build --release --locked \
  --manifest-path tools/agentbench/readpath/Cargo.toml
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_fdatasync.py \
  --raw target/sqlite/reproduction/raw
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_sync_probe.py \
  --raw target/sqlite/reproduction/raw --out target/sqlite/reproduction/sync-probe
taskset -c 12-21 python3 tools/agentbench/readpath/sqlite_report.py \
  target/sqlite/reproduction/raw
```

These commands use fresh reproduction folders. The helpers default to the recorded `validation/sqlite` locations when their path flags are omitted.
