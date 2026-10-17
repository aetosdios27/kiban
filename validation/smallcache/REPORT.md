# Small-cache point reads

Investigation in progress. The five-pair bulk/32 MiB/index-on pilot reduced
point p50/p99 from 3.712/5.504 to 1.536/2.176 microseconds. The candidate
LMDB control was 1.344/1.824. These are pilot results, not the final matrix.

The initial missing-SST error regression is fixed at its source. Parent
clarification confirmed that caught, fixable candidate regressions do not
trigger the correctness stop rule. The decisions record and development logs
preserve that sequence.

Full gates and the frozen final paired warm/cold/write matrix are pending.
No final performance or correctness verdict is claimed yet.
