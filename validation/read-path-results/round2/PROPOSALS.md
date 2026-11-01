# Read-path round 2 proposals

These changes are proposals only. None changes the SST format, public API,
or the meaning/default of `mmap_max_level` in this branch.

## Borrowed read results

An additional closure-based point/range API could let an application decode
values directly from a verified block while its owner is pinned. Keep the
existing owned `get` and iterator APIs. A closure is easier to bound than a
public cache lease: it cannot accidentally hold every evicted block forever.
Specify reentrancy, error propagation and behavior during compaction before
implementation. Measure the complete decode workload, including concurrent
writers, rather than only the removed allocation. No measured gain claimed.

## Point-only mapped access

The existing `mmap_max_level` selects mapping by table level and remains
unchanged. Enabling mappings independently for point reads would need a
separate explicit option and documented address-space/page-fault behavior.
A candidate must cache successful block verification once per immutable
mapping, retain the mapping through the entire read, preserve error reporting,
and test reads after compaction unlinks a file. Compare truly cold single-
and multi-reader p99/p999 with positional reads, as well as warm point reads.
Do not silently enable this path when `mmap_max_level` is `None`. No new
option is implemented here. A prototype within the existing mapping opt-in
was measured and rejected; see [REJECTED.md](REJECTED.md),
[MAPPED.md](MAPPED.md) and [COLD.md](COLD.md). A separate point-only option
remains a proposal, with no claimed gain.

## SST point index

A future format version could add independent keys/offsets or denser restart
metadata to reduce prefix reconstruction. Keep checksums, sequence visibility
and tombstone semantics explicit. Specify cross-version readers, migration,
write amplification and file-size costs first. The rejected round-one
allocation-free comparison saved instructions but added branch misses; its
2.432 → 2.752 µs normal-layout full-read result is evidence against assuming
that a smaller decoder automatically makes reads faster. See the round-one
`POINT_COMPARE_PAIRED_NORMAL.md` and `BLOCK_COMPARE_PROTOTYPE.md` evidence.
