# Agent-memory measurement attempts

| Directory | Status | Use |
| --- | --- | --- |
| [final-10000-clean](final-10000-clean/) | Preliminary; 1-minute measured load 1.40–5.55 | Latest complete five-pair attempt; primary report in [validation/AGENT_MEMORY.md](../../../validation/AGENT_MEMORY.md) |
| [final-10000](final-10000/) | Preliminary; measured peak 5.93 | Earlier attempt retained unchanged as separate raw evidence |

Neither directory contains a qualifying clean campaign. Names record the intended
attempt, while `status.json` and `REPORT.md` record its actual classification.
Do not pool these attempts or infer an engine-level winner from embedded versus
HTTP latency. Compact evidence is tracked; databases, binaries, source snapshots
and full logs remain under the matching ignored `target/agentbench-results/` path.
Each directory has a SHA-256 manifest for all other archived files.

## In-process engine study

[engine-small-10000](engine-small-10000/) contains the engine head-to-head:
durable qualifies under the median-load gate; fast is preliminary. Both use
five alternating trials, an untimed 10,000-record base, and 10,000 measured
inserts (20,000 final records). [engine-sync-diagnostic](engine-sync-diagnostic/)
is excluded instrumented evidence, not a latency comparison.

[engine-large-1000000-kv](engine-large-1000000-kv/) contains three alternating
Kiban/LMDB trials per mode at a 1,000,000-record base. Both modes are preliminary:
later trial median loads exceeded 3. All correctness and reopen checks passed.
[helix-memory-diagnostic](helix-memory-diagnostic/) retains excluded calibration
and independent HTTP evidence for Helix's large-N memory block, including the
one-field BM25 config delta, exact generated query, server hash and full RSS
curve. No diagnostic latency is pooled into either engine campaign.
