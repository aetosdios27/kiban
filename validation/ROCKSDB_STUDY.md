# RocksDB mechanisms relevant to Kiban

This study reads the RocksDB 11.8.1 source shipped in the locked
`librocksdb-sys 0.19.0+11.8.1` dependency, the same implementation used in the
comparison campaign. It is a targeted study of write, read, flush, compaction,
WAL and version-management paths, not a claim to have audited every RocksDB
feature. Source hashes are retained with the foreground diagnostic evidence.

## Write coordination and waiting

[`db/write_thread.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/write_thread.cc)
coordinates queued writers, selects a leader, and joins compatible writers into
a group. `AwaitState` first tries a short pause loop, optionally performs bounded
adaptive yielding, then blocks on a condition variable. It reduces futile CPU
work without treating a timing hint as the publication protocol.
[`db/db_impl/db_impl_write.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/db_impl/db_impl_write.cc)
coordinates WAL order, sequence allocation and memtable publication for those
groups; pipelined writes additionally allow separate groups' WAL and memtable
phases to overlap. These mechanisms have correctness machinery Kiban does not
currently possess. They are not just a switch from a mutex to an atomic.

Kiban serializes individual mutations under its existing engine gate, then
mirrors them into a published memtable. Rust RAII guards make partial acquisition
and unwind cleanup explicit, but do not remove serialization or allocation cost.
The current acquisition sampler separates engine, memtable and read-view waits.
Group commit/pipelining should be a separate design only if measured WAL/engine
serialization remains limiting after the smaller fixes. Snapshot sequence
publication, batch atomicity, failure poisoning and sync coverage must be modeled
before attempting that design.

## Flush progress and compaction pressure

[`db/db_impl/db_impl_compaction_flush.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/db_impl/db_impl_compaction_flush.cc)
uses high-priority scheduling for flushes and lower-priority compactions, with a
documented fallback when the flush pool is empty. It has explicit job limits,
shutdown/error checks and queue accounting. This prevents a full compaction from
being the only way a queued flush can get CPU time.

Kiban's cooperative checkpoints achieve some of that progress with its one
existing worker. The sustained-write measurements show the remaining ceiling is
L0 capacity. Adding another worker would require proving concurrent plan
validity, file lifetime and durable publication ordering, rather than simply
spawning a thread. This pass first reduces measured BUILD CPU cost.

[`db/compaction/compaction_job.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/compaction/compaction_job.cc)
separates prepared input, key/value processing, output construction and result
installation. Kiban already has PLAN/BUILD/COMMIT and pinned immutable inputs.
Retaining that architecture is appropriate; Rust ownership can express input
and output lifetimes without changing the logical visibility/drop rules.

## Checksums and read selection

[`util/crc32c.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/util/crc32c.cc)
processes chunks and selects implementations according to its build/platform.
Its checksum is CRC32C, not Kiban's CRC-32/IEEE. Substituting the x86 CRC32
instruction would therefore corrupt compatibility with existing Kiban files.
The useful lesson is to remove the serial byte-at-a-time dependency while keeping
Kiban's polynomial and framing. A portable slicing-by-eight implementation uses
safe fixed-size slices, explicit little-endian loads, and no dependency or unsafe
code. Independent bitwise tests cover alignment, tails and arbitrary streaming
state. Existing corruption checks must still execute.

[`db/version_set.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/version_set.cc)
uses `FilePicker`, per-level search state, range filtering and file metadata to
avoid probing irrelevant tables. Kiban's corrected resolver compares sequence
numbers across all candidates; file/level order cannot replace that rule.
Safe range exclusion can reduce irrelevant probes, but early termination needs
proven sequence bounds. Sequence metadata or a new format is not introduced
merely to match RocksDB's implementation.

## Durable installation and read lifetimes

[`db/flush_job.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/flush_job.cc),
[`db/memtable_list.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/memtable_list.cc)
and `VersionSet::LogAndApply` separate building an SST from installing flush
results and advancing durable version/log state. Kiban's WAL must likewise remain
live until its records are represented in committed durable storage, not merely
until a memtable is frozen. Failure/rollback paths matter as much as success.

[`db/db_impl/db_impl.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/db_impl/db_impl.cc)
references a SuperVersion for reads and releases it through cleanup machinery.
Kiban's `Arc<ReadView>` and pinned `Arc<Version>` serve the corresponding lifetime
purpose, with different update costs. Rust makes lifetime ownership explicit;
it does not establish correct logical sequence visibility automatically.
[`db/db_impl/db_impl_open.cc`](https://github.com/facebook/rocksdb/blob/v11.8.1/db/db_impl/db_impl_open.cc)
recovers live WAL generations and propagates recovery errors. Kiban's deterministic
fault tests and reopen verification remain mandatory for every adopted change.

## Adoption policy

Use profiled mechanisms, not RocksDB's headline numbers or defaults as targets.
Keep original Kiban workloads and durability settings fixed. Measure allocation,
checksumming, lock waits and table probes separately. Prefer safe Rust and RAII
where they express the same algorithm cleanly. Preserve storage compatibility,
snapshot/tombstone rules and existing fault assertions. Large architectural
changes need their own evidence and concurrency models; they are deferred here.
