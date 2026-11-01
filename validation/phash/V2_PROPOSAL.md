# Optional immutable point-index SST format

This is a proposal for the next isolated prototype. It is not an engine patch or a production commitment. The measured experiment uses decoded blocks and empty memtables. Preserve the real engine's visibility and recovery rules before measuring integration.

## Persist the index inside each SST

A v2 SST retains sorted data blocks and the ordered block index for scans. Add typed metadata sections for point-index parameters, PtrHash pilots, minimal-remap data, and slots. A checksummed metadata directory records each section's offset, length, encoding version, key count, and checksum. A v2 footer identifies that directory and the existing sorted-index/filter sections. It needs a distinct format version; existing readers must reject v2 explicitly. New readers continue accepting v1 through the existing path. A downgrade requires rewriting v2 tables as v1. The manifest can continue identifying tables by file number and level, since format is detected from each SST footer.

The MPHF section specifies algorithm version, hash algorithm and seed, parameters, slot count, pilot/remap encoding, and byte order. Freeze a documented encoding or vendor a small stable encoder. Do not persist Rust structs, pointers, Vec headers, or an undocumented crate serialization ABI. Build-time collision detection must retry the full-key hash seed or choose a wider input representation; a collision cannot silently coalesce two user keys.

A compact slot is `{u32 block_offset, u32 entry_offset, u16 fingerprint, u16 flags}`. Limit this encoding to files with block offsets below 4 GiB; use a separate 16-byte wide-offset encoding for larger files. The decoded prototype uses 12 bytes per slot, including padding. A section flag identifies which encoding applies. The fingerprint only rejects nonmembers. A complete key comparison remains mandatory, including for fingerprint collisions.

## Make records independently verifiable by key

The current prefix-compressed entry cannot verify its key from an entry offset alone. Test either full keys inline in v2 point-addressable records or an explicit full-key side section. Inline full keys keep the intended pilot → slot → record dependency chain; a separate side array may add a load. Keep sorted order for scans in either case.

Preserve record kind and sequence number, plus length bounds for arbitrary binary keys and values. The measured u16 lengths are a workload specialization and must not become a general engine limit. Use bounded variable-width lengths or a general u32 encoding. A slot identifies the newest version's entry or a version-chain head for that user key. Snapshot reads must select the newest version at or below their sequence. A tombstone is a successful logical match that stops lower-level probing, not an absent fingerprint. If compacted output contains versions of one key, construct the MPHF over distinct user keys and retain the sequence-ordered versions behind its slot.

## Build before publishing the SST

Flush and compaction already produce sorted output and block offsets. While writing each output SST, collect distinct user-key hashes and their block/entry offsets. After data blocks finish, build its MPHF, place slots, verify collision freedom and slot bijection, then write index sections, metadata directory, and footer. Verify output bounds and checksums before installation. Build one independent index per output SST so no global mutable index or cross-SST transaction is introduced.

Account for buffers in the existing compaction budget. The tool's total construction time includes work already performed by a normal compactor, such as decoding and CRC checks. Its MPHF-only timer estimates one part of added CPU cost. It does not measure the full cost of retaining input hashes, writing new sections, or extra I/O from larger full-key blocks. Stream/spill hashes or bound SST output size when builder workspace would exceed the compaction budget. Never weaken durability to hide construction cost.

## Budget resident bytes and retain a fallback

Charge mapped/resident MPHF sections and slots to the read-memory budget, together with decoded blocks and other indexes. The experiment's bytes/key and total decoded footprint are in REPORT.md. Restoring sequence, kind, general lengths, and snapshot history increases that footprint. Builder peak memory is additional and was not bounded by the read-cache budget. Per-SST pilot/remap metadata and Vec-like control structures add overhead for small tables.

Load and verify metadata once before publishing the table into a version. Keep mappings alive through the same version/ownership mechanism as existing SST readers. If the optional point index cannot be made resident within budget, use the sorted lookup path. Keep Bloom filters available: this experiment's negative results must guide whether to probe a Bloom first, particularly for lower levels that do not contain the requested key. Measure the resulting positive/negative workload mix instead of assuming the extra filter is free.

A lookup first checks active and immutable memtables, acquires the visible table version, then probes tables in existing recency order. Higher-level range routing still picks at most one candidate SST per level. Each MPHF is a locator within that candidate, not a replacement for level ordering. Range scans and latest-K continue to use sorted data/indexes and receive no assumed benefit from the point index.

## Preserve the existing installation protocol

Write the complete new SST to its temporary/unpublished filename. Finish every data and index section, checksums, and footer before file sync. Publish through the existing rename, directory-sync, and manifest-commit sequence. Only retire input SSTs after the new manifest is durable and old versions no longer reference them. An interrupted build leaves an unreferenced incomplete output to clean up, never a manifest-visible partial MPHF. Metadata corruption must fail table validation or use an explicitly trusted sorted fallback; it must never turn a present key into a missing result. No new independently committed sidecar is needed.

Future crash tests should interrupt construction, section writes, footer writes, sync, rename, manifest installation, and input retirement. Validate negative fingerprints and malformed offsets as untrusted on-disk data. This proposal runs no crash tests and does not interfere with the ongoing sweep.

## Gate the next prototype on integrated behavior

First preserve versions, tombstones, binary keys, range scans, and mixed v1/v2 recovery in a separate experiment. Then repeat CPU-8 controls with normal memtable/version overhead and the same 512 MiB total read budget. Include negative-heavy workloads, scans/latest-K, compaction CPU/bytes written, startup index loading, and checksum validation. Add a sorted/fence-search control over exactly the same decoded arena to separate locator effects from representation, cache bypass, and page size. Compare full keys inline with compressed blocks plus a full-key side section. Repeat with a quiet sibling and controlled huge-page settings before attributing the speedup to address dependencies alone. A global SST maplet remains an optional later experiment because it adds memory and lifetime complexity across compaction.
