#!/usr/bin/env python3
"""Assemble the final report only from a complete, validated analysis."""
import json, statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'validation/phash'
x=json.loads((OUT/'summary.json').read_text())
assert x['gate'] and all(r['trials']==5 for r in x['variants'])
assert x['floor_end'], 'run the closing floor before finalizing'
rows={(r['layout'],r['workload'],r['variant']):r for r in x['variants']}
bulk=rows['bulk','hit','mphf'];lmdb=rows['bulk','hit','lmdb'];normal=rows['normal','hit','mphf'];nlmdb=rows['normal','hit','lmdb']
commands=[json.loads(l) for l in (OUT/'results/commands.jsonl').read_text().splitlines()]
assert len(commands)==100

def cpu(s,n):
    v=list(map(int,next(l for l in s.splitlines() if l.startswith(f'cpu{n} ')).split()[1:9]));return sum(v),sum(v)-v[3]-v[4]
sibling=[]
for r in commands:
    a,b=cpu(r['started']['cpu_stat'],9),cpu(r['ended']['cpu_stat'],9)
    sibling.append(100*(b[1]-a[1])/(b[0]-a[0]))
meta=json.loads((OUT/'results/metadata.json').read_text())
assert 'end' in meta
cost=[]
for layout,b in x['overhead'].items():
    arena=b['arena_bytes']['median'];index=1000000*(b['slot_bytes_per_key']['median']+b['mphf_bits_per_key']['median']/8)
    cost.append(f"{layout}: {(arena+index)/2**20:.2f} MiB decoded data plus locator; {(arena+index-b['original_sst_bytes']['median'])/1000000:.2f} extra bytes/key net versus the source SSTs. MPHF construction {b['mphf_build_ns']['median']/1e6:.1f} ms/1M; total tool construction {b['build_ns']['median']/1e6:.1f} ms/1M.")
