# Large-N subset: Kiban and raw LMDB

Three alternating trials per engine and mode; 1,000,000 untimed base records.
Both modes are preliminary because later trial median loads exceeded 3.
All correctness, affinity, durability-flag and reopen checks passed.

Helix was explicitly omitted with `--engines kiban lmdb` after the independent
[memory reproduction](../helix-memory-diagnostic/). This is a complete two-engine
subset, not a completed three-engine campaign. No valid large-N Helix numbers
exist. See [ENGINE_H2H.md](../../../../validation/ENGINE_H2H.md) for the blocked
cells, full tables, per-run loads/RSS/disk and interpretation.

The binary is identical to the small-N campaign. No rebuild occurred. Source
snapshots and full logs/databases remain in the ignored matching target directory;
these compact files retain raw stages, resources, reopen markers and provenance.
