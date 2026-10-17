# Rejected point-read experiments

All full-read pilots below use five alternating frozen before/after trials,
1M records, bulk layout, 512 MiB, CPU 3, and contemporary LMDB controls.
Every timed result and both complete dataset passes were verified.
Numbers are median trial p50 / p99, in microseconds. Complete trial ranges,
throughputs and controls are in the linked reports. Each prototype passed
workspace debug/release tests, Clippy, formatting, fixed crash corpora and
loom before the retention decision. The mapped full gates followed its warm pilot; it also completed the full matrix. The handle tests include 160,000 concurrent reads
against 64 keys with only four 256-byte blocks admitted at once.

| Prototype | Point before | Point candidate | LMDB candidate control | Latest-K before → candidate |
| --- | ---: | ---: | ---: | ---: |
| Weak resident handles | 1.696 / 2.432 | 1.792 / 2.688 | 1.472 / 2.048 | 3.072 / 4.736 → 3.264 / 5.120 |
| Explicitly invalidated slots | 1.696 / 2.432 | 1.824 / 2.752 | 1.472 / 2.496 | 3.136 / 4.992 → 3.072 / 4.864 |
| Inline aligned slots (bulk/512) | 2.112 / 3.200 | 1.984 / 2.944 | 1.664 / 2.432 | 3.776 / 6.912 → 3.648 / 7.296 |
| Verified mapped point path (opt-in) | 2.496 / 3.840 | 2.560 / 3.904 | 1.728 / 2.624 | 4.224 / 8.960 → 4.224 / 8.704 |

[Weak-handle pilot](weak-handles/bulk-512/report.md),
[invalidated-slot pilot](slots/bulk-512/report.md), and independently
applicable source patches under [experiments](experiments) preserve the work.
Neither removes enough complete-read latency to justify shipping it.
Weak handles also add an indirection to ordinary cache-map reads, hurting W3.
The invalidated design binds at most one slot per resident and clears the
previous binding when a trivial move reopens a table; tests prove both
rebinding and eviction release payload ownership.

After these two failures, the premise was tested directly rather than
assuming the cache's profile percentage was entirely hashing. Fresh baseline
component measurements were 36.094 ns for hashing, 174.388 ns for a complete
cache hit, 10.197 ns for an uncontended read lock and 9.090 ns for Arc cloning
([raw runs](profile-slots)). These are overlapping isolated costs from one
diagnostic run, not additive estimates or repeated speedup measurements.
The full baseline profile attributes 19.16% of samples to `BlockCache::get`;
the candidate moves that work into the inlined point-read parent and still
performs cold metadata/ownership accesses. Removing a hash does not remove
those memory accesses. The weak prototype's one diagnostic counter run
reduced instructions 8253 → 7954/op, but increased cycles 6208 → 6358/op and
branch misses 2.73 → 3.67/op. This does not establish a performance win;
the five complete latency trials decide rejection.

No mapped default was implemented. Changing the existing off-by-default,
level-based mapping contract would conflict with the brief; the separate
point-only option and its required cold/concurrent tests are proposed in
[PROPOSALS.md](PROPOSALS.md).

The aligned inline-slot candidate had the pointer in the index entry, reduced
inline separator capacity from 46 to 38 bytes, and aligned slots to one cache
line. Its [complete matrix](INLINE_SLOTS.md) exposed the cost under pressure:
bulk/32 point p50/p99 4.736/7.808 → 5.120/8.960 µs; normal/32
4.992/8.704 → 5.248/10.240. Normal/512 point p50 stayed at 2.240 µs.
The modest bulk/512 win did not justify the extra metadata and cold-miss
work. The source is reverted. A delayed-binding sketch is retained but was
not built or measured; it has no claimed performance result.

The mapped prototype used only the existing `mmap_max_level` option, with
both before and after set to `Some(u32::MAX)`. Defaults and level selection
were preserved. A per-block `OnceLock` cached checksum/layout verification,
and point decoding borrowed mapped bytes while the owning table remained
borrowed. Point calls did not touch the block-cache map. Tests checked actual
values under concurrent reads, one verification per touched block, stable
corruption errors and post-unlink reads. Despite removing cache lookup and
ownership work, the [warm pilot](mapped/bulk-512/report.md) did not beat LMDB
and worsened the paired p50 ratio 1.231 → 1.367. Trial ranges overlap and
controls drift, so a precise regression is not claimed; there is no evidence
to ship it as a warm-read improvement. The [full mapped matrix](MAPPED.md) confirms the tradeoff: bulk/32 point p50/p99 fell 3.840/6.528 → 2.432/3.584 µs and normal/32 fell 4.096/7.296 → 2.688/4.352, but normal/512 rose 2.368/3.904 → 2.688/4.352. Every scenario remained behind its contemporary LMDB control. The source is reverted rather than shipping this configuration-dependent result. Cold-start results are linked separately.

[Verified cold-start probes](COLD.md), 1M normal layout and 32 MiB, show actual
storage I/O and major faults after post-open PTE/file-page eviction. With one
reader, mapped p99/p999 were 622.592/2424.832 µs versus pread
450.560/1441.792 µs. With four readers sharing CPU 3, mapped tails rose to
2949.120/8650.752 µs versus pread 589.824/999.424 µs. LMDB's concurrent
p99/p999 were 3866.624/13893.632 µs. These compare configurations in the
prototype binary, not an old-mmap/new-mmap regression. The existing mapping
opt-in already documents this tradeoff. All 300k sampled values passed the
payload oracle. The initial pre-open-only cache-drop run warmed mapped data
during open and is explicitly excluded; its zero storage/fault counters were
caught by the report guard, not reported as a cold speedup.
