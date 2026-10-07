# Correctness repair report — 2026-10-07

The three scale-validation failures are repaired. Further validation exposed
and repaired two related correctness defects: acknowledgement after a failed
direct flush, and snapshot SST lifetime across metadata-only level moves.
No scheduler, workload, durability option, data format, or benchmark was changed.
The existing WAL/SST/MANIFEST architecture and one-immutable limit remain intact.

## Root causes, changes, and restored invariants

### WAL durability

`Kiban::freeze` kept the old WAL number MANIFEST-live but replaced and dropped
its `Wal` writer. Dropping its buffered writer did not fsync the old file.
`Kiban::sync` then synced only the new active WAL, so successful sync after freeze
could acknowledge records that vanished on power loss.

`src/db.rs`: `Immutable` now owns its WAL writer; `freeze` transfers ownership;
`sync` syncs the immutable generation and active generation, poisoning the engine
on either failure using existing failure policy. `commit_flush` retains the
immutable/WAL on build or commit failure and releases it only after the output
SST has been durably published and MANIFEST installation succeeds.

The lifetime is now append -> mutable memtable -> freeze retaining writer and
MANIFEST reference -> immutable memtable -> durable SST -> durable MANIFEST
installation -> release immutable/WAL -> best-effort unlink. Merely freezing,
creating an SST, or syncing an SST is not a WAL-retirement point. A failed or
ambiguous MANIFEST install cannot acknowledge further mutations. Recovery replays
all MANIFEST-live generations and durably consolidates them before removing old
WALs; stale unreferenced WALs are swept. Those recovery rules were preserved.
There is at most one pending immutable, hence at most two live writers; repeated
generations and recovery with multiple WALs are tested.

Restored invariant: a successful sync covers every previously accepted mutation
until durable committed SST state supersedes its WAL.

### Snapshot visibility

Direct `Snapshot` pinned only its sequence and SST Version. `get_at`/`scan_at`
consulted the engine's current memtables. Once captured data flushed, it existed
neither in those memtables nor in the pinned, pre-flush Version. Sequence metadata
was intact; the snapshot had lost ownership of a necessary source.

`src/db.rs`: `Snapshot`, `Kiban::snapshot`, `get_at`, and `scan_at` now capture and
read a copy of the active memtable and an Arc of the frozen memtable, alongside
the pinned Version, matching the existing shared-snapshot design. Every source
is still filtered by the captured sequence. Future tombstones cannot hide older
visible values; captured tombstones continue to hide older values. Compaction's
existing sequence/drop rules are unchanged.

The extended campaign then found a second snapshot cause: a trivial level move
constructs another `TableEntry` for the **same SST number**. Reclamation checked
only the later entry's Arc count, missing a snapshot that pinned an older entry
for that physical file. The old SST was unlinked after a subsequent rewrite.
A real-filesystem regression reproduced this without any crash or injected fault.

`src/db.rs`: `TableEntry` now has a shared file-lifetime token. New physical SSTs
create a token; `CompactionPlan::build` clones it for metadata-only moves.
`reclaim_obsolete` requires both the metadata entry and the physical-file token
to be unshared before invalidating descriptors/unlinking. Multiple moves and
independently released snapshots preserve the same file and eventually permit
reclamation. This also protects captured read views and shared snapshots, which
pin the same Version entries.

Restored invariant: physical movement and reclamation cannot change the logical
value or readable files of a pinned snapshot.

### Point reads versus scans

`VerifiedBlock::get` searched for the last restart key `<= target`. Equal-key
versions span restart intervals in descending sequence order, so that start
position skipped newer versions. The 40-update reproduction returned value 8
while the merge scan returned 40. It could also skip a visible tombstone.

`src/sstable/block.rs`: lookup starts before the **first** restart `>= target`,
then returns the first sequence-visible match in internal-key order. Starting
one interval earlier also covers a key whose versions begin before that restart.
The table builder already keeps one user key's versions in a single block; no
format or block-cut rule changed.

`src/db.rs`: `get_from_version_at` compares visible candidates from every
applicable SST by sequence rather than returning the first file/level match.
Direct `get` now uses this same resolver as `get_at`, shared gets and shared
snapshots. Live direct reads remain unbounded, matching live iteration; snapshots
pass their explicit boundary. A winning tombstone hides all older candidates.
Mutable and frozen memtables retain their generation-order precedence: writes
in a later WAL generation have strictly later sequences, and invisible versions
fall through for snapshot reads. Tests cover these source transitions separately.
Existing read-amplification counters remain consistent on the unified direct path.

