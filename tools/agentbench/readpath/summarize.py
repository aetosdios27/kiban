#!/usr/bin/env python3
"""Print auditable medians and ranges; never drop a completed trial."""
import collections
import json
from pathlib import Path
import statistics
import sys

folder=Path(sys.argv[1])
rows=collections.defaultdict(list)
for path in sorted(folder.glob('*-r*-*.jsonl')):
    for line in path.read_text().splitlines():
        row=json.loads(line)
        if row['type']=='stage':
            assert row['errors']==0
            parts=path.stem.split('-')
            key=(*parts[:3],parts[-1],row['workload'])
            rows[key].append(row)
print('| N | Layout | Cache/mode | Engine | Workload | Trials | p50 µs [min, max] | p99 µs [min, max] |')
print('| ---: | --- | --- | --- | --- | ---: | ---: | ---: |')
for key, samples in sorted(rows.items()):
    def field(name):
        values=[s['latency'][name]/1000 for s in samples]
        return f'{statistics.median(values):.3f} [{min(values):.3f}, {max(values):.3f}]'
    print('| '+' | '.join((*key,str(len(samples)),field('p50_ns'),field('p99_ns')))+' |')
