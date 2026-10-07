# Foreground latency and endurance pass

- [x] Study pinned RocksDB write coordination, maintenance scheduling, checksums, read selection and durable version/recovery paths.

- [x] Reproduce baseline/current mixed workloads at 4, 8 and 16 threads with identical parameters and alternating trial order.
- [x] Profile foreground gate waits, read amplification and maintenance BUILD work; separate measurements from hypotheses.
- [ ] Identify and fix supported causes of mixed regression and L0 write stalls, preserving durability and visibility.
- [ ] Add relevant deterministic regressions; run focused checks after each change.
- [ ] Repeat fixed workloads, including read/write/update/burst/concurrency cases; retain failures and regressions.
- [ ] Run 100M operations with reopened-state verification.
- [ ] Run one hour of sustained maintenance pressure and transitions; report degradation by interval.
- [ ] Run a cache-reduced, larger-than-memory workload within measured disk/memory limits; document practical limits.
- [ ] Run formatting, both Clippys, unit/integration/example tests, Loom, ignored release stress and recovery/fault campaigns.
- [ ] Commit coherent stages and publish a reproducible report with actual gains, regressions and unresolved risks.

No historical workload parameters will be changed. New diagnostic and endurance
plans will be separately named. Profiling runs will be labeled and excluded from
primary throughput comparisons. No automatic global cache clearing or host-wide
power changes. Success means repeatable improvements without hiding adverse
results; significant improvements in every workload cannot be assumed in advance.
