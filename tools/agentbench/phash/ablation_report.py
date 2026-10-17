#!/usr/bin/env python3
"""Validate the completed ablation and write reproducible attribution tables."""
import json, re, statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'validation/v2/ablation'
rows={}
for path in OUT.glob('*-r*-*.jsonl'):
    layout,trial,variant,thp=path.stem.split('-')
    data=[json.loads(line) for line in path.read_text().splitlines()]
    row=next(r for r in data if r['type']=='read')
    assert row['errors']==0 and row['verified_all_hits']==1000000 and row['verified_timed_results']==1000000
    assert row['digest']==296732188 and row['operations']==100000
    if variant in ['mphf','sorted']:
        build=next(r for r in data if r['type']=='build')
        assert build['keys']==1000000 and build['arena_bytes']==338732640
    rows[layout,int(trial[1:]),variant,thp]=row
assert len(rows)==80, f'Incomplete: {len(rows)}/80'
assert len((OUT/'commands.jsonl').read_text().splitlines())==80
summary=[]
lines=['# Attribution ablation','',
       'Five alternating trials, CPU 8, 1M records, private copies. Every process verifies all stored values before timing and every timed result afterward. Raw samples return owned values; E2E includes formatting and JSON parsing. No build/test overlapped these runs. Host background benchmarks/crash sweeps remained active.','',
       'Sorted and MPHF both retain identical decoded arenas, original SST membership, routing, slots, MPHF and sorted-offset arrays. Only the locator changes; sorted uses binary search over full-key record offsets. THP allowed/disabled is checked with PR_GET_THP_DISABLE after optional PR_SET_THP_DISABLE, before backend allocation. Allowed does not force huge-page allocation. Memory samples record actual AnonHugePages.','',
       '| Layout | THP | Variant | Raw p50 ns median [range] | Raw p99 ns | E2E p50 ns | p50 / paired LMDB |',
       '|---|---|---|---:|---:|---:|---:|']
def med(v):return statistics.median(v)
def values(layout,thp,v,field,percentile):return [rows[layout,t,v,thp][field][percentile] for t in range(1,6)]
for layout in ['bulk','normal']:
    for thp in ['enabled','disabled']:
        for variant in ['lmdb','kiban','sorted','mphf']:
            p50=values(layout,thp,variant,'raw','p50_ns')
            p99=values(layout,thp,variant,'raw','p99_ns')
            e2e=values(layout,thp,variant,'e2e','p50_ns')
            ratios=[rows[layout,t,variant,thp]['raw']['p50_ns']/rows[layout,t,'lmdb',thp]['raw']['p50_ns'] for t in range(1,6)]
            lines.append(f'| {layout} | {thp} | {variant} | {med(p50):.0f} [{min(p50)}, {max(p50)}] | {med(p99):.0f} | {med(e2e):.0f} | {med(ratios):.3f} |')
            summary.append(dict(layout=layout,thp=thp,variant=variant,p50=med(p50),p99=med(p99),e2e_p50=med(e2e),p50_range=[min(p50),max(p50)],paired_ratio=med(ratios)))
lines+=['','## Attribution','',
         'A binary-search decoded representation can be slower than the old cached-block search. This control isolates changing the locator within the decoded representation; it does not isolate decoding from cache ownership, memory layout, hash computation, or query routing in the original engine. Do not add the components as independent percentage gains.','',
         '| Layout | THP | Sorted minus MPHF p50 ns | Kiban minus sorted p50 ns |', '|---|---|---:|---:|']
for layout in ['bulk','normal']:
    for thp in ['enabled','disabled']:
        get=lambda v:med(values(layout,thp,v,'raw','p50_ns'))
        lines.append(f'| {layout} | {thp} | {get("sorted")-get("mphf"):.0f} | {get("kiban")-get("sorted"):.0f} |')
lines+=['','| Layout | Variant | Disabled minus allowed p50 ns | Disabled / allowed |','|---|---|---:|---:|']
for layout in ['bulk','normal']:
    for v in ['lmdb','kiban','sorted','mphf']:
        a=med(values(layout,'enabled',v,'raw','p50_ns')); b=med(values(layout,'disabled',v,'raw','p50_ns'))
        lines.append(f'| {layout} | {v} | {b-a:.0f} | {b/a:.3f} |')
mem={}
for line in (OUT/'memory.jsonl').read_text().splitlines():
    r=json.loads(line); key=r['title']; kb=int(re.search(r'^AnonHugePages:\s+(\d+)',r['rollup'],re.M)[1]);mem[key]=max(mem.get(key,0),kb)
lines+=['','Sampled per-process maximum AnonHugePages (KiB; opportunistic, not synchronized peak RSS):','', '| THP | Variant | Median [min,max] KiB |','|---|---|---:|']
for thp in ['enabled','disabled']:
    for v in ['lmdb','kiban','sorted','mphf']:
        kb=[x for k,x in mem.items() if k.endswith(f'-{v}-{thp}')]
        if kb:lines.append(f'| {thp} | {v} | {med(kb):.0f} [{min(kb)}, {max(kb)}] |')
lines+=['','THP-disabled samples retain 2–4 MiB of pre-main anonymous huge pages. PR_SET_THP_DISABLE is applied before backend/query allocations, not before process startup; the large decoded arena is not huge-page backed in those runs. THP is a process-wide ablation, so the observed difference includes all anonymous allocations, not only the MPHF.','',
         'The gain is retained with THP disabled. The real engine implementation will preserve full-key verification and MVCC, keep Bloom-first rejection, charge resident arenas against the existing cache budget, and fall back to sorted blocks when a section cannot be admitted. Integrated performance remains to be measured.','']
(OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
(OUT/'REPORT.md').write_text('\n'.join(lines))
print('\n'.join(lines))
