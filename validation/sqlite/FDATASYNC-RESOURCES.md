# Fdatasync resources

## write-10000-bulk-None-durable

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 15.547 [15.348, 15.617] | 3.742 | 3.754 |
| kiban-index1 | 19.324 [15.789, 19.418] | 4.658 | 4.668 |
| lmdb | 14.355 [14.176, 14.402] | 8.633 | 8.625 |
| sqlite-full | 10.375 [10.191, 10.445] | 3.477 | 3.477 |
| sqlite-full-fdatasync | 10.379 [8.773, 10.387] | 3.477 | 3.477 |
