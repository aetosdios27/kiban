# Compact point index: KIBANPH3

The SST container remains format 4, with the checksummed 24-byte footer extension
and locator feature flag 2. New writers emit section magic `KIBANPH3`. Readers
accept `KIBANPH2` and `KIBANPH3`, including mixtures with format-3 tables,
independently of `KibanOptions::point_index`. The option still defaults to false.
Older PH2-only engines explicitly reject PH3. Rewrite tables with the option off
before downgrading to such an engine. Trivial compaction moves preserve bytes.
Rewriting compactions use the current writer.

All integers are little endian. The 64-byte section header is:

- 8 bytes magic, u64 hash seed, u32 distinct key count N, u32 bucket count B;
- u64 arena offset;
- six u32 fields: address width W, key-length width K, version-count width C,
  sequence-delta width S, tagged-value-length width L, reserved zero;
- u64 minimum sequence number.

W is 4 or 8. K and C are 1, 2, or 4. S is 1, 2, 4, or 8. L is 2 or 4.
B is ceil(N/2). The existing PH2 hash, fingerprint, bucket reduction and pilot
interpretation are unchanged. There are B u32 pilots followed by N slots of
W-byte arena-relative record offset and u16 fingerprint. PH2 retains its own
ceil(N/3) dimensions and twelve-byte slots.

The arena contains one record per distinct key, in key order. Every group of
1,024 keys starts with an anchor whose full key is stored. Each record has a
W-byte arena-relative address of its group anchor key bytes, K-byte shared prefix
length, K-byte suffix length, C-byte version count, suffix bytes, and versions.
Anchor shared length is zero. Keys compare as two byte spans. A point probe does
not reconstruct or allocate a key or search a sorted block.

Each version stores an S-byte sequence delta from the header minimum, an L-byte
tagged value length, and a W-byte absolute value address in the original sorted
data region. The length high bit marks a tombstone, whose length must be zero.
Versions remain sequence-descending. A snapshot selects the first visible
version. Values appear once on disk. Keeping MVCC fields with the value address
avoids a dependent sorted-header read before the value copy.

W=4 only when the finished narrow arena and every value address fit u32.
Otherwise W=8. Each remaining width is the smallest permitted width that fits
the table maximum. The length high bit is reserved for kind. Readers accept
valid wider encodings too. Full-width addresses are compared before narrowing
during validation, including on 32-bit readers. The whole section ends with the
usual zero type byte and CRC-32/IEEE trailer. No native struct layout is persisted.

On admitted open, one ordered walk of the checksummed sorted blocks validates
key order, lengths, version order, exact value coverage, every anchor/prefix/
suffix, and the computed hash slot and fingerprint. It compares sequence, kind,
length and value address with every sorted record, and rejects overflow in
sequence reconstruction. Extra or missing records and pointers to other in-range
values are corruption. Only validated addresses reach point lookups. PH2 keeps
its existing structural and backing validation. On cache denial, both encodings
stream their section checksum in bounded chunks and fall back to sorted reads.
Point section and complete sorted backing remain charged together to the
existing cache budget. Scans share verified backing blocks.

Construction distributes key indices into contiguous bucket ranges using
counts and prefix sums. Equal hashes must share a bucket, so a local pair check
replaces the global hash sort. Buckets are still placed largest first with
bounded retries and an explicit bijection check. Mean bucket size two increases
pilots from about 1.333 to 2 bytes/key but leaves more singleton slots, reducing
expensive late multi-key pilot searches. Whole-SST builders still retain a
temporary full-key MVCC arena and MPHF workspace. This change does not introduce
a hard process-memory bound.

Publication, durability, cache admission, public APIs and cross-table sequence
resolution are unchanged. Focused tests exercise PH2/PH3/format-3 mixtures,
checksummed malformed fields, both address widths, full-key fingerprint
collisions, multiple anchor groups, binary/empty/long keys, large values,
sequence extremes, wide version counts, tombstones, snapshots and reservation
release. The two crash suites retain seed-parity format-3/PH3 writes and
format-switching reopens.
