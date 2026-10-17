# Compact resident locators and verified value reads

The existing format-4 point section contains 80.3 MB for the 1M-record
fixtures. The old reader reserves that section together with all sorted
data, or discards the section and uses sorted block reads. Neither the
combined region nor the unchanged section fits 32 MiB.

The candidate derives a smaller in-memory encoding from the same on-disk
section. It preserves format 3, format 4, the public read API, and the write
default. `mmap_max_level` continues to select mappings by level, with its
existing default and fallback on mapping failure. The compact path uses
positional value reads when a table has no mapping.

## Resident representation

The hash algorithm, seed, and bucket pilots are unchanged. Each resident
slot is eight bytes: a 48-bit address and ordinal within a compressed key
group, plus the existing 16-bit fingerprint. Groups restart every 16 keys.
Each key retains a prefix length, suffix bytes, and the byte length of its
version metadata. This lets lookup skip earlier metadata while rebuilding
the key. The complete reconstructed key must match the requested byte key.
The fingerprint only rejects misses.

Version metadata uses private varints for sequence, value length, and absolute
value offset, followed by the existing kind and a derived CRC-32/IEEE. It
retains every version in descending sequence order. Snapshot lookup returns
the newest visible version, including a tombstone. This representation is
never written to disk, so the private encoding requires no format version.

The existing global cache reservation charges exactly the resident records,
slots, and pilots. Boxed slices release excess vector capacity. Reservations
live with immutable SST handles and release on drop. Existing metadata and
allocator overhead remain outside this raw-byte budget, as before. Open
workspace includes the original point section, compressed output, and one
verified sorted block. Cache accounting is not a bound on peak RSS.

## Verification and ownership

Open verifies the point section's stored CRC and validates dimensions,
pilot bounds, slot bijection, sorted complete keys, version ordering, kinds,
lengths, and value bounds. A fingerprint match cannot bypass that validation.

The reader then walks sorted blocks through `TableFileCache`. Each block
passes the existing trailer checksum and layout verification. Record visits
reconstruct each key and require exact agreement with the point section's
key, sequence, kind, value length, and absolute value address. Coverage must
match in both directions. Only these verified value bytes generate the
resident per-value checksums. The sorted blocks are temporary and are not
admitted during this walk.

Every point value is copied into an owned `Vec<u8>`, through one positioned
read or the existing mapping. The reader checks this owned buffer against
the resident value CRC before returning it. Thus the returned allocation is
the allocation verified, including on the mapped path. Reads after file-page
eviction recheck their fetched bytes. No per-block "verified once" bitmap
substitutes for checking subsequently fetched values. Empty values and
tombstones rely on verified resident metadata and their empty checksum.

The derived metadata remains immutable in RAM with the SST handle. Changes
to on-disk key headers after open cannot redirect its addresses or sequences.
Corruption in a fetched value yields `SstError::Corrupt`; short reads do too.
Range reads, compaction reads, and unadmitted tables retain their existing
block verification. Existing mmap lifetime and immutable-file assumptions
still apply, including mapping survival after unlink.

Focused tests cover complete keys despite a forced fingerprint collision,
binary and empty keys, histories spanning blocks, snapshot limits, tombstones,
raw-byte accounting and release, sorted corruption before open, value
corruption after open for both access paths, short positioned reads, and
concurrent mapped reads after unlink. The existing mixed-format model and
crash suites remain required gates.

## Admission policy

On reopen, total referenced SST bytes determine the preference. If they fit
the cache capacity, the reader first tries the existing full resident path.
If they exceed it, the reader prefers compact locators for indexed tables.
It also tries compact locators when full admission fails. Reservations never
evict existing residents. Denied compact admission leaves the sorted path.
The remaining cache capacity admits hot verified blocks with the existing
CLOCK policy, including blocks used by short scans.

The prepass must classify an unreadable or missing referenced SST as
corruption, exactly as `SstTable::open` does. The first prototype leaked an
I/O error through `?`. The existing missing-SST test caught that regression;
the final prepass converts the error at the source.

The preference is chosen on reopen, rather than continuously reallocating
pinned table handles. Snapshots, compaction inputs, and table-open order can
still deny admission. Reopening recomputes the preference. This is not a
frequency-based table policy; no claim of learning hot tables is made.

## Cost and a future optional section proposal

The candidate pays a full data verification walk at indexed-table open when
constructing compact locators. Cold point probes run after this open and
after another file-specific page drop. They therefore measure cold point
faults and I/O, not the complete cold-open cost. Inspection receipts report
open cost and retained raw bytes separately.

A future optional versioned point section could persist checksums for value
records. Its section CRC would bind full keys, sequences, kinds, lengths,
addresses, and value checksums. Readers would still check every returned
owned value against its record checksum. Such a version needs mixed-version
support, structural coverage rules, independent builder/decoder corruption
tests, publication crash tests, and cold-open/RSS measurements. It could
avoid deriving checksums from a full sorted-data walk at each open. It is a
proposal only; no disk section or feature flag is added in this task.
