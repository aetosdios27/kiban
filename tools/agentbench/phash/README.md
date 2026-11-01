# Immutable-SST perfect-hash experiment

This standalone crate depends on the engine public API. It never changes `src/`.
It imports the existing readbench model and PRNG instead of redefining the workload.

Build separately from measurements:

```sh
taskset -c 6,7,10,11 nice -n 10 cargo build --locked --release --manifest-path tools/agentbench/phash/Cargo.toml
```

Copy the prepared Kiban and LMDB directories to `target/phash-data/` first. Use real copies
or copy-on-write reflinks, never hard links. The runner opens LMDB in read-only mode against a private copy, with its own
reader lock file and a 10 GiB map, matching the readpath transaction setup. Its native LMDB dependency is
locked to version 0.2.5 for compatibility with that dataset.

```sh
taskset -c 6,7,10,11 python3 tools/agentbench/phash/run.py
```

The runner pins every timed child, including the floor, to CPU 8, sets
`RAYON_NUM_THREADS=1`, and never invokes a compiler. It requires the existing
`target/read-path-tools/root/usr/bin/perf` in the source data worktree. Five trials
alternate `LMDB, Kiban, LMDB, MPHF, LMDB`, reversing the sequence in even trials.
Each layout runs positive and negative workloads separately. LMDB appears three
times per trial to bracket both candidates. Summaries first median those three
controls within each trial, then median the five trials.

`floor` follows one random cycle through 8,388,608 cache-line-sized nodes. Every
load depends on the previous load, including across chain boundaries, so there
is no overlap between independent chains. Five trials reverse chain lengths.
This measures dependent cache-line latency over 512 MiB, including the host's
current memory contention. It does not guarantee every node misses the LLC.

`mphf` preserves actual SST membership, level, key order, and original block
boundaries. Each block is CRC-verified and decoded into contiguous records:
`u16 key_len | u16 value_len | full_key | value`. Slots contain a u32 decoded block
offset, u32 offset within that block, u16 fingerprint, and two padding bytes.
The full byte key is hashed with XXH3-128; its low 64 bits feed PtrHash's default
integer hasher and its high bits supply the fingerprint. Build rejects duplicate
64-bit hashes. Lookup checks the complete stored key after the fingerprint.
Higher levels use binary range routing; L0 probes newest files first. There is
no global map, memtable, snapshot history, or tombstone handling in this prototype.

Each backend returns an owned copy. LMDB opens and commits a read transaction per
operation. Raw samples use precomputed keys; end-to-end samples include the
original key formatting and JSON decoding. Both exclude result destruction,
matching the readpath timer boundary. A separate one-million-operation loop
includes destruction and provides throughput and gated perf counters. It uses
one million independently generated workload keys rather than repeating a small
hot set. Verification checks all one million stored values before timing and
all million timed results after timing. Negative keys use IDs 1M through 2M with
the same formatting and random sequence. Reversed IDs make these negatives cluster into 64 key-space gaps; this is an added
missing-ID workload, not uniformly scattered interior misses. The `errors: 0` output is emitted only
after these checks pass.

The packed decoded records and metadata fit within 512 MiB. This is a warm
memory experiment, not a cache implementation, compaction implementation, or
production-ready file format. Read `validation/phash/REPORT.md` for limitations.

After the suite, an optional second `taskset -c 8 .../kiban-phash floor 200000`
run can be saved as `results/floor-end.jsonl`. Run analysis on driver CPUs:

```sh
taskset -c 6,7,10,11 python3 tools/agentbench/phash/analyze.py validation/phash/results
```

Analysis checks all 100 subprocess outputs, control counts, cross-variant digests,
ten builds per layout, counter coverage, and unchanged Kiban files. It emits the
brief's bulk-hit cutoff as a machine-readable gate. This numeric gate does not
assert an idle SMT sibling or isolate hashing from the decoded-block change.
