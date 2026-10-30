# Website benchmark refresh

This campaign supplies the numbers on the website's benchmark section. It runs
one commit against tuned RocksDB, SQLite and LMDB, so the rows can share a board.
The earlier reports used different commits and stay unchanged.

- `campaign.py` runs the four-engine reads and durable writes.
- `group_commit.py` repeats `validation/group-commit/run_matrix.py`.
- `scale/` holds `tools/compare/plans/scale.json`, repinned to this commit.
- `REPORT.md` records the results, deviations and limits.
