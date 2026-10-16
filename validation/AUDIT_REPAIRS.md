# October 2026 audit repairs

This pass implements the concrete findings from the audit of `b01f274`.
Changes are currently uncommitted on `main`. This document separates correctness
repairs, operational improvements and measured performance evidence.

## Correctness and recovery

- Shared batches publish all mutations under one mirrored-memtable guard. Reads
  bypassing the engine gate cannot observe an intermediate batch value. A paused
  batch regression covers repeated-key updates, deletion and another key.
- `LOCK` is acquired through Rust's nonblocking OS file-lock API before recovery
  changes any artifacts. Independent owners are rejected; shared clones use one
  owner. Snapshot leases preserve directory ownership while captured files are
  needed. Tests cover two opens, another process, SIGKILL/reopen and snapshot clones.
- New MANIFESTs use a versioned CRC-32 envelope. Legacy MANIFESTs remain readable.
  All single-bit changes to a valid new envelope are rejected. WAL/SST identities
  must be globally unique, including nonadjacent tables from different levels.
- Recovered leveled SST ranges are checked in `(level, first_key)` order so an
  intervening table at another level cannot hide an overlap.
- WAL batches validate counts, operation kinds, lengths, delete semantics,
  sequence intervals and exact consumption before applying anything. Malformed
  suffixes do not apply a prefix. Per-key replay sequences must increase.
- Mutation sequences use checked arithmetic and return `SequenceExhausted`.
  Compaction uses an explicit empty predecessor rather than reserving
  `u64::MAX` as a sentinel; the last valid sequence survives flush/compaction/reopen.
- SST index counts are bounded by encoded bytes before allocation. Entry ranges
  use checked bounds and must exactly cover the data region. Truncated data-entry
  headers and invalid shared-key prefixes return corruption instead of panicking.
- Framed recovery grows its buffer only as bytes arrive; metadata/read allocations
  are bounded or fallible. New mutation limits reject oversized input before WAL
  I/O without poisoning the engine.

## Memory, snapshots and maintenance

Active engine memtables keep the latest value/tombstone per key. Snapshot capture
copies that state; direct clones share the captured copy and an identity-aware
lease. The last clone automatically releases bookkeeping. Explicit release is
idempotent and cannot release another snapshot at the same sequence.

Flushing uses cumulative mutation volume, not just resident bytes. This is
necessary after pruning history: repeated overwrites of one key must still rotate
the WAL. The regression verifies multiple automatic flushes for a tiny working set.
Stats distinguish resident memtable bytes from write volume.

Snapshots pin the physical files in their captured topology. They never consult
new compaction output, so that output can discard hidden intermediate versions.
Long-lived snapshots no longer require every subsequent update to be retained in
new SSTs. A regression preserves an old snapshot through 100 flushes while bounding
versions in the live topology. Pinned obsolete files still remain until release.

Governor batch averages use running totals instead of an unbounded history vector.
Memtable iterators stream without building a complete reference vector. SST filter
construction retains distinct hashes instead of copied keys. Shared snapshots expose
streaming `iter()` and half-open `range()`; `scan()` explicitly collects results.

Compaction opens an independent input descriptor only while refilling a buffer,
so simultaneously held input descriptors do not grow with fan-in. Foreground cache
limits still describe that cache, not every process descriptor. Failed obsolete
SST deletions remain queued for retry; stats expose pending bytes and failed attempts.

## API and operations

Mutation methods consistently return `DbError`, with typed invalid-argument,
locked-directory, sequence-exhaustion, maintenance and poison states. Converting
to `io::Error` retains the original error as its source. `checkpoint()` and
`close()` report durability/maintenance failures explicitly; drop remains insufficient
as a durability acknowledgement. Other clones/snapshots can keep ownership alive.

Logical backup exports a captured view into an exclusively created destination,
flushing incrementally and syncing before success. A failed destination can be
partial and must not be treated as a completed backup. Existing destinations are
never overwritten. The CLI supports `check`, `inspect`, `backup` and `restore`.
Integrity checking reads all SST entries, including hidden versions, verifies
ordering and compares point lookups with scans. Opening for inspection performs
ordinary recovery; it is not a read-only forensic operation.

Rust 1.89 is the declared/tested minimum. CI now covers diagnostics, strict rustdoc,
Python supervisor tests, bounded process recovery, release stress and minimum-version
checking. Ordinary read-progress testing is smaller and records actual progress;
the original eight-million-read workload remains an ignored release stress test.
Unexpected reader and writer errors fail the test, and failure signals stop before
reporting it. CI also bounds the test process.

