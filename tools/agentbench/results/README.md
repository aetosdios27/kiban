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
