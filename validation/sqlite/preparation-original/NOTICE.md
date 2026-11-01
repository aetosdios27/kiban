These are setup receipts before tuning SQLite fixture loading, not latency trials.
The clustered layout was inserted in ID order, causing excess page slack.
Final preparation loads SQLite in primary-key order, builds the rowid secondary
index after preload, and runs ANALYZE. All final trials use the rebuilt binary
and the fixtures recorded under ../raw/prepare. No rows from this directory
enter a latency table or layout decision.
