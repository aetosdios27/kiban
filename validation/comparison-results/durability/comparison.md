| Case | Engine | Trials | Median ops/s (min–max) | GET p99 µs | PUT p99 µs |
| --- | --- | ---: | ---: | ---: | ---: |
| write-sync-0 | kiban | 3 | 293,418 (265,883–300,819) |  | 55.30 |
| write-sync-0 | rocksdb | 3 | 991,524 (856,062–1,028,763) |  | 8.19 |
| write-sync-1 | kiban | 3 | 1,748 (1,634–1,793) |  | 16252.93 |
| write-sync-1 | rocksdb | 3 | 4,957 (4,947–5,184) |  | 1605.63 |
| write-sync-1000 | kiban | 3 | 211,191 (191,636–211,332) |  | 71.68 |
| write-sync-1000 | rocksdb | 3 | 397,089 (371,024–483,064) |  | 9.73 |
