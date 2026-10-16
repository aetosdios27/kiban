# Site facts (schema: 1)

Edit `site/claims.json` in a reviewed PR. Keep clauses within the limits in
`README.md` and `ARCHITECTURE.md`; add real test function names as evidence.
Run `python3 tools/site_facts.py validate` and
`python3 -m unittest discover -s tools/tests` before committing.
Mark examples with `// site:begin <id>` and `// site:end` in Rust files that CI
compiles or tests. Python uses `#` markers in binding tests when available.
Regions are unique, non-nested, closed and extracted verbatim with common
indentation removed. The five Rust tabs are `basic`, `batch`, `range`, `snapshot`
and `shared`; `python` comes from the binding's tested code.

CI validates the schema, evidence function references, example markers and the
source patterns used for computed counts. It cannot prove that prose follows
from a test or automatically detect a newly implemented roadmap feature. Review
claims when behavior changes. Specs are checked against Cargo metadata and the
Linux target; optional Loom test support is not a default runtime dependency.

## Generate and consume

Use Python 3.11+ (stdlib only):

```sh
python3 tools/site_facts.py generate --out /tmp/facts.json
cargo test --lib -- --list > /tmp/lib-tests.txt
python3 tools/site_facts.py generate --out /tmp/facts.json --test-list /tmp/lib-tests.txt
```

`--test-list` must contain only the default-feature library test binary's list;
otherwise the default counts literal `#[test]` functions in `src/`, including
ignored and feature-gated tests. Neither count means tests passed.

The publisher runs on main pushes and successful main nightly completions.
It reads the previous `site-data:facts.json`, serializes publication, and writes
only that branch. The branch starts with an orphan commit. Identical content
ignoring `generated_at` produces no commit or deploy hook. The optional
`VERCEL_DEPLOY_HOOK` secret triggers revalidation after a successful push.
The site can fetch
`https://raw.githubusercontent.com/aetosdios27/kiban/site-data/facts.json` at
build/revalidate time. Reject unknown schema versions; show the engine revision
and benchmark source beside claims. No website-repo changes are included here.

Nightly shards upload success receipts for both direct and shared suites.
Only a fully successful nightly run is counted. `--previous FILE` carries totals
forward; `--add-nightly-seeds N --nightly-run-id ID` credits a run once, even on
retry. Counts are seed executions across both engine modes, not unique numeric
seeds, crashes, process-torture cycles, or the local sweep. History starts at zero
when the feed is introduced. Missing legacy receipts are skipped without credit.
A failed fetch of an existing feed fails publication rather than resetting it.
The publisher uses current main for facts even when an older nightly finishes;
receipts retain the nightly run ID used for accounting.

## Fields

`claims.json` has exactly these fields, copied unchanged into the feed:

- `specs`: nonempty array of unique spec strings. Rust edition, default runtime
  dependencies, Linux target and license must match repository metadata.
- `guarantees`: object with `promised`, `not_promised`, `not_built` arrays.
  Each entry has a unique short `key`, a one-sentence `clause`, and optional
  `evidence`: nonempty array of unique Rust function names in `src/` or `tests/`.
  `promised` entries require evidence. References are not a semantic proof.
- `bugs_found`: array of `{date, title, pr}`; `date` is ISO `YYYY-MM-DD`, `title`
  is text and `pr` is a positive GitHub PR number. Dates are fix/merge dates;
  discovery can precede the merge.
- `capabilities`: array of `{id, label, status, link?}`. Unique `id` is a slug;
  `status` is `shipped`, `in_review` or `planned`; optional `link` is an HTTPS URL.
- `roadmap`: array of `{id, title, status}`. Unique slug `id`; `status` is `done`,
  `active`, `next` or `planned`. Titles describe features, not strategy.
- `local_sweeps`: array of `{date, commit, suite, seeds, result}`. ISO date,
  full 40-character Git commit, suite name, positive integer seed executions,
  and textual outcome. Empty until reviewed local evidence is added.

Generated `facts.json` adds:

- `schema`: integer `1` (the version of this entire document).
- `generated_at`: UTC ISO timestamp of generation; preserved on a no-op publish.
- `engine`: `{version, commit, commit_date}` from Cargo package version and Git
  HEAD's full SHA and committer date (ISO timestamp with timezone).
- `tests.lib_tests`: number of literal library test functions, or test entries
  from `--test-list`; excludes benchmarks and integration tests.
- `tests.crash_seeds_per_pr`: sum of the direct/shared fixed-corpus seed counts
  from `src/simulation_tests.rs`, once per corpus (not per CI profile).
- `tests.crashes_per_seed`: simulated losses per seeded suite execution,
  including its WAL-creation probe, computed from the suite's loops/calls.
- `tests.nightly_seeds_total`: cumulative successful nightly seed executions
  since feed publication began; local sweeps and PR corpora do not contribute.
- `tests.nightly_run_ids`: workflow run IDs already credited, as unique strings;
  retained to make retries idempotent. `--add-nightly-seeds` requires a run ID.
- `examples`: object mapping unique region IDs to dedented source strings,
  including internal comments and newlines, without marker lines.
- `benchmarks.source`: report path `validation/COMPARISON.md`.
- `benchmarks.data_source`: machine-readable summary path, when supported.
- `benchmarks.commit`: measured Kiban engine SHA from each campaign's metadata,
  never the current feed revision.
- `benchmarks.rows`: summary array, when machine-readable. Each row has
  `campaign` (result directory), `case`, `engine`, `trials`, `ops_per_second`,
  `read_p99_us`, and `write_p99_us`. Each metric is `{median, min, max}` or null.
  Numbers describe the report's workload/environment and verified historical
  trials; they are not measurements of current main or pooled percentiles.
  If the summary or measured revision is unavailable, only `source` is emitted
  and generation reports the fallback on stderr.

All fields above are required unless marked optional or described as conditional.
Unknown claims fields, invalid types/enums/dates, duplicate IDs/keys, missing
references, and malformed source patterns fail validation. Claim wording and
roadmap/status correctness still require human review.
