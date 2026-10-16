# Kiban architecture

Kiban is an embedded, ordered byte-key/byte-value LSM storage engine for Linux.
Applications link the Rust library and own a database directory. `Kiban` is a
direct synchronous handle; `SharedKiban` offers concurrent clones and a background
maintenance worker. There is no server, SQL layer or transaction coordinator.

## Ownership and durable state

Opening acquires an exclusive, nonblocking OS lock on `LOCK` before recovery
creates, replays, truncates or removes anything. A second engine owner is rejected,
including in another process. Shared clones use one owner. The OS releases the
lock after a process dies; never unlink or replace `LOCK` while the directory is
in use. A snapshot lease retains ownership while it needs captured files.

The MANIFEST defines the live WAL generations, SSTs, levels, sequence boundary and
next file identity. New writes use a versioned CRC-32 envelope; legacy unversioned
MANIFESTs are readable and become enveloped at the next installation. Every live
WAL/SST has a unique identity. Files at levels above L0 have disjoint key ranges.
Recovery validates these invariants before serving data.

A mutation appends a checksummed WAL record and updates the memtable. A write
batch uses one WAL record and publishes every operation under one shared-memtable
guard, so concurrent point reads cannot see a prefix. Buffered mutation success
does not acknowledge crash durability. Successful `sync()` covers all live WAL
generations; errors that leave WAL/commit state unsafe poison future mutations.
Rotation syncs the closed WAL generation before opening the next generation, so
WAL generations become durable in order and recovery is a prefix across generations.

Flush and compaction write new immutable SSTs, sync them and install a new
MANIFEST through temporary-file write, file sync, rename and directory sync.
Only then may old files be retired. An ambiguous commit is surfaced rather than
guessed. Batch replay validates the entire payload before applying any operation.

Every new WAL begins with an `OP_WAL_HEADER` frame containing
`[op u8][format_version u8 = 1][nonce u64 LE]`. The header is written before the
existing durable-create file sync and directory sync; it adds no fsync. The nonce
stays internal: production derives 64 bits from a freshly keyed `RandomState`
hash of time and pid, while simulated devices use a deterministic counter.

WAL sync appends an `OP_PAD` frame to end on a 4096-byte boundary before flushing
and syncing. A gap smaller than a frame extends to the following boundary. After
a successful sync, the next mutation first appends an `OP_SYNC_MARK` frame at that
aligned offset, carrying `[op u8][synced_end u64 LE][nonce u64 LE]`. Padding can
add write syscalls, but adds no fsyncs. Open syncs the recovered WAL once. When
it contains more than a header, open schedules a lazy marker, padding first if
needed.
Headers, pads and markers have no memtable effect. Headerless logs do not emit
new markers.

On a framing or decode failure at offset X, recovery scans later 4096-aligned
offsets for a valid sync marker carrying this file's header nonce and claiming
`synced_end > X`. A writer marker exists only after the sync covering
`[0, synced_end)` succeeded, so a surviving matching marker proves X was synced.
User values can mimic such a marker only by guessing the private 64-bit nonce
(accidental match approximately 2^-64 per candidate). Deliberate forgery is out
of scope under the buggy-not-malicious threat model. Recovery reports corruption
and leaves the WAL untouched. Ordinary aligned records and markers with another
nonce do not count. If the header is missing (an old-format log) or unreadable,
no marker counts and damage truncates. If truncation removes the header frame,
open writes a fresh header with a new nonce and syncs it before accepting writes.
Without a surviving matching marker,
recovery truncates and syncs at the last valid frame boundary, including for
zero-filled or partially written tails. Damage to synced bytes with no later
surviving marker also truncates rather than errors. The new op codes change the
WAL format: old versions cannot read new logs, while new code reads old logs.

## Reads and snapshots

Shared point reads capture a published read view, probe its active and frozen
memtables, then resolve the newest visible SST sequence. They bypass the engine
writer gate but can wait briefly on mirrored memtable mutation or read-view
publication. A visible tombstone hides older values. Physical level/file order
does not establish which value is newest.

