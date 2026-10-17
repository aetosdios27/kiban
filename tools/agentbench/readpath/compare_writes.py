#!/usr/bin/env python3
"""Compare write latency and call throughput, retaining every trial range."""
import collections
import json
from pathlib import Path
import statistics
import sys

def load(folder):
    result=collections.defaultdict(list)
    commands=[json.loads(s) for s in (folder/'commands.jsonl').read_text().splitlines()]
    assert len(commands)==20
    for command in commands:
        path=folder/command['config'].replace('.config.json','.jsonl')
        for line in path.read_text().splitlines():
            row=json.loads(line)
            if row.get('workload') in ('remember','update'):
                assert row['errors']==0
                result[path.stem.split('-')[2],path.stem.split('-')[-1],row['workload']].append(row)
    return result

before=load(Path(sys.argv[1]));after=load(Path(sys.argv[2]))
print('| Mode | Engine | Workload | p50 µs before → after | p99 µs before → after | Call ops/s before → after |')
print('| --- | --- | --- | ---: | ---: | ---: |')
for key in sorted(before):
    cells=[]
    for field in ('p50_ns','p99_ns','call_ops_s'):
        formatted=[]
        for rows in (before[key],after[key]):
            values=[r[field] if field=='call_ops_s' else r['latency'][field]/1000 for r in rows]
            assert len(values)==5
            formatted.append(f'{statistics.median(values):.3f} [{min(values):.3f}, {max(values):.3f}]')
        cells.append(' → '.join(formatted))
    print('| '+' | '.join([*key,*cells])+' |')
