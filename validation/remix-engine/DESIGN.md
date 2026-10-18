# PH3 scan extension

The feature is opt-in: `point_index = true`, `scan_prefix_len = 1..=24`.
Zero leaves the frozen PH3 encoding unchanged. Prefixes are fixed byte slices,
with their length persisted in each table; they are not UTF-8 or delimiter
assumptions. Incompatible extractors, PH2, format 3, wide arenas, and cache
admission denial use sorted ranges. The original owned range API is unchanged.
The `range_values` API maps borrowed values into owned callback outputs.
`prefix_values` accepts the natural prefix query. An exact prepared extractor
prefix uses its validated population extent directly. Other finite prefixes up
to 24 bytes derive their upper bound on the stack. Empty, longer, and unprepared
all-255 prefixes use the sorted prefix iterator. It caches no results.

PH3 flag bit 1 advertises `KIBANSC1` version 1. Bit 0 remains reserved and is
not interpreted. The PH3 arena, pilots, slots, key headers and MVCC version
strides are unchanged. Immediately after the existing arena comes:

* magic `KIBANSC1` (8 bytes);
* fixed-byte prefix length (u32 little endian);
* anchor stride 32 (u32 little endian);
* distinct prefix count (u64 little endian);
* one arena-relative PH3 key-group offset per distinct SST key, using PH3's
  existing address width;
* extension byte length, excluding this field (u64 little endian).

The decoder retains only the scan-header offset in the PH3 handle; extractor
configuration stays in its checked section bytes. Builder output capacity
includes the extension before serialization, avoiding a doubling reallocation.

The existing type byte and CRC follow the length. The CRC covers the whole
PH3 section including the extension and length. The footer checksum and file
installation boundary are unchanged. Readers validate every scan offset against
the independently validated, sorted PH3 group position and recompute prefix
coverage before admitting the section. Cache-denied readers stream the section
checksum before taking the unchanged sorted-block path. Older readers reject
the nonzero flag when admitting PH3; their verified sorted fallback remains
valid. Disable the extension and rewrite tables before using an old admitted
reader. No second key arena, value arena, or MVCC history list is built.

A pinned Version owns a derived global ordering across **all its SSTs**, using
8-byte source-slot/PH3-offset locators. Bit 63 marks equal-key continuation.
Runtime selection resolves all equal groups at the snapshot sequence before
returning a Put or tombstone. A 64-byte open-addressing directory stores each
prefix's population extent, first locator, and first unique contiguous source
span. Inline 48-byte anchors every 32 groups accelerate other lower bounds;
long keys compare against the full two-span PH3 key. Strict lower-bound anchor
search preserves equal groups crossing anchor boundaries.

A prefix-aligned start uses the first locator already stored in its directory
slot and its first source span directly. PH3 widths are dispatched once per
span. At the unbounded current sequence, the loop reads the first version
directly; snapshot spans retain explicit visible-version selection. A prefix extent
proves the upper bound once when the exclusive end is after the prefix's whole
population. Tight upper bounds keep full-key comparisons. A direct value loop
still selects a visible version and handles tombstones. The remaining SST
stream merges only with the mutable and frozen memtables. Arbitrary ranges
that can cross extractor prefixes use the existing engine merge.

Publication does not build the global view. The first eligible `range_values`
requests a background build against that exact Version and uses sorted scans
until it is ready. `prepare_range_index` lets callers explicitly prepare it
before latency-sensitive work. Build failure or budget denial leaves the
sorted path available. Weak startup ownership and periodic cancellation avoid
retaining a retired topology after all readers release it. Old snapshots can
keep their own views and shared table backing. Views do not retain their
own Version (no ownership cycle), and cannot publish into a newer Version.
Unchanged-prefix sharing between different Versions is not implemented.

The global locator stream, anchors and directory are reserved against the
existing block cache before allocation. Build workspace has a separate temporary
reservation. PH3 backing is owned and charged once by the existing point index;
scan locators borrow it through their pinned Version. Multiple pinned Versions
have additive derived-view reservations. No result page or decoded Memory is
cached. Timing preparation, full output verification, and JSON decoding remain
explicit in the measurement protocol.