Restored invariant: point lookup and merge iteration choose the newest visible
sequence, independent of restart, file-number, or level ordering. File-order
robustness is additionally tested with a hand-built valid MANIFEST whose sequence
ordering differs from physical ordering; that fixture does not claim the normal
scheduler had produced that topology in earlier measurements.

### Related direct-flush acknowledgement defect

Tracing flush failures found `flush_without_compaction` installing the MANIFEST
**before** fallible SST/replacement-WAL handle preparation. An open failure left
RAM on the old, now-retired WAL. Continuing with put + successful sync could lose
that newly acknowledged write on crash. A deterministic read-open fault at index
16 reproduced the loss before the fix.

`src/db.rs`: direct flush prepares both handles before MANIFEST installation;
the post-commit state transition has no fallible handle preparation. It reserves
unique output identities before preparation, preventing retry reuse of a file
number whose inode/blocks may already be cached. Failed attempts leave harmless
orphans for existing recovery sweeping. No additional fsync policy is introduced.

`src/sys.rs`: an opt-in **unit-test-only** extension includes read opens in the
existing indexed fault stream. Ordinary fault sweeps retain their original
numbering, and production read paths contain no added fault checks.

Restored invariant: a failed flush cannot leave an unpoisoned engine accepting
durable writes into a WAL that the committed MANIFEST no longer references.

## Deterministic regressions

`src/scale_validation_tests.rs`:

- `sync_after_freeze_must_durably_cover_the_older_wal`: former ignored failing
  probe, now a passing ordinary test with its preservation assertion unchanged.
- `frozen_wal_barrier_survives_every_flush_fault_and_power_loss`: all single and
  pairwise checked faults across freeze, both syncs, SST create/fsync/publication,
  MANIFEST install and WAL cleanup; recovery checks durable data and deletes.
- `frozen_wal_sync_failure_poisoning_covers_both_generations`: failed write/sync
  of either generation rejects further acknowledgement.
- `repeated_frozen_wal_generations_remain_durable_before_and_after_commit`: same
  simulated namespace, repeated generations, crashes before BUILD, after durable
  SST creation but before installation, and after commit.
- `durable_acknowledgements_after_a_flush_error_remain_recoverable`: enumerate
  write/durability/read-open faults, continue where safe, sync, power-loss/reopen.
- `failed_flush_retry_never_reuses_a_cached_sst_identity`: retries with changed
  values must agree with scans and survive recovery, including cached preparation.

`src/correctness_tests.rs`: snapshots before first flush, multiple sequence
boundaries, deletes and future tombstones, active/frozen/table sources, overlapping
L0 generations, compactions, reopen, 40 versions crossing restart intervals,
sequence versus file/level ordering, and physical SST lifetime through repeated
metadata moves followed by rewrite. The lifetime test disables the block cache,
pins two distinct metadata views, and verifies eventual reclamation.

`src/sstable/block.rs`: 80 versions, tombstones, six prefix/restart alignments,
and every sequence boundary, compared directly against block iteration.

`tests/recovery_validation.rs`: existing snapshot/point failure-detector campaigns
now require success rather than expecting defects. Durable-state, snapshot and
point/scan assertions in the unchanged torture oracle remain strict.

Before-fix logs preserve deterministic failures for the original frozen-WAL
probe, both snapshot regressions, all three point regressions, restart lookup,
post-error direct-flush acknowledgement, and moved-file lifetime. The snapshot
stage's full run passed 253 library tests but its campaign still failed on the
not-yet-fixed point/scan defect; it was not ignored or suppressed. All subsequent
final tests use the completed fixes.

## Commands and results

The focused runs included:

```sh
cargo test --lib sync_after_freeze_must_durably_cover_the_older_wal -- --nocapture
cargo test --lib frozen_wal -- --nocapture
cargo test --lib correctness_tests::snapshot -- --nocapture
cargo test --lib correctness_tests -- --nocapture
cargo test --lib restart_lookup_matches_iteration
cargo test --test recovery_validation -- --nocapture
cargo test --lib durable_acknowledgements_after_a_flush_error -- --nocapture
cargo test --lib failed_flush_retry_never_reuses
cargo test --lib snapshots_pin_the_same_sst -- --nocapture
```

