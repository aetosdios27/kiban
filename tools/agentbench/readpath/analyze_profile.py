#!/usr/bin/env python3
"""Normalize isolated perf counters by the recorded operation count."""
import json
from pathlib import Path
import re

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'validation/read-path-results'
folder=BASE/'profile-resumed'

def counters(path):
    fields={}
    for line in path.read_text().splitlines():
        if ';cpu_core/' in line:
            row=line.split(';')
            assert float(row[4])==100.0, row
            fields[row[2].split('/')[1]]=float(row[0])
    return fields

def result(path):
    return next(json.loads(line) for line in path.read_text().splitlines() if line.startswith('{'))

out=['# Isolated hardware counters','',
     'All user events ran 100% of their enabled interval. Counts are divided by the configuration operation count. IPC is instructions/cycles. Profiling timings include destruction of returned objects and are excluded from latency comparisons.','',
     '| Layout | MiB | Workload | Engine | Cycles/op | Instructions/op | IPC | L1 load misses/op | LLC load misses/op | Branch misses/op | dTLB load misses/op |',
     '| --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
for path in sorted(folder.glob('*.stat')):
    if path.stem.startswith('micro-'):continue
    c=json.loads(path.with_suffix('.config.json').read_text());n=c['operations'];f=counters(path)
    row=path.stem.split('-')
    vals=[f[x]/n for x in ('cycles','instructions')]
    vals+=[f['instructions']/f['cycles']]
    vals += [f[x]/n for x in ('L1-dcache-load-misses','LLC-load-misses','branch-misses','dTLB-load-misses')]
    out.append('| '+' | '.join([*row,*[f'{v:.3f}' for v in vals]])+' |')
out+=['','## Component microbenchmarks','',
      'One isolated million-operation loop per component. These costs overlap and include their access pattern; do not add them or substitute them for end-to-end latency.','',
      '| Component | ns/op | Cycles/op | Instructions/op | LLC load misses/op |',
      '| --- | ---: | ---: | ---: | ---: |']
for path in sorted(folder.glob('micro-*.stat')):
    c=json.loads(path.with_suffix('.config.json').read_text());n=c['operations'];f=counters(path);r=result(path.with_suffix('.log'))
    vals=[r['ns_per_op'],f['cycles']/n,f['instructions']/n,f['LLC-load-misses']/n]
    out.append('| '+path.stem+' | '+' | '.join(f'{v:.3f}' for v in vals)+' |')
(BASE/'COUNTERS.md').write_text('\n'.join(out)+'\n')

out=['# Sample evidence','',
     'Self percentages come from perf report. Empty DWARF chains are restored from their sampled instruction pointer and labeled unwound_callers_unavailable. Every decoded sample contributes its complete period to the SVG; missing callers are not invented.','']
for path in sorted((BASE/'profile-complete-stacks').glob('*.report')):
    out += ['## '+path.stem,'',f'[SVG](profile-complete-stacks/{path.stem}.svg), [text report](profile-complete-stacks/{path.name}), [sample coverage](profile-complete-stacks/{path.stem}.coverage.json)','',
            '| Self % | Symbol |','| ---: | --- |']
    rows=re.findall(r'^\s+([\d.]+)%\s+\S+\s+\S+\s+\[.\]\s+(.+)$',path.read_text(),re.M)
    for pct,symbol in rows[:12]:out.append(f'| {pct} | `{symbol.replace("|", "&#124;")}` |')
(BASE/'SAMPLES.md').write_text('\n'.join(out)+'\n')

def category(symbol):
    if 'memcmp' in symbol or 'get_from_version_at' in symbol or any(n in symbol for n in ('mdb_page_search','mdb_node_search','mdb_cmp_memn')):
        return 'Search, routing and byte comparison'
    if 'BlockCache' in symbol or 'Hasher' in symbol or 'BuildHasher' in symbol or 'Arc<' in symbol:
        return 'Kiban block-cache lookup/ownership'
    if any(n in symbol for n in ('header_at','VerifiedBlock','seek_restart_index','full_key_at_restart')):
        return 'Kiban in-block entry decoding'
    if 'serde_json' in symbol or 'core::str::' in symbol:
        return 'JSON and text decoding'
    if symbol.startswith(('mi_','_mi_','_int_','__rust')):
        return 'Heap allocation/free'
    return 'Other work'

out=['# Approximate attribution of cache-hit point-read cycles','',
     'Exclusive self-symbol percentages from the complete sample reports are multiplied by cycles/op from the separate stat runs. These are attribution estimates, not independently additive microbenchmark timings. Search includes LMDB B-tree work and Kiban table routing, inlined index/filter work and byte comparisons. Missing parent chains do not affect sampled instruction pointers.','',
     '| Category | Bulk K cycles/op | Bulk LMDB | Bulk extra | Normal K | Normal LMDB | Normal extra |',
     '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
groups={}
for layout in ('bulk','normal'):
    for engine in ('kiban','lmdb'):
        name=f'{layout}-512-w2-{engine}'
        cycles=counters(folder/(name+'.stat'))['cycles']/1000000
        report=(BASE/'profile-complete-stacks'/(name+'-stacks.report')).read_text()
        sums={}
        for pct,symbol in re.findall(r'^\s+([\d.]+)%\s+\S+\s+\S+\s+\[.\]\s+(.+)$',report,re.M):
            group=category(symbol);sums[group]=sums.get(group,0)+float(pct)*cycles/100
        groups[layout,engine]=sums
names=set().union(*[set(v) for v in groups.values()])
for name in sorted(names,key=lambda n:groups['bulk','kiban'].get(n,0)-groups['bulk','lmdb'].get(n,0),reverse=True):
    values=[]
    for layout in ('bulk','normal'):
        k=groups[layout,'kiban'].get(name,0);lm=groups[layout,'lmdb'].get(name,0);values.extend([k,lm,k-lm])
    out.append('| '+name+' | '+' | '.join(f'{x:.0f}' for x in values)+' |')
(BASE/'CYCLE_ATTRIBUTION.md').write_text('\n'.join(out)+'\n')
