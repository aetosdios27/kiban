# redb key order A/B

The descending comparator reverses natural byte order and uses a reverse range to return the latest ten. The plain candidate uses redb's built-in byte-slice comparator and a forward range. Both return the same records because keys already encode the creation ID in reverse order. The comparison measures the combined effect of traversal direction, comparator, and resulting B-tree layout.

Prepare and oracle-check both comparators' bulk and normal fixtures with the original seed and loader. Use the screened Maximum layout for bulk reads and write preload, leave normal fixtures as loaded, and keep quick repair off. Run five paired, alternating rounds for each of the four read scenarios and both write modes. Use the same release binary for both candidates, pin database processes to CPU 8, and record CPU 9 activity and host load. Each read command verifies all 1M records before and after timing. Each write command starts from a fresh compacted 10k fixture, verifies the initial and final populations and every call, and reopens and verifies the final model.

Use the observed five-trial ranges to gauge noise. Count a direction as measurable only when the two ranges are disjoint and the relative median difference exceeds one histogram bucket (3.125%). Apply this rule to p50 and p99 for each operation in every scenario. Adopt the plain comparator only if it improves at least one cell and regresses none. This rule is descriptive and does not estimate a confidence level from five samples.

```bash
driver=tools/agentbench/readpath/redb_bench.py
taskset -c 12-21 nice -n 10 python3 "$driver" prepare-keyorder
taskset -c 12-21 nice -n 10 python3 "$driver" measure-keyorder
```

Audit and render the results with `python3 tools/agentbench/readpath/redb_report.py validation/redb/keyorder-ab --keyorder`. The audit checks recorded hashes, configuration, CPU affinity, stage counts, and full-population verification receipts.

For the original primary, use `redb_report.py --source-revision 5a8dcce4` to check protocol and source hashes against the original revision after rebuilding the local executable.