Focused commands were run after each fix, followed by
`cargo test --lib --tests --examples`. Final commands:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features loom -- -D warnings
cargo test --features loom --lib -- --test-threads=1 loom_
cargo test
cargo test --examples
cargo test --release --lib -- --ignored --nocapture
python3 validation/run.py torture --seed 20261006 --threads 4 \
  --data-root target/correctness-data/torture-final \
  --output-root validation/correctness-results/torture-final --execute
```

Use fresh data/output directories to reproduce the last command. Exact generated
CLI arguments and resolved engine options are preserved in `correctness-results`.
No benchmark workload was run or altered in this correctness pass.

Final regular checks: **260 library tests passed**, **3 integration tests passed**,
**3 example tests passed**, **3 loom models passed**, both Clippy configurations and
formatting passed. The two pre-existing default-ignored gate tests were both
executed in release mode and passed separately. No newly ignored tests remain.
Default `cargo test` includes the existing recovery, single/pairwise syscall,
power-loss, snapshot, iterator, compaction and background-worker tests. The three
standalone harness-free performance benches are not correctness tests and were
not executed; all-target Clippy compiled them unchanged.

Final campaign results are recorded in `correctness-results/summary.json` and
`torture-final/exit-codes.csv`; every final mode exited zero. The simulated
campaign completed **1,000 recovery cycles**, **415,929 mutations**, **332 fired
injected faults**, with snapshots enabled. There were **728 exact-state recoveries** and **272 band-verified recoveries**.
Exact verification is required when there is no ambiguous flush; otherwise the oracle enforces the known per-key
acknowledged/attempt band. This distinction is unchanged.

The unchanged shared-engine process campaign also completed **1,000 SIGKILL /
reopen cycles**, verifying **530,100 acknowledged mutations** and a total **62,272
operation tail budget**. Both snapshots and the whole-domain tombstone/point/scan
oracle remained enabled. Both original real-file probes passed. This proves the
recorded campaign's invariants, not a deterministic execution schedule.

An exploratory campaign after the first four repairs reached 173 verified
simulated cycles before exposing the moved-file lifetime defect. Its logs are
retained. Its process campaign stopped with ENOENT after 898 verified cycles
while release rebuilding replaced its executable; this is an incomplete run,
excluded from final correctness conclusions. A standalone Linux diagnostic
reproduced `current_exe()` returning the replaced path with ` (deleted)` and
child spawn failing with errno 2 (`checks/executable-replacement.log`); no engine
conclusion is drawn from that interrupted run. The final campaign starts only
from the rebuilt binary; that executable is kept unchanged for the whole run.

## Remaining limits and architectural debt

- No remaining failure is claimed repaired merely because of a passing test.
  These tests/campaigns establish bounded evidence, not an exhaustive proof of
  arbitrary OS/device behavior or every thread schedule.
- The volatile-device simulator is thread-local, does not model the background
  worker or full directory-entry persistence, and faults do not cover every
  possible read error. Freeze/flush power-loss tests exercise the actual private
  pipeline synchronously. Process torture covers concurrent SharedKiban and
  SIGKILL with surviving OS cache; it is not a machine-power-loss experiment.
- MANIFEST has structural validation but no checksum. Protection against
  media corruption of MANIFEST bytes needs a separately designed format change;
  WAL framing and SST checksums were preserved. This is a remaining integrity
  risk, not a newly failing durability regression demonstrated here.
- Direct snapshots still use explicit release bookkeeping and copy the active
  memtable, matching the established shared design. More precise retention and
  ownership APIs can be considered later; captured sources and shared physical
  file lifetime must be preserved. Retained memtable history is conservative.
- Independent simultaneous engine opens of one directory are not validated
  here. Runs use one owner or SharedKiban clones. No claim is made about unrelated
  owners across processes or snapshots carried across independent reopenings.
- Point resolution may probe more SSTs now that it does not trust physical order.
  No read-path or compaction optimization was attempted. Existing scale results
  predate these repairs and must not be presented as post-fix performance data.