report=f'''# Immutable-SST MPHF experiment

**{x['gate']['verdict']} for a V2 prototype.** The brief's numeric bulk hit cutoff passes. Raw MPHF median p50 is {bulk['raw']['p50_ns']['median']:.0f} ns versus LMDB {lmdb['raw']['p50_ns']['median']:.0f} ns, or {x['gate']['ratio']:.3f}x. The worst paired raw trial is {x['gate']['worst_paired_raw']:.3f}x and the worst paired end-to-end trial is {x['gate']['worst_paired_e2e']:.3f}x, both below 0.8. End-to-end median p50, including key formatting and JSON decoding, is {bulk['e2e']['p50_ns']['median']:.0f} ns versus {lmdb['e2e']['p50_ns']['median']:.0f} ns. This supports the combined decoded-block + MPHF path; it does not isolate hashing's contribution or promise an integrated-engine result.

The strict idle-sibling requirement was not achieved by the host. Every benchmark process is pinned to CPU 8 and no experiment/build work is assigned to CPU 9, but unrelated browser and Btrfs kernel threads run on that sibling. Across process intervals, CPU9 busy time is median {statistics.median(sibling):.1f}% [min {min(sibling):.1f}%, max {max(sibling):.1f}%]. Evidence is in [sibling-activity.json](sibling-activity.json) and [per-run CPU accounting](results/commands.jsonl). No foreign affinity, priority, or process was changed. Start load averages were {meta['start']['load']}; end loads were {meta['end']['load']}. Thus GO is a prototype decision from contended-host, interleaved measurements, not proof under a quiet sibling or proof that dependent loads alone explain the gain. A quiet-sibling confirmation remains outstanding.

Bulk contains exactly one L0 SST, making table routing degenerate. The normal layout contains one L1, six L2 and 77 L3 SSTs. It preserves actual file membership and block boundaries and probes levels in order. Normal raw MPHF p50 is {normal['raw']['p50_ns']['median']:.0f} ns against its LMDB control {nlmdb['raw']['p50_ns']['median']:.0f} ns ({normal['raw']['p50_ns']['median']/nlmdb['raw']['p50_ns']['median']:.3f}x); relative to the bulk LMDB median it is {normal['raw']['p50_ns']['median']/lmdb['raw']['p50_ns']['median']:.3f}x. The normal end-to-end medians are {normal['e2e']['p50_ns']['median']:.0f} and {nlmdb['e2e']['p50_ns']['median']:.0f} ns. The original kill criterion remains bulk; normal is a separate multi-level result.

The start floor's one-load median is {x['floor']['1']['median']:.1f} ns, and the closing floor is {x['floor_end']['1']['median']:.1f} ns. Bulk MPHF raw p50 is equivalent in elapsed time to {bulk['raw']['p50_ns']['median']/x['floor']['1']['median']:.2f} start-floor loads; normal is {normal['raw']['p50_ns']['median']/x['floor']['1']['median']:.2f}. A pilot, slot and record give the proposed dependency chain, with occasional remap access, but this is not a counted hardware dependency graph. Bulk MPHF records {bulk['perf']['LLC-load-misses']['median']:.2f} LLC misses/op in the raw throughput loop because full-key comparison and copying the value touch more lines. TLB/page-size differences and instruction cost prevent interpreting these quotients as literal miss counts.

Slots cost 12 bytes/key, including a 16-bit fingerprint and two padding bytes; PtrHash costs approximately 2.99 bits/key. {' '.join(cost)} The original compressed SST is decoded to store independently comparable full keys. The measured record format omits sequence and kind and specializes lengths to u16; production visibility metadata and general lengths cost more. Construction timings are single-threaded CPU-8 proxies, include independently repeated builds, and are not additional measured compaction wall time. Peak builder memory was not bounded or measured.

The negative workload is new: 1M..2M IDs use the same reversed-ID key shape and cluster into 64 key-space gaps. Ordered indexes can reuse hot leaves, so these results do not establish uniformly scattered interior-negative cost. Existing Kiban Bloom rejection beats MPHF on bulk misses. There is no global SST maplet or sorted-search control over the same decoded arena. Keep Bloom filters as a design option and add that ablation before attributing gains solely to MPHF.

Correctness checks compare every one of the million existing values to deterministic model output before each process's timing and compare every one of the million timed results afterward; misses must return none. MPHF verifies the full byte key on hits, rejects duplicate build hashes, and checks one-to-one slots. All 100 accepted processes report zero errors and equal cross-variant digests. The analyzer verifies all counter coverage, trial/control counts, ten builds per layout, and unchanged private Kiban files. [Provenance](provenance.json) additionally compares private data with supplied source data. No engine `src/` files changed.

These measurements exclude memtable probes, snapshot/version ownership and ref-counting in the prototype, multi-version/tombstone logic, and per-read checksum verification. Input blocks are checked once during construction; new metadata needs equivalent validation. Range scans and latest-K receive no assumed benefit. LMDB here owns a copy before JSON parsing, whereas the existing readpath parses mapped bytes directly; that biases the end-to-end comparison slightly toward MPHF. Raw ownership is equal across variants. Anonymous huge pages and LMDB's file mapping have different TLB behavior. LMDB itself uses a copy-on-write B+tree, so this does not establish that LMDB could never use a static accelerator. See [methodology](METHODOLOGY.md) for configuration and sources.

[The V2 proposal](V2_PROPOSAL.md) sketches optional checksummed SST sections, independent full-key records, mixed-version support, flush/compaction construction, read/build memory budgets, snapshots/tombstones and installation/crash-safety obligations. It is proposal only. Before integration claims, repeat with a quiet sibling, equalized page-size conditions, a sorted decoded-arena control, and real memtable/version overhead.

## Measurements

'''+(OUT/'TABLES.md').read_text()+'''
## Evidence and reproduction

- [Standalone crate and commands](../../tools/agentbench/phash/README.md), including exact workload and timer boundaries.
- [Raw results, perf stat counters and metadata](results/metadata.json); `commands.jsonl` names all 100 successful processes.
- [Machine-readable analysis and gate](summary.json), generated by `analyze.py` without `--partial`.
- [Decision trail](decisions.tsv), [cross-model review](REVIEW.md), and [responses/self-review](REVIEW_RESPONSE.md).
- [Build output](build.log), [toolchain](toolchain.txt), and [source/copy/binary provenance](provenance.json).

Accepted measurements use the final recorded binary SHA. Earlier smoke and rejected setup files remain for audit and are excluded from analysis. They caught a native LMDB format-version mismatch and an aborted DBI-opening transaction. The final DBI-open transaction commits before any lookup; all accepted controls then pass full validation.
'''
(OUT/'REPORT.md').write_text(report)
print(report.split('\n')[2])
