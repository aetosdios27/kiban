# PH3 locators with verified external values

This implementation extends commit 4feca511, not the private PH2-derived codec
preserved in experiments/ph2-locators.patch. The final branch is rebased onto
main 37feee99, where PH3 was squash-merged as f3d21dc1. Main's resident-index
Bloom bypass is retained, with locator hits using the same ordering.
PH3 flag zero remains byte-for-byte
compatible. The new writer sets feature bit 0 at byte 52 and appends one u32
CRC-32/IEEE immediately after each version's value address. Its version stride
is S + L + W + 4. The checksum covers the exact value bytes; zero is a valid
checksum, including for empty values and tombstones. All other PH3 fields,
hashing, anchors, pilots and slot meanings retain the shared encoding.
Unknown flags cannot produce a point address: admitted readers reject them;
cache-denied readers verify the section CRC and use verified sorted blocks.
PH2 and flag-zero PH3 retain their existing full-resident or sorted fallback.

On open, the manifest's referenced SST lengths select an admission preference.
If their total exceeds the cache capacity, CRC-bearing PH3 tables try locator
admission rather than spending the budget on complete values from early tables.
Missing referenced files remain corruption, matching the existing engine.
This changes admission only: existing snapshots retain their reservations.
When the dataset fits, the existing full-resident path and shared scan backing
remain available. The normal block CLOCK cache receives the budget left over. Reservations are
non-evicting and admission is decided when each table handle opens. Denied
handles keep the verified sorted fallback; there is no adaptive promotion after
snapshots or overlapping compactions release reservations. Legacy PH2 and
flag-zero PH3 do not receive locator-only admission until rewritten by the new
writer.

The resident locator view keeps PH3's arena verbatim. Its hash tables omit the
u16 fingerprint and pack pilots into three bytes, relocating DIRECT from bit 31
to bit 23. Key equality still compares both PH3 key spans. Admission requires
fewer than 2^23 distinct keys, representable data addresses, and packable pilots;
otherwise the reader falls back safely. This is an in-memory layout only.
Resident locator-buffer bytes are exactly section length minus bucket count minus twice key
count, charged to the existing global cache with a lifetime reservation. The
original disk section is temporary and is released after validation and packing.
Fixed table bookkeeping, temporary allocations, allocator retention, the ordinary
sorted index, and OS
page cache remain outside the block-cache byte accounting; this is not a bound
on process RSS or peak open memory.

Verification has two boundaries. Before publishing locators, the reader checks
the point-section trailer CRC, validates all dimensions and strides, streams
and checksums every sorted block, and matches each locator to the exact sorted
key, descending sequence, kind, length and absolute value address. It validates
the new per-value checksum against that verified sorted value. Anchors, full
key order, coverage, computed hash slots and fingerprints retain PH3's semantic
validation. The resident metadata is owned and immutable after publication.

For every external value hit, the reader copies exactly the selected value
bytes, either by pread through the file cache or from an already permitted
mapping. It then computes the value CRC on the owned return buffer and compares
it with the validated resident checksum. A changed mapped page cannot change the
returned buffer after verification. Short reads, I/O failures and CRC mismatches
return corruption. Whole-block checksums are not repeated for these point hits.
Admitted point indexes use their validated full-key comparison directly, without
a redundant Bloom membership check. The Bloom section is still checksummed at
open, and sorted fallback reads retain Bloom filtering. Full-resident hits copy
from already verified owned data; sorted fallbacks and
scans retain their existing whole-block verification. No verification-once bit
allows subsequent unchecked reads from file pages. Verification granularity is
the returned record value: corrupt bytes in an unrelated sorted record or header
are checked when a sorted block is read or on the next locator validation at
open. They do not alter the already validated resident key/MVCC/address metadata
or the independently verified value returned by a locator hit.

The implementation creates no mappings beyond the existing mmap_max_level
selection. None remains pread. Some(L) retains exactly the existing level rule,
best-effort mapping fallback, immutable SST lifetime and unlink guarantees.
External truncation of a live mapped SST remains outside that existing contract.

The decision to ship depends on five alternating pairs with adjacent LMDB
controls: warm 32 MiB point p50 <= 0.9 LMDB and p99 <= LMDB for bulk and normal,
plus larger-cache, latest-K, cold concurrent tail and write controls. Measurements
must label mmap None and Some(u32::MAX), include all rejected candidates, and
separate retained heap data from file-page cold tests.
