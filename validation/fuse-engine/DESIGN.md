# Static SST filter integration

Base: fetched main 76a6a190. Existing TableBuilder retains distinct user-key
hashes, emits sorted blocks and optional PH3, then Bloom, index and footer.
Both foreground and planned flush/compaction construct whole immutable byte
buffers before atomic publication. SstTable validates footer, filter CRC and
index on open; its point lookup tries admitted PH3 first. Filter metadata is
owned by the table as before, outside block-cache admission.

Chosen interface: KibanOptions::binary_fuse_filter selects new writes.
TableBuilder::with_filter(bool) composes with with_point_index. A private
SstFilter enum hides legacy Bloom, fuse8 and always-positive fallback. Legacy
versions 3/4 retain their exact encoding. New versions 5/6 use block type 1,
KIBANFL2 revision 1 and the proposal's 48-byte header. Version 6 keeps the PH3
extension and checksum. Header, dimensions and CRC must validate before use.

Alternative considered: keep footer versions 3/4 and change the block trailer.
Rejected: older readers must reject the format before treating filter bytes as
Bloom. Another alternative, tagged Bloom fallback, requires retaining both FNV
and XXH3 hashes or rescanning solely for fallback. Explicit always-positive is
smaller and correctness-preserving, with bounded retries and workspace.

Construction uses deterministic graph seeds, deduplicated hashes (equal hashes
safely represent all colliding keys since lookup depends only on the hash), and
bounded workspace. Any duplicate distinct-key hash instead falls back positive
so collision handling remains explicit. Source keys are already deduplicated
by builder ordering. Every encoded/decoded new filter is checked against the
sorted source blocks in release as well as debug before publication, including
tombstones and retained versions. This costs a source scan; measurements decide
whether default-on is justified. No source-key copy is retained for the scan.

XXH3 is a safe scalar Rust port of seed-zero/default-secret XXH3_64bits from
xxHash v0.8.3 (BSD-2). Reference C vectors cover all short lengths, stripe/block
boundaries, long and binary data. Fuse geometry and peeling follow the supplied
experiment and FastFilter reference (Apache-2.0). No runtime dependency added.

The expected performance claim is faster nonresident/index-off SST point
lookups and absent-key lookups, with unchanged resident-PH3 and latest-K paths.
Five alternating paired trials, LMDB control, actual sibling activity, complete
work/error counts and maintenance bytes/times constrain any recommendation.

Verification counters are read from the persisted algorithm tags during fixture
inventory: algorithm 0 counts safe construction fallbacks. This avoids new
lookup counters or atomics on PH3/the filter path. Construction does not retain
a second set of FNV hashes for rare fallback. The bounded 64 MiB workspace
budget covers graph arrays/queue/stack but excludes the existing input hashes
and whole SST output. Legacy metadata accounting remains unchanged.

Missing-ID workloads use high-half IDs proven disjoint from the 0..n fixture.
Uniform selects across all 64 agents; agent-prefix fixes agent 7. These are
engine-level misses: range routing may reject them before the SST filter.
Results must not be reported as forced filter-query microbenchmarks.
