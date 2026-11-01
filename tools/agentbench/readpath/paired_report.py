#!/usr/bin/env python3
"""Summarize interleaved frozen binaries and their contemporary LMDB controls."""
import collections
import json
from pathlib import Path
import statistics
import sys

folder=Path(sys.argv[1]);commands=[json.loads(line) for line in (folder/'commands.jsonl').read_text().splitlines()]
assert len(commands)==20
rows=collections.defaultdict(dict)
for command in commands:
    path=folder/command['config'].replace('.config.json','.jsonl')
    parts=path.stem.split('-');trial,label,engine=parts[3:]
    for line in path.read_text().splitlines():
        r=json.loads(line)
        if r['type']=='stage':
            assert r['errors']==0
            rows[r['workload']][label,engine,trial]=r

print('| Workload | K p50 µs before → after | K p99 before → after | LMDB p50 before → after | LMDB p99 before → after | Paired K/LMDB p50 before → after | K call ops/s before → after |')
print('| --- | ---: | ---: | ---: | ---: | ---: | ---: |')
for work,data in sorted(rows.items()):
    cells=[]
    for engine,field in [('kiban','p50_ns'),('kiban','p99_ns'),('lmdb','p50_ns'),('lmdb','p99_ns')]:
        parts=[]
        for label in ('before','after'):
            values=[data[label,engine,'r'+str(i)]['latency'][field]/1000 for i in range(1,6)]
            parts.append(f'{statistics.median(values):.3f} [{min(values):.3f}, {max(values):.3f}]')
        cells.append(' → '.join(parts))
    parts=[]
    for label in ('before','after'):
        values=[data[label,'kiban','r'+str(i)]['latency']['p50_ns']/data[label,'lmdb','r'+str(i)]['latency']['p50_ns'] for i in range(1,6)]
        parts.append(f'{statistics.median(values):.3f} [{min(values):.3f}, {max(values):.3f}]')
    cells.append(' → '.join(parts))
    cells.append(' → '.join(f'{statistics.median(data[label,"kiban","r"+str(i)]["call_ops_s"] for i in range(1,6)):.0f}' for label in ('before','after')))
    print('| '+work+' | '+' | '.join(cells)+' |')
