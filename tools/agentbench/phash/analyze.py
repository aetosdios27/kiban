#!/usr/bin/env python3
"""Derive all reported tables from retained JSONL and perf stat evidence."""
import argparse, json, statistics
from collections import defaultdict
from pathlib import Path
median=statistics.median

def spread(xs):
    return dict(median=median(xs),min=min(xs),max=max(xs))
def cell(s, decimals=1):
    return f"{s['median']:.{decimals}f} [{s['min']:.{decimals}f}, {s['max']:.{decimals}f}]"
def perf(path, ops):
    counters={}
    for line in path.read_text().splitlines():
        cols=line.split(';')
        if len(cols)>4 and cols[2].startswith('cpu_core/'):
            assert '<' not in cols[0], (path,line)
            assert float(cols[4])>=99, ('multiplexed',path,line)
            counters[cols[2].split('/')[1]]=float(cols[0])/ops
    assert len(counters)==4, path
    return counters

def main():
    p=argparse.ArgumentParser();p.add_argument('results',type=Path);p.add_argument('--partial',action='store_true');a=p.parse_args()
    groups=defaultdict(lambda:defaultdict(list));builds=defaultdict(list)
    for path in a.results.glob('*-r*-*.jsonl'):
        if not path.read_text().strip():continue
        layout,trial,workload,_,variant=path.stem.split('-')
        rows=[json.loads(l) for l in path.read_text().splitlines()]
        if rows[-1]['type']!='read':continue
        row=rows[-1]; assert row['errors']==0 and row['verified_timed_results']==row['counter_operations'] and row['verified_all_hits']==1000000
        row['perf']=perf(path.with_suffix('.stat'),row['counter_operations'])
        groups[(layout,workload,variant)][trial].append(row)
        for b in rows[:-1]:
            if b['type']=='build':builds[layout].append(b)
    if not a.partial:
        assert len(groups)==12, ('incomplete groups', len(groups))
        assert len(list(a.results.glob('*-r*-*.jsonl')))==100
        assert len((a.results/'commands.jsonl').read_text().splitlines())==100
        for (layout,workload,variant),trials in groups.items():
            assert len(trials)==5
            assert all(len(rs)==(3 if variant=='lmdb' else 1) for rs in trials.values())
            assert len({r['digest'] for ts in groups.values() for rs in ts.values() for r in rs if r['missing']==(workload=='miss')})==1
        for layout in ['bulk','normal']:
            assert len(builds[layout])==10
            checks=json.loads((a.results/(layout+'-kiban-checksums.json')).read_text())
            assert checks['unchanged'] and checks['before']==checks['after']
    summary=[]
    for (layout,workload,variant),trials in sorted(groups.items()):
        if a.partial and any(t not in groups[(layout,workload,'lmdb')] for t in trials): continue
        if not a.partial: assert len(trials)==5
        row=dict(layout=layout,workload=workload,variant=variant,trials=len(trials),runs=sum(map(len,trials.values())))
        for suite in ['raw','e2e','perf']:
            row[suite]={metric:spread([median([r[suite][metric] for r in rs]) for rs in trials.values()]) for metric in next(iter(trials.values()))[0][suite]}
        for metric in ['throughput_ops_s','batch_mean_ns']:
            row[metric]=spread([median([r[metric] for r in rs]) for rs in trials.values()])
        row['raw_ratio_to_lmdb']=spread([median([r['raw']['p50_ns'] for r in rs])/median([r['raw']['p50_ns'] for r in groups[(layout,workload,'lmdb')][t]]) for t,rs in trials.items()])
        row['e2e_ratio_to_lmdb']=spread([median([r['e2e']['p50_ns'] for r in rs])/median([r['e2e']['p50_ns'] for r in groups[(layout,workload,'lmdb')][t]]) for t,rs in trials.items()])
        if variant=='lmdb':
            row['all_control_runs']={suite:{metric:spread([r[suite][metric] for rs in trials.values() for r in rs]) for metric in ['p50_ns','p99_ns','mean_ns']} for suite in ['raw','e2e']}
        summary.append(row)
    floor=defaultdict(list)
    for line in (a.results/'floor.jsonl').read_text().splitlines():
        r=json.loads(line);floor[r['length']].append(r['ns_per_chain'])
    floor={str(k):spread(v) for k,v in sorted(floor.items())}
    overhead={l:{k:spread([b[k] for b in bs]) for k in ['mphf_bits_per_key','slot_bytes_per_key','arena_bytes','original_sst_bytes','build_ns','mphf_build_ns']}|{'layout':bs[0]['layout'],'builds':len(bs)} for l,bs in builds.items()}
    gate={}
    if not a.partial:
        bulk={r['variant']:r for r in summary if r['layout']=='bulk' and r['workload']=='hit'}
        ratio=bulk['mphf']['raw']['p50_ns']['median']/bulk['lmdb']['raw']['p50_ns']['median']
        gate=dict(verdict='GO' if ratio<=0.8 else 'NO-GO',criterion='bulk hit median raw p50 <= 0.8x LMDB median raw p50',ratio=ratio,threshold=0.8,
                  worst_paired_raw=bulk['mphf']['raw_ratio_to_lmdb']['max'],worst_paired_e2e=bulk['mphf']['e2e_ratio_to_lmdb']['max'])
    floor_end=defaultdict(list)
    if (a.results/'floor-end.jsonl').exists():
        for line in (a.results/'floor-end.jsonl').read_text().splitlines():
            r=json.loads(line);floor_end[r['length']].append(r['ns_per_chain'])
    floor_end={str(k):spread(v) for k,v in sorted(floor_end.items())}
    out=dict(floor=floor,floor_end=floor_end,variants=summary,overhead=overhead,gate=gate)
    (a.results.parent/'summary.json').write_text(json.dumps(out,indent=2)+'\n')
    tables=[('PARTIAL PREVIEW. ' if a.partial else '')+'All cells are median [minimum, maximum] across completed trials (five in the final report). Each LMDB trial is the median of three bracketing controls. Ratios are medians of paired trial ratios.\n', '| Dependent loads | ns/chain |','|---|---:|']
    for l,s in floor.items():tables.append(f'| {l} | {cell(s)} |')
    if floor_end:
        tables+=['','End-of-suite floor, five further trials:','', '| Dependent loads | ns/chain |','|---|---:|']
        for l,s in floor_end.items(): tables.append(f'| {l} | {cell(s)} |')
    for suite in ['raw','e2e']:
        tables+=['',f'### {suite} latency','', '| Layout | Workload | Variant | p50 ns | p99 ns | Mean ns | p50 / LMDB |','|---|---|---|---:|---:|---:|---:|']
        for r in summary:tables.append(f"| {r['layout']} | {r['workload']} | {r['variant']} | {cell(r[suite]['p50_ns'])} | {cell(r[suite]['p99_ns'])} | {cell(r[suite]['mean_ns'])} | {cell(r[suite+'_ratio_to_lmdb'],3)} |")
    tables+=['','### Raw throughput and counters','', '| Layout | Workload | Variant | Mops/s | cycles/op | instructions/op | LLC misses/op | dTLB misses/op |','|---|---|---|---:|---:|---:|---:|---:|']
    for r in summary:
        thr={k:v/1e6 for k,v in r['throughput_ops_s'].items()}
        tables.append(f"| {r['layout']} | {r['workload']} | {r['variant']} | {cell(thr,3)} | {cell(r['perf']['cycles'])} | {cell(r['perf']['instructions'])} | {cell(r['perf']['LLC-load-misses'],3)} | {cell(r['perf']['dTLB-load-misses'],3)} |")
    tables+=['','### Build and space','', 'Build cells cover ten independently built MPHF instances per layout, including hit and miss processes. Construction is single-threaded on CPU 8.','', '| Layout | Levels: files | MPHF bits/key | Slot B/key | Decoded MiB | Original SST MiB | MPHF ms/1M | Total build ms/1M |','|---|---|---:|---:|---:|---:|---:|---:|']
    for l,b in overhead.items():
        scale=lambda k,d:{x:y/d for x,y in b[k].items()}
        tables.append(f"| {l} | {b['layout']} | {cell(b['mphf_bits_per_key'],4)} | {cell(b['slot_bytes_per_key'],0)} | {cell(scale('arena_bytes',2**20),2)} | {cell(scale('original_sst_bytes',2**20),2)} | {cell(scale('mphf_build_ns',1e6),2)} | {cell(scale('build_ns',1e6),2)} |")
    tables+=['','### Serial-load equivalents','', 'These divide raw p50 by the length-1 floor. They are latency equivalents, not measured dependency counts. The floor likely uses anonymous huge pages, whereas LMDB uses a file-backed mapping; TLB/page-walk overhead is not equalized. Code, copies, cache hits, and timer overhead remain in the numerator.','', '| Layout | Workload | Variant | Load equivalents |','|---|---|---|---:|']
    for r in summary:tables.append(f"| {r['layout']} | {r['workload']} | {r['variant']} | {r['raw']['p50_ns']['median']/floor['1']['median']:.2f} |")
    tables+=['','### Individual LMDB control variability','', 'These ranges include all 15 controls per layout/workload, before the within-trial median reduction.','', '| Layout | Workload | Raw p50 ns | E2E p50 ns |','|---|---|---:|---:|']
    for r in summary:
        if r['variant']=='lmdb':tables.append(f"| {r['layout']} | {r['workload']} | {cell(r['all_control_runs']['raw']['p50_ns'])} | {cell(r['all_control_runs']['e2e']['p50_ns'])} |")
    (a.results.parent/'TABLES.md').write_text('\n'.join(tables)+'\n')
    print(json.dumps(out,indent=2))
if __name__=='__main__':main()