The current architecture is documented in `ARCHITECTURE.md`; the README links
current reports and includes operations/limits. `examples/concurrent.rs` demonstrates
concurrent mutation and a stable snapshot. Local private design notes are unchanged.

## Compatibility

The MANIFEST envelope is a forward format change: old Kiban builds cannot read it.
Legacy unversioned files can be read and migrate on the next MANIFEST installation.
Mutation return types changed to `DbError`; callers matching `io::Error` directly
must adapt or convert. New mutation/WAL limits are 1 MiB keys, 16 MiB values,
100,000 batch operations and 64 MiB framed records. Oversized old WAL records or
SST blocks/metadata beyond the new parser bounds are rejected rather than allocated
without practical limits. This is an unreleased pre-1.0 change, not a published tag.

## Validation evidence

Evidence is preserved locally under `target/audit-repairs-20261012/`, including
source/binary hashes, source archives, commands, exit codes, raw CSVs and logs.
`checks/final/results.json` records the final formatting/compiler/test gates;
`final-build/receipt.json` identifies the final frozen engine and sources.

The timed-build ordinary and diagnostics suites each passed 283 library tests and eight
integration tests. Eight example tests, three Loom models and all three ignored
release stress tests passed. All three Clippy variants, strict rustdoc, Rust 1.89
all-target checking, package verification and six Python supervisor tests passed.
Targeted parser/mutation regressions and a bounded format-aware mutation corpus
are part of the ordinary suite; this is not an exhaustive external fuzz campaign.

The first repaired build passed durability probes, 1,000 simulated fault/power-loss
cycles and 1,000 real SIGKILL/reopen cycles with snapshots. The final build also passed all probes, 1,000 simulated cycles (417,709 completed
mutations, 319 injected-fault cycles) and 1,000 SIGKILL/reopen cycles (532,576
acknowledged mutations) after precise snapshot cleanup and maximum-sequence
hardening. Its records are under `checks/final-recovery/`. A run is only a pass when its exit code and status
both indicate completion.

## Concurrency transition

The separately named audit campaign reuses the exact parameters of the historical
ten-minute 1-to-16-thread transition: one million preloaded keys, 128-byte values,
50/50 reads/writes, identical seed/cache/buffer/compaction settings, five minutes
per phase, final sync/flush and full reopened-state verification.

The initial repaired build completed 60,857,107 operations and verified all one
million keys. Complete minute windows in its high-thread phase changed from
134,134 to 136,726 ops/s (+1.9%). Median interval read p99 stayed at 135,168 ns;
write p99 changed from 1,507,328 to 1,474,560 ns. Absolute PUT tails remain material.
The low-thread phase changed from 71,349 to 68,022 ops/s (-4.7%). Full intervals,
latencies, resource counters and source/binary receipts are in `transition/`.

The historical 35.3% decline did not reproduce in this run. This is evidence of
stability under the recorded conditions, not proof of its root cause or a universal
speedup. The host was discharging with the powersave governor throughout; absolute
rates are substantially below the earlier campaign. Do not pool those cohorts or
claim that an isolated engine effect has been measured. No power setting or global
cache state was changed. The final frozen build completed its separately recorded transition under
`final-transition/`: 57,827,819 operations and full verification of one million
keys. Its high-thread first/last windows were 124,756 and 123,116 ops/s (-1.3%).
Median interval read p99 changed from 114,688 to 118,784 ns; write p99 changed from
2,064,384 to 1,949,696 ns. Low-thread throughput changed by -1.8%. Final compaction
debt was zero. Both builds finished their measured high-thread phases without the
historical 35% decline; the data do not establish why that historical run declined.

A closing API regression also rejects direct snapshots passed to a different
engine owner. That read-only argument guard was added after the timing/recovery
builds above; their exact source archives remain unchanged. The final tree adds
that regression (284 ordinary and diagnostics library tests) and is separately
compiler/test checked. An additional 1,000-cycle simulated recovery campaign
passed with this owner guard; its current source and binary hashes are in
`checks/closing-guard/receipt.json`. No comparison claims are made for an
unrecorded binary.

## Further profiling and design work

Writer grouping/pipelining, cache membership sharding, indexed leveled lookup,
heap merge selection and fully streaming SST construction remain architectural
options to evaluate with profiles. They were suggestions rather than demonstrated
correctness defects. This pass fixes the confirmed defects and removes supported resource-growth
causes. These architectural options need profiles and dedicated validation. Transactions, SQL, compression and additional iterator operations
remain outside this engine's current feature scope.
