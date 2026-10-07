# Foreground performance and endurance validation

The todo list is `FOREGROUND_TODO.md`. The pinned RocksDB source study is
`ROCKSDB_STUDY.md`. This pass changes measured CPU work and rejects malformed
sizes; it does not change storage bytes, WAL retirement, snapshot retention,
compaction drop rules, or durability settings.

## Reproduce measurements

`foreground_freeze.py` requires committed compiled inputs, builds release
executables, and retains source/binary hashes and explicit compiler flags.
`foreground_pair.py` checks those receipts against Git and alternates build order
within randomly ordered case/repetition pairs. It stops on failures, timeouts or
wall/monotonic clock gaps. All measured trials use fresh databases unless a plan
explicitly names a shared prepared dataset and requests reuse+verification.
It reopens and verifies the full expected key set outside foreground timing.

```sh
python3 validation/foreground_freeze.py --output target/foreground-binaries/BUILD --debug
python3 validation/foreground_pair.py \
  --pair current=target/foreground-binaries/current \
  --pair candidate=target/foreground-binaries/BUILD \
  --plan validation/foreground-primary-plan.json \
  --output validation/foreground-results/CAMPAIGN \
  --data target/foreground-data/CAMPAIGN
python3 validation/foreground_analyze.py validation/foreground-results/CAMPAIGN
```

Diagnostic builds additionally use `--diagnostics` when freezing and
`--diagnostic` when running. Clocks sample one in 1,024 acquisitions per thread
and lock kind. `acquisitions.csv` is process-wide cumulative data; subtract the
initial row before describing foreground samples. Maxima are since process start,
not interval maxima. Timings overlap across threads and do not measure lock hold
or CPU time. Ordinary builds contain neither sampling nor clocks. Diagnostic
rates must not be substituted for primary rates.

The CPU profile uses a locally extracted Fedora perf package, with no root/global
configuration changes:

```sh
python3 validation/foreground_pair.py \
  --pair current=target/foreground-binaries/profile \
  --plan validation/foreground-profile-plan.json \
  --output validation/foreground-results/profile \
  --data target/foreground-data/profile \
  --perf target/profiler/usr/bin/perf
```

The runner records 99 Hz userspace CPU-clock samples with DWARF call chains.
Reports and assembly annotations are retained; large raw `.perf` files stay local
with their SHA-256/size receipts. Profiles include preparation and verification
inside the scale child, but the separate reopened-state verifier is not profiled.
CPU percentages describe sampled userspace execution, not elapsed time or kernel
I/O waits. The short mixed profile has fewer samples than the long write profile.

## Endurance and larger-than-memory validation

The separately labeled endurance plan includes 100M random writes, a one-hour
steady/write-burst/read-recovery transition (20 minutes per phase), and a ten-minute
1-to-16-thread concurrency transition. It retains the canonical 4 MiB buffers,
32 MiB cache and L0 ceiling of eight. The timed trials preload the whole key
space: every allowed write preserves the deterministic value for its key, making
full post-run key/value verification exact despite timing-dependent operation IDs.
The torture campaign separately checks differing versions, deletes and durable
acknowledgements.

```sh
python3 validation/foreground_pair.py \
  --pair candidate=target/foreground-binaries/FINAL \
  --plan validation/foreground-endurance-plan.json \
  --output validation/foreground-results/endurance \
  --data target/foreground-data/endurance
python3 validation/foreground_analyze.py validation/foreground-results/endurance
python3 validation/foreground_windows.py validation/foreground-results/endurance
```

`foreground_windows.py` reports stage-specific minute windows and first/last-third
trends. It preserves latency as median/worst sampled-interval p99, never labels
these pooled quantiles. Throughput, interval SST amplification, L0, debt, disk,
RSS and descriptors make degradation visible; a fast beginning cannot conceal a
slow end.

The large-memory plan writes nine million 4,096-byte **keyed** values (36.9 GB
payload). Historical constant values compress strongly on this Btrfs/zstd host,
so logical length alone is inadequate evidence. Keyed values are deterministic
per seed/key and preserve order-independent expected state. This new parameter
is not applied to any historical or primary comparison workload. Value generation
occurs outside individual API latency timing but remains in total throughput.
Preparation records both logical and allocated file bytes; compare allocated
size with recorded physical RAM, and use process read bytes to detect real I/O.

The first case prepares and prewalks the entire dataset. That cannot make a
larger-than-RAM working set fully resident. The second verifies the same prepared
dataset, closes it, applies existing file-scoped advisory cache reduction and
reopens. No machine-global cache dropping occurs. Report these as prewalked and
cache-reduced, not guaranteed hot/cold. A disk-space preflight requires 100 GiB
free before preparation and 32 GiB before the reused read run. The large plan
explicitly uses 32 MiB memtables/SST targets and a 256 MiB base budget; it is not
silently compared with the small-workload settings.

```sh
python3 validation/foreground_pair.py \
  --pair candidate=target/foreground-binaries/FINAL \
  --plan validation/foreground-large-plan.json \
  --output validation/foreground-results/large \
  --data target/foreground-data/large
python3 validation/foreground_analyze.py validation/foreground-results/large
```

A complete status record and successful full reopened verification are required.
Prepared commands or partial runs are not measured successes. Generated evidence
is excluded from crate packaging. No automatic retries or workload resizing.
