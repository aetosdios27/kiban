# Superseded libmdbx campaign

These results use the libmdbx 0.9.0 Rust wrapper, which dispatched each write transaction's begin and commit to a helper thread on the same benchmark CPU and reopened the default-table handle in every transaction. The rerun uses the same native library with caller-thread transactions and an environment-scoped table handle. This directory preserves the original report, measurements, screens, checks, and protocol as recorded at commit `3b56c789`. [Revised protocol](../PROTOCOL.md)
