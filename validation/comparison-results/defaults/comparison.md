| Case | Engine | Trials | Median ops/s (min–max) | GET p99 µs | PUT p99 µs |
| --- | --- | ---: | ---: | ---: | ---: |
| defaults-mixed | kiban | 3 | 548,892 (511,033–591,662) | 23.55 | 35.84 |
| defaults-mixed | rocksdb | 3 | 977,136 (976,914–979,412) | 4.99 | 12.03 |
| defaults-random-read | kiban | 3 | 3,634,778 (3,304,233–3,654,946) | 2.02 |  |
| defaults-random-read | rocksdb | 3 | 4,277,753 (3,675,403–4,498,227) | 1.92 |  |
| defaults-random-write | kiban | 3 | 308,968 (308,301–310,374) |  | 51.20 |
| defaults-random-write | rocksdb | 3 | 533,566 (533,457–540,707) |  | 12.54 |
