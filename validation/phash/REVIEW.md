Reviewed by Claude Opus 5.5. Read-only review; no further agents were spawned after the parent instruction.

Review is done. I ran no benchmarks, builds or tests, and changed no files. Every shell command used `taskset -c 6,7,10,11`.

## Verified OK
- **Binary matches the recorded run.** Its sha256 `d018…2ed2a` equals `results/metadata.json`, and it was built at 23:49:32, after the last `main.rs` edit at 23:49:13. `rejected-setup` used a different binary (`ad4ff…`), as expected.
- **Workload matches readpath stage 1:** `mix(SEED ^ mix(1) ^ op) % N`, with the same model key and value. The new e2e LMDB p50 is about 1.95 µs, against readpath's 1.86 µs at 1M bulk. Kiban at 512 MiB is about 2.1–2.45 µs, against 2.24–2.30 µs. That is consistent.
- **CPU isolation:** CPU 8 is a P-core (`cpu_core` is 0–11) and its sibling is 9. The process shows `Cpus_allowed_list: 8`, 1 thread, and all counters at 100% enabled with no multiplexing.
- **Correctness:** all 1M hit values are checked before timing and all 1M timed results after. MPHF build checks one-to-one slot assignment and non-overlap. The L0 newest-first order is correct: the manifest is sorted ascending by (level, number) and the code iterates it in reverse. The LMDB fix (commit the DBI-open transaction) is the real root cause. LMDB closes a DBI handle opened in a transaction that is later aborted.

## Flags (most severe first)

**HIGH 1 — The bulk layout is one SST, so it doesn't test a per-SST MPHF.** Every bulk build row shows `"layout":{"0":1}`. The kill-criterion layout is therefore a single global MPHF, with no routing and no multi-file probing. Only the normal layout tests the per-SST design, and it hasn't run yet. Write this in the trail, and also evaluate the kill criterion on the normal layout.

**HIGH 2 — The speedup can't be credited to the MPHF alone (documented idealization, but no control).** The MPHF variant also gets things production Kiban doesn't have:
- a fully decoded heap arena, with no block-cache lookup and no restart or prefix decoding;
- transparent huge pages: Kiban's process shows `AnonHugePages: 294912 kB`, and MPHF dTLB misses are 0.01/op against LMDB's 2.0/op.

The counters show the gap: 473 instructions/op for MPHF against 3398 for Kiban. A GO supports "decoded arena + MPHF", not MPHF by itself. Either add a control (the same arena with sorted or fence lookup) or state the combined attribution in the decision row. Space so far: arena 338.7 MB against 329.8 MB of SST (+2.7%), plus 12 B/key of slots and 2.99 bits/key of MPHF.

**MEDIUM 3 — The miss workload clusters into 64 key-space gaps.** `key(id)` uses `u64::MAX - id`, so every miss ID ≥ 1M sorts just before its agent's hit keys. All 1M misses land in 64 gaps, so ordered indexes resolve them in a few hot leaves:
- LMDB: 0.01 LLC misses/op
- Kiban: 0.03 LLC misses/op
- MPHF: 0.88 LLC misses/op

In the partial summary Kiban beats MPHF on misses (raw ratio 0.279 against 0.435). This is an artifact of the workload, not evidence about random negative lookups. `REPORT.context.md`'s line "some SST range checks can reject them" understates it. Readpath has no miss workload, so this one is new and should be labelled that way.

**MEDIUM 4 — The kill criterion isn't in the trail or the code.** `decisions.tsv` has no row stating the metric: raw or e2e, hit only or both, which layouts, and whether the median or the worst trial must be ≤ 0.8. `analyze.py` computes no GO/NO-GO. It also divides normal-layout MPHF by normal-layout LMDB, while the brief says "LMDB bulk". The partial results are about 0.31× raw and 0.42× e2e, so adding the rule now is post-hoc: cite the brief as its source. I recommend requiring both raw and e2e hit max-trial ratios ≤ 0.8.

**MEDIUM 5 — Host contention is real and should be quoted.**
- Load average was 21–25 at start (`metadata.json`), and the shared 24 MiB L3 and memory bandwidth are under load.
- LMDB controls in r1 range from 1582 to 1946 ns raw p50, a 23% spread. The first-position LMDB run is slowest in r1 and r2.
- r4 LMDB batch was 2317 ns against about 1650 normally; the r3 MPHF miss batch was 176 ns against about 105.

Paired ratios absorb most of this. Absolute ns, the miss LLC behaviour (MPHF's 12 MB slot array competes for the shared L3) and the floor do not. `analyze.py` doesn't gate on load or frequency.

**MEDIUM 6 — The floor isn't bracketed and probably runs on huge pages.**
- It runs once, at the start (161 ns at length 1; first trial 168). Normal-layout variants run much later.
- It is a 512 MiB anonymous heap allocation on a host with THP enabled (`THP_enabled: 1`), so it likely measures DRAM latency without page-walk cost. LMDB pays that cost on 4 KiB file-backed mappings (2 dTLB misses/op).

So "serial-load equivalents" overstate LMDB's load count compared with MPHF and Kiban. Add this caveat to the TABLES text.

**LOW 7 — Stale partial tables.** `validation/phash/TABLES.md` and `summary.json` (23:52:50) come from a `--partial` run with 4 trials, but the header text still says "across five trials". Regenerate them before citing. Also:
- `analyze.py` doesn't assert 10 builds per layout; the current summary shows `builds: 8`.
- It doesn't check `*-kiban-checksums.json` `unchanged` or cross-variant digest equality.
- With `--partial`, a missing LMDB trial crashes it with `StatisticsError` from `median([])`.

**LOW 8 — Trail gaps on provenance.**
- The smoke run and the rejected setup opened the original `t3-53ad0f47/target/read-path-data` directly (`smoke-progress.log`), before the private-copy decision. The opens were read-only and later content is fully verified, but the trail doesn't record this.
- Copy method (reflink or copy, not hard link) and copy-to-source checksums aren't recorded.
- Rows 4–5 applied two changes together (private copy + normal locking, and the commit). Row 5 asserts the root cause without an isolated rerun. Cite the LMDB `mdb_dbi_open` behaviour on abort instead.

**LOW 9 — e2e LMDB does one more copy than readpath.** Readpath parses JSON straight from the mapped slice inside the transaction. Here LMDB copies to a `Vec` first. That adds one ~340 B alloc+memcpy to LMDB e2e, a slight bias toward MPHF on e2e only. Raw is uniform across backends.

**LOW 10 — Unchecked parts of the prototype (not correctness bugs).**
- SST index entries are hand-parsed; the footer and index CRC aren't checked if the format has one. End-to-end value checks cover correctness.
- `"threads":1` is hard-coded rather than measured.
- Counters are user-mode only (`/u`), so kernel work isn't counted.

I found no code flaws that invalidate the bulk hit measurement. The material risks are in interpretation: flags 1–4.
