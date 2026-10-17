# Approximate attribution of cache-hit point-read cycles

Exclusive self-symbol percentages from the complete sample reports are multiplied by cycles/op from the separate stat runs. These are attribution estimates, not independently additive microbenchmark timings. Search includes LMDB B-tree work and Kiban table routing, inlined index/filter work and byte comparisons. Missing parent chains do not affect sampled instruction pointers.

| Category | Bulk K cycles/op | Bulk LMDB | Bulk extra | Normal K | Normal LMDB | Normal extra |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Kiban block-cache lookup/ownership | 1260 | 0 | 1260 | 1203 | 0 | 1203 |
| Kiban in-block entry decoding | 1054 | 0 | 1054 | 945 | 0 | 945 |
| Search, routing and byte comparison | 4674 | 3756 | 919 | 5014 | 3950 | 1065 |
| Heap allocation/free | 224 | 197 | 27 | 195 | 216 | -22 |
| JSON and text decoding | 706 | 791 | -85 | 764 | 799 | -36 |
| Other work | 588 | 710 | -123 | 1103 | 836 | 267 |
