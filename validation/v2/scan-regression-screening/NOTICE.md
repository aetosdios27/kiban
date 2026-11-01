The first integrated matrix is rejected as final performance evidence: the
normal/512 MiB latest-K paired p50 ratio regressed 1.407 -> 1.667, with disjoint
trial ranges. The point path wins at 512 MiB but this violates the scan gate.
The matrix was stopped during normal/32 MiB. All completed timed operations
passed payload verification. No data correctness failure was observed.

Profiling below investigates the source of the scan regression before a fix.
