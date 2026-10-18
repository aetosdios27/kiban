# First integrated version

The five alternating paired trials passed three of four latest-K cells. Median
paired p50 / p99 Kiban-to-LMDB ratios were:

| Layout/cache | p50 | p99 |
|---|---:|---:|
| bulk/512 | 0.847 | 0.819 |
| bulk/default | 0.847 | 0.697 |
| normal/512 | 0.861 | 0.981 |
| normal/default | 0.847 | 0.885 |

Normal/512 failed the required 0.85 p50 gate. All engine and benchmark gates
passed on that exact source, including both 20,000-seed suites. The source patch,
source hashes, gates, complete paired records and cost probe are preserved in
[round1](round1/). The earlier pilot and interrupted `screen-v1` predate the
short-span memtable regression fix and are excluded from every adoption gate.

The diagnostic value-only routing p50 was 164 ns in both layouts, including
bounds formatting and output allocation. Bulk JSON decode plus traversal was
1,792 ns versus a pre-materialized JSON floor of 1,632 ns. Normal was 1,728 ns
versus 1,600 ns. These are single diagnostic runs, not the primary paired gate.
A single mutable overwrite per agent raised routing p50 to 400 ns; ten raised
it to 496/504 ns. The decoded cases with memtables were 1,984–2,016 ns.

The retained cross-SST stream had already removed the normal layout's runtime
level merge. Two integration costs remained avoidable: the direct loop repeated
PH3 width dispatch per value, and it reloaded the first locator from the global
array despite keeping that locator in the prefix directory. The second version
dispatches once per span and uses the cached first locator. Its current-sequence
span loop skips sequence arithmetic after choosing the first version; snapshots
still choose a visible version. It also retains a single scan-header offset
instead of copying the header configuration into the point handle, and sizes
the serializer output including the extension before allocation. The persisted
format is unchanged between the two versions.
