# Comparative validation

The first comparative phase measures Kiban **v0.1.0** against RocksDB **11.8.1**.
This answers a different question from the release gate: how throughput, tails,
resource use and maintenance behavior compare under a specified workload.
Testing evidence is not a maturity or production-readiness claim.

The isolated tool, fixed JSON plans, methodology, metric definitions, durability
modes and reproduction commands are in [tools/compare](../tools/compare/README.md).
The tool pins Kiban's release commit as a Git dependency and does not change the
engine or move the v0.1.0 tag. RocksDB is a dependency of the unpublished comparison
tool only. Generated results and the tool are excluded from the Kiban crate package.

The recorded report distinguishes completed measurements, preparation-only plans,
observations and hypotheses. Comparisons use fresh data, shared deterministic
traces, serial randomized-order repetitions and explicit matched/default profiles.
No system-wide cache clearing, power changes, automatic retries or workload
parameter reductions are performed.
