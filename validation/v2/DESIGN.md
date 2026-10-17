# Optional SST point index (format 4)

Status: implemented design, selected after the completed five-trial attribution ablation.
Bulk raw MPHF p50 is 465 ns (THP allowed) / 532 ns (disabled), versus
2,851 / 3,203 ns for binary search in the identical decoded arena. Normal is
741 / 749 ns versus 3,435 / 3,654 ns. See ablation/REPORT.md for paired controls
and the limits of separating representation, locator and page-size effects.
The baseline is main `2a083fa`; the isolated experiment is imported as `71acf68`.
No public read API, sequence merge rule, or publication protocol changes.

## Decision and scope

Use a per-SST minimal perfect hash over distinct byte keys. Preserve sorted,
prefix-compressed blocks for range/latest-K and compaction input. Add a redundant
full-key point-record arena so a hit compares the entire key without restart
reconstruction. A fingerprint is only a rejection filter. Retain Bloom-first
lookup initially: the prior gap-miss experiment favored it, especially on L0.
The decoded-arena binary-search and THP ablations determine whether this combined
layout has an attributable advantage; they do not predict integrated timings.

`KibanOptions::point_index` controls **writing**, defaults to false, and is
captured by both direct/background flush and compaction plans. Readers accept
old format 3 and new format 4 independently of this write option. Unsupported
formats fail explicitly. Compaction rewrites into the configured format;
zero-copy trivial level moves preserve existing file bytes.

## Stable encoding

All integers are little endian; no Rust struct or third-party serialization ABI
is persisted. Format 3 is unchanged. Format 4 layout is:

    sorted data blocks | point section | Bloom | sorted index | footer extension | original-shaped footer

The trailing 44 bytes retain `KIBANSST` and all existing field positions, with
format=4. A 24-byte extension immediately precedes them: u64 point offset,
u64 point length, u32 features (=1), u32 footer CRC-32/IEEE. The footer CRC covers
the other 20 extension bytes and all 44 trailing bytes. Index/filter placement
must be exact, not merely in bounds. Thus a single-bit format downgrade cannot
silently hide the point section. Existing format-3 readers reject format 4.

The point section has an algorithm/version header, seed, distinct-key count,
bucket count, record-arena offset, bucket pilots, and slots. Pilots use u32:
high-bit clear is a displacement; high-bit set encodes a singleton's direct
slot. There are exactly N slots, each u64 arena-relative record offset,
u16 fingerprint and u16 reserved (=0). The algorithm limits N below 2^31;
file/record offsets are u64 rather than imposing a 4-GiB file limit. The section
ends with the standard type=0 and CRC-32/IEEE trailer, covering every byte.

Each arena group is u32 key length, u32 version count, full key, then versions
in descending sequence order: u64 sequence, u8 kind, u32 value length, value.
Lengths retain the engine's general key/value limits. A tombstone has zero value
bytes. A snapshot selects the first sequence <= its limit; no visible version
means no candidate from this SST. A visible tombstone returns Found(Tombstone),
so the unchanged cross-table sequence merge can hide older values correctly.

## Self-contained hash construction

Implement bucket-displacement minimal perfect hashing using only std, under the
repository's MIT license. Hash the full byte key with a seed-dependent chunk
hash; use independent avalanche mixes for bucket/position/fingerprint. Process
multi-key buckets largest first, searching a bounded displacement that maps all
members into unused slots. Assign singleton buckets to the remaining slots.
Validate input-hash uniqueness and every slot's bijection. Retry with a new seed
on a collision or exhausted pilot search; bound retries and return an error
rather than coalescing keys or publishing a partial index. The on-disk algorithm
version freezes these functions and pilot interpretation. This favors a small,
auditable implementation over PtrHash's tighter bits/key.

## Trust, memory, and ownership

Check the new footer and entire point section before publishing an SstTable.
A resident section is structurally validated: bounds, lengths, kind, sequence
ordering, sorted distinct keys, exact record coverage, and agreement between
each key's computed slot, stored offset and fingerprint. This proves each
lookup address is an actual record head. No checksum runs in a point lookup.
Ordinary data blocks retain their existing verify-on-load path.

Reserve resident section bytes from the **same global block-cache capacity**.
An RAII reservation lives with the immutable SstTable and is released on drop,
including failed opens. Blocks use the remaining capacity; reservations and
block insertions are serialized by the existing cache lock. Point hits only
borrow immutable section bytes and acquire no cache lock. Budget pressure
(including old snapshot/compaction input handles) may deny admission. In that
case verify the section in bounded chunks, retain no point bytes, and use the
existing sorted path. A checksum failure is always corruption, including when
admission is denied. No separate unbounded point-cache budget is introduced.

Full-key/value duplication increases disk bytes and write work. Builder size
estimates include the new section so compaction output targets account for it.
Existing whole-SST buffering remains: arena and construction vectors are extra
workspace proportional to one output, with the existing soft target-file bound
and existing oversized-key/version-group exception. This is not a new hard
bound on total process RSS. Report workspace and duplicated disk space honestly.
Resident admission is table-granular and favors tables opened first; late output
may fall back while input versions still pin reservations. Reopening can change
admission. This is an explicit initial policy, to be measured rather than hidden.

## Installation and verification

Build every byte (section, checksums, footer) before `atomic::commit_file` writes
the unpublished temporary SST. Keep its write/fsync/rename/directory-fsync and
manifest commit sequence unchanged. Never install a separate sidecar. Failure
before manifest commit leaves the old WAL/input tables authoritative; recovery
sweeps incomplete/unreferenced outputs through the existing ownership rules.

Tests must cover bijection, forced fingerprint collisions, binary/empty/long
keys, absent keys, versions/snapshots/tombstones, mixed old/new tables, scans,
checksum and malformed-layout rejection, cache reservations under concurrent
admission/drop, and power loss during new-section publication. Seeded suites
must exercise both write formats and mixed-format reopens. Run all existing
gates, then 20,000 seeds for each final direct/shared crash model. Measure the
real public read API against frozen baseline/LMDB, five alternating trials per
layout/cache, plus writes, disk bytes, construction and compaction costs.

Full-key expansion also changes PLAN's output-file reservation: the old
input_bytes/target bound is unsound when converting compressed format-3 SSTs.
Reserve a range of identities using 2*RESTART_INTERVAL+8 as a conservative byte
expansion bound (both sorted output and arena may contain every reconstructed
key; values duplicate twice; slack covers slots/headers). A Range avoids an
allocation proportional to this deliberately loose identity bound. A dedicated
conversion test must exceed the old bound and verify every output value.
