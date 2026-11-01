#!/usr/bin/env python3
"""Compare complete alternating read trials, retaining contemporary controls."""
import collections
import json
from pathlib import Path
import statistics
import sys

def load(folder):
    commands=[json.loads(line) for line in (folder/'commands.jsonl').read_text().splitlines()]
    assert len(commands)==80, (folder,len(commands))
    result=collections.defaultdict(dict)
    for command in commands:
        path=folder/command['config'].replace('.config.json','.jsonl')
        rows=[json.loads(line) for line in path.read_text().splitlines()]
        assert rows[-1]['type']=='complete' and rows[-1]['errors']==0 and rows[-1]['timed_writes']==0
        parts=path.stem.split('-')
        for row in rows[:-1]:
            assert row['errors']==0 and row['reads']['count']==100000
            result[(*parts[:3],row['workload'])][parts[-1],parts[3]]=row
    return result

before=load(Path(sys.argv[1]));after=load(Path(sys.argv[2]))
print('| N | Layout | MiB | Workload | K p50 before → after | K p99 before → after | LMDB p50 before → after | LMDB p99 before → after | Paired K/LMDB p50 before → after |')
print('| ---: | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |')
for key in sorted(before):
    cells=[]
    for engine in ('kiban','lmdb'):
        for pct in ('p50_ns','p99_ns'):
            cells.append(' → '.join(f'{statistics.median(s[key][engine,"r"+str(i)]["latency"][pct] for i in range(1,6))/1000:.3f}' for s in (before,after)))
    ratio=[]
    for s in (before,after):
        ratios=[s[key]['kiban','r'+str(i)]['latency']['p50_ns']/s[key]['lmdb','r'+str(i)]['latency']['p50_ns'] for i in range(1,6)]
        ratio.append(f'{statistics.median(ratios):.3f} [{min(ratios):.3f}, {max(ratios):.3f}]')
    print('| '+' | '.join([*key,*cells,' → '.join(ratio)])+' |')
