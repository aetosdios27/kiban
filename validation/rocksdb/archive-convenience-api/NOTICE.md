# Superseded point convenience API campaign

The initial adapter used `DB::get_pinned`, whose rust-rocksdb 0.25.0 implementation constructs and frees default ReadOptions on every point call. The final adapter stores reusable ReadOptions and calls `get_pinned_opt`. This preserves current-read semantics with no fixed snapshot.

All 270 original screening commands and 61 accepted primary commands remain here. A primary command was interrupted at the coordinator during the tuning correction, so an unreceipted result can also be present. This archive is excluded from the final latency summaries and final command audit. No correctness failure triggered the restart.

The original timing sources are under `sources/`, identified by raw/metadata.json hashes and commit 33a8491. The old binary is retained locally at `target/rocksdb/convenience-api-bin`; its SHA256 is in raw/metadata.json. Original fixtures are retained at `target/rocksdb/data-convenience-api`. Config files preserve their historical paths. The final campaign uses fresh fixtures, fresh screens, and independent primary trials.
