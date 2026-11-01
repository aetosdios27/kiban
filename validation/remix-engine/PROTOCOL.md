# Frozen paired protocol

Baseline engine: main `f3d21dc1f4c40be62b3c6ce1fc4493dc422816ac`, archived
under `target/remix-before-src`. Only its readpath harness adds the same varied
bounds probe; its engine sources match that commit. Candidate dependencies,
payload JSON, parser, allocator, key order, histogram and compiler profile are
unchanged. The final candidate uses the real engine's `prefix_values` API for latest-K
and `range_values` for varied starts, and returns
ten owned decoded `Memory` objects. The baseline uses its existing owned range
API. The latest-K caller formats one prefix on both the final candidate and LMDB;
the engine uses the prepared prefix extent directly. Other finite prefixes
derive an upper bound on the stack. Archived main still
formats its original two bounds. This API/setup change is explicit.
Every read still traverses stored records; no result cache exists.

Builds and tests use CPUs 12–21 at nice 10. Timed processes use CPU 3 only and
never overlap this task's builds, tests, or other timed processes. CPU 4 is CPU
3's sibling. Complete-process `/proc/stat` counters are saved for CPU 4; this
shared host does not provide exclusive ownership of that sibling. No unrelated
process or its affinity is changed.

Each of bulk/normal × 512 MiB/default contains five trials. Odd trials run
before Kiban, its LMDB control, after Kiban, its LMDB control. Even trials
reverse the before/after pairs. Each process runs 100,000 point and 100,000
latest-ten operations, verifies every timed result outside its measured call,
and verifies the full million-record model before and after the stages.
Prepared index construction is outside timing and printed in each process log.
The default is resolved by the actual engine, not approximated as 512 MiB.

The decision statistic is the median of the five per-trial Kiban/LMDB p50
ratios and independently the median of the p99 ratios. Report the ratio of
median latencies and all per-trial ratios too. Latest-K requires paired p50
<=0.85 and p99 <=1.0 in every layout/cache scenario. Point reads retain the
same five pairs; normalized after/before ratios expose any non-regression
uncertainty. No tolerance is silently substituted for the requested target.

Varied bounds use 100,000 unique starts across all 64 agents and their full
population. Bounds formatting, traversal, JSON decode and owned outputs are
inside timing. Both engines return the same ten decoded values, with complete
payload/id checks outside timing. The final matrix uses five alternating before/after pairs per cell. Earlier
rounds kept one pair per cell as a robustness probe.

Fast and durable write runs start from private 10k fixtures. Each of five
alternating before/after pairs has its own LMDB control. W1/W4 contain 1,000
writes each; point/latest-ten stages and completion verification use the same
existing readpath suite. Mutable fixtures are reflinked privately per process.

The owned-range control links the same `range.rs` source (CPU assertion changed
to 3) against each frozen library. It verifies complete keys and payloads for
10- and 256-row general ranges. Five alternating pairs cover each layout/cache.
Final maintenance uses separately built release test binaries for archived main
and the candidate on CPU 3. The archived test changes only its CPU assertion
from 8 to 3, then its source is restored. Five trials alternate process order,
with `KIBAN_POINT_BENCH_N=1000000` and one selected trial per process.
The main comparison selects PH3-on rows on both engines and scan-on rows on
the candidate. Each raw process also retains its off control. Input fill is
outside flush and compaction BUILD timing. Earlier rounds used a candidate-only
scan-off/on control.

The cost probe separates value-only routing, complete JSON decode, a
pre-materialized JSON floor, and zero/one/ten mutable overwrites per agent.
Those are diagnostics, not LMDB adoption numbers. FIFO-gated perf counters and
call stacks exclude startup, model verification and view construction. Raw
commands, binary hashes, source hashes, failures and CPU receipts are retained.

The resumed final runner waits for no visible compiler worker or engine unit-test executable from this task's worktree and a
250 ms CPU-4 sample with zero busy ticks before each measured process. This avoids
starting beside known builds but cannot reserve the host against work that
starts later. Full-process sibling counters remain the evidence for each run.
All earlier rounds remain separate. No failed or partial result is reused as
a successful trial. Source hashes and hashes of all six final binaries are
recorded in `final/`.

The worker check follows the task boundary. Other tasks keep their assigned
cores and are not stopped or moved. This task's builds and tests finish before
the matrix starts. Other host activity is captured in the CPU receipts.