Snapshots copy the active memtable and pin immutable sources and an SST topology.
Future writes cannot change those sources. Active engine memtables therefore keep
only the latest value/tombstone per key; copied snapshots preserve earlier views.
Snapshot identities are independent even when their sequence boundaries match.
Direct snapshot reads reject a different engine owner. Direct snapshot clones
share their copied memtable and lease; automatic
release happens when the last clone drops. Explicit release is idempotent and cannot release another snapshot.
Released snapshots still pin their files and directory owner until dropped.
Compaction can discard versions hidden in the new live topology because captured
snapshots read their pinned older topology, not the new output. Long-lived
snapshots therefore do not force every intermediate update into new SSTs.

Direct iterators and shared snapshot `iter()`/`range()` stream results; `scan()`
explicitly collects them. Merge sources retain one head each. Memtable iteration
does not allocate a reference vector covering the entire memtable.

Each published version indexes every level above L0 by first key and verifies
that the level is disjoint. A point lookup binary-searches one candidate table
per such level; a scan reads each such level through one concatenating source
that opens tables lazily, and `range()` skips tables wholly outside its bounds.
A level that is ever found overlapping falls back to probing every table, never
to a binary search it cannot trust. Iterators seek within their first block via
restart points instead of decoding it from the start.

## Maintenance and resource accounting

Shared maintenance uses PLAN/BUILD/COMMIT: capture a job, build outside the engine
gate, then apply a delta to the current topology. One frozen memtable may await
flush while a new active generation accepts writes. Cooperative flush progress
prevents long compaction builds from permanently blocking pending flushes.
The adaptive governor selects work from pressure and estimated costs; alternative
fixed/scored schedulers are available. Writer backpressure bounds L0 growth and
waits for pending flush capacity. Background errors are sticky and observable.

The block cache budgets resident data bytes, not all database heap allocations.
An optional range tier (`range_cache_bytes`, disabled by default) holds raw SST
read-ahead windows for sequential iterators under its own byte budget; each
iterator may additionally pin one window outside that budget.
The foreground SST descriptor cache has its own hard handle limit. Compaction
opens an independent descriptor only during a buffer refill, so its simultaneous
read descriptors do not grow with merge fan-in. Each input may still own a
read-ahead buffer. Snapshots, topology/filter/index metadata, mirrored memtables,
WAL buffers and SST construction are additional memory costs. SST builders keep
distinct key hashes for filter construction, rather than copied key payloads.

Governor batch averages use constant-space totals. Retired SSTs remain pinned
while readers/snapshots reference them. Failed unlink attempts stay queued for
retry; stats expose pending file bytes and failed reclamation attempts.

## Operations and limits

`checkpoint()` syncs accepted writes and flushes them; shared checkpoints also
wait for requested background work. Coordinate other writers for a quiescent
boundary. `close()` reports checkpoint failures and releases that handle; other
shared clones and pinned snapshots can keep ownership alive. Drop is not a
durability acknowledgement.

Logical backup exports a captured view into an exclusively created destination
database, flushing incrementally and syncing before success. A failed export may
leave a partial destination, which must not be treated as a completed backup.
Backup/restore does not copy tuning settings or historical versions. The offline
CLI opens through ordinary recovery and locking, checks every SST entry and live
point/scan agreement, and supports logical backup/restore. It can truncate a torn
WAL tail and sweep orphans; it is not a read-only forensic parser.

Current input limits are 1 MiB keys, 16 MiB values, 100,000 batch operations and
64 MiB WAL records. Oversized input is rejected before WAL I/O without poisoning
the database. Sequence arithmetic cannot wrap. Framed reads grow only as bytes
arrive; decoded counts are checked against actual encoded sizes. Old records
above the new limits are rejected explicitly. The write-buffer setting triggers flushing by cumulative mutation volume, so
hot-key overwrites rotate WAL generations even when resident memory stays small.
It is not a process RSS limit or a maximum single mutation size.

## Validation and remaining design work

Tests cover recovery faults, simulated volatile-device power loss, SIGKILL/reopen,
snapshot visibility, concurrent maintenance, malformed formats and batch
publication. Loom models cover the custom gate protocol; they are not models of
every database operation. SIGKILL preserves OS caches and is not physical power
loss. The historical concurrency decline requires fresh measurement before a
root cause or performance improvement can be claimed.

Future architectural work includes streaming SST construction and heap merge
selection, backed by profiling. Compression,
transactions, reverse iterators and range deletes remain outside the current
engine's supported API.
