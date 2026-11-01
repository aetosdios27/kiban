# Attribution ablation

Five alternating trials, CPU 8, 1M records, private copies. Every process verifies all stored values before timing and every timed result afterward. Raw samples return owned values; E2E includes formatting and JSON parsing. No build/test overlapped these runs. Host background benchmarks/crash sweeps remained active.

Sorted and MPHF both retain identical decoded arenas, original SST membership, routing, slots, MPHF and sorted-offset arrays. Only the locator changes; sorted uses binary search over full-key record offsets. THP allowed/disabled is checked with PR_GET_THP_DISABLE after optional PR_SET_THP_DISABLE, before backend allocation. Allowed does not force huge-page allocation. Memory samples record actual AnonHugePages.

| Layout | THP | Variant | Raw p50 ns median [range] | Raw p99 ns | E2E p50 ns | p50 / paired LMDB |
|---|---|---|---:|---:|---:|---:|
| bulk | enabled | lmdb | 1654 [1593, 1825] | 2546 | 2007 | 1.000 |
| bulk | enabled | kiban | 1849 [1821, 2126] | 2804 | 2307 | 1.135 |
| bulk | enabled | sorted | 2851 [2735, 2952] | 3891 | 3248 | 1.717 |
| bulk | enabled | mphf | 465 [451, 590] | 721 | 835 | 0.289 |
| bulk | disabled | lmdb | 1642 [1612, 1681] | 2437 | 2050 | 1.000 |
| bulk | disabled | kiban | 2114 [2090, 2216] | 3205 | 2591 | 1.273 |
| bulk | disabled | sorted | 3203 [3187, 3319] | 4527 | 3635 | 1.959 |
| bulk | disabled | mphf | 532 [507, 559] | 827 | 915 | 0.325 |
| normal | enabled | lmdb | 1638 [1510, 1731] | 2447 | 1990 | 1.000 |
| normal | enabled | kiban | 2182 [2076, 2270] | 3999 | 2658 | 1.355 |
| normal | enabled | sorted | 3435 [3105, 4147] | 5333 | 3782 | 1.984 |
| normal | enabled | mphf | 741 [607, 1042] | 1158 | 1140 | 0.449 |
| normal | disabled | lmdb | 1731 [1511, 1776] | 2667 | 2071 | 1.000 |
| normal | disabled | kiban | 2250 [2071, 2448] | 3879 | 2663 | 1.303 |
| normal | disabled | sorted | 3654 [3520, 4048] | 10164 | 4153 | 2.142 |
| normal | disabled | mphf | 749 [738, 847] | 1176 | 1138 | 0.445 |

## Attribution

A binary-search decoded representation can be slower than the old cached-block search. This control isolates changing the locator within the decoded representation; it does not isolate decoding from cache ownership, memory layout, hash computation, or query routing in the original engine. Do not add the components as independent percentage gains.

| Layout | THP | Sorted minus MPHF p50 ns | Kiban minus sorted p50 ns |
|---|---|---:|---:|
| bulk | enabled | 2386 | -1002 |
| bulk | disabled | 2671 | -1089 |
| normal | enabled | 2694 | -1253 |
| normal | disabled | 2905 | -1404 |

| Layout | Variant | Disabled minus allowed p50 ns | Disabled / allowed |
|---|---|---:|---:|
| bulk | lmdb | -12 | 0.993 |
| bulk | kiban | 265 | 1.143 |
| bulk | sorted | 352 | 1.123 |
| bulk | mphf | 67 | 1.144 |
| normal | lmdb | 93 | 1.057 |
| normal | kiban | 68 | 1.031 |
| normal | sorted | 219 | 1.064 |
| normal | mphf | 8 | 1.011 |

Sampled per-process maximum AnonHugePages (KiB; opportunistic, not synchronized peak RSS):

| THP | Variant | Median [min,max] KiB |
|---|---|---:|
| enabled | lmdb | 100352 [98304, 100352] |
| enabled | kiban | 530432 [462848, 532480] |
| enabled | sorted | 468992 [454656, 548864] |
| enabled | mphf | 456704 [98304, 540672] |
| disabled | lmdb | 4096 [4096, 4096] |
| disabled | kiban | 4096 [4096, 4096] |
| disabled | sorted | 4096 [2048, 4096] |
| disabled | mphf | 4096 [2048, 4096] |

THP-disabled samples retain 2–4 MiB of pre-main anonymous huge pages. PR_SET_THP_DISABLE is applied before backend/query allocations, not before process startup; the large decoded arena is not huge-page backed in those runs. THP is a process-wide ablation, so the observed difference includes all anonymous allocations, not only the MPHF.

The gain is retained with THP disabled. The real engine implementation will preserve full-key verification and MVCC, keep Bloom-first rejection, charge resident arenas against the existing cache budget, and fall back to sorted blocks when a section cannot be admitted. Integrated performance remains to be measured.
