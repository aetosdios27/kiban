#!/usr/bin/env python3
"""Summarize the final evidence, check completeness, and count physical SST bytes."""
import json
from pathlib import Path
import statistics as st
import struct

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent/'final'
summary = json.loads((OUT/'summary.json').read_text())
reads = [r for r in summary if r['action'] == 'reads']
varied = [r for r in summary if r['action'] == 'varied']
writes = [r for r in summary if r['action'] == 'writes']
assert len(reads) == 8 and len(varied) == 4 and len(writes) == 10
assert all(r['trials'] == 5 for r in reads+varied+writes)
latest = [r for r in reads if r['workload'] == 'recent-for-agent']
print('Latest-K paired ratios (each cell has five pairs)')
for r in latest:
    p50, p99 = [r['after'][f]['paired'] for f in ['p50_ns', 'p99_ns']]
    print(r['scenario'], p50, p99, 'PASS' if p50 <= .85 and p99 <= 1 else 'FAIL')

maintenance = json.loads((OUT/'maintenance/measurements.json').read_text())
assert len(maintenance) == 10
cost = {}
for field in ['flush_ns', 'compaction_build_ns', 'compaction_commit_ns']:
    before = {r['trial']:r[field] for r in maintenance if r['label'] == 'before'}
    after = {r['trial']:r[field] for r in maintenance if r['label'] == 'after'}
    ratios = [after[t]/before[t] for t in range(1,6)]
    cost[field] = dict(before=st.median(before.values()), after=st.median(after.values()),
                       paired=st.median(ratios), ratios=ratios)

ranges = []
for layout in ['bulk', 'normal']:
    for cache in ['512', 'default']:
        for count in [10, 256]:
            data = {}
            for trial in range(1,6):
                for label in ['before', 'after']:
                    path = OUT/'range'/f'{layout}-{cache}-r{trial}-{label}.log'
                    rows = [json.loads(line) for line in path.read_text().splitlines() if line.startswith('{')]
                    row = next(r for r in rows if r['count'] == count)
                    assert row['errors'] == 0 and row['verified_records'] == count*1000
                    data[trial,label] = row
            ranges.append(dict(layout=layout, cache=cache, count=count,
                **{field:dict(before=st.median(data[t,'before'][field] for t in range(1,6)),
                after=st.median(data[t,'after'][field] for t in range(1,6)),
                ratios=[data[t,'after'][field]/data[t,'before'][field] for t in range(1,6)])
                for field in ['p50_ns','p99_ns']}))

space = []
for label in ['before','after']:
    for layout in ['bulk','normal']:
        files = list((ROOT/'target/remix-data'/label/f'kiban-1000000-{layout}').glob('*.sst'))
        row = dict(label=label, layout=layout, files=len(files), sst_bytes=0, point_bytes=0, scan_bytes=0)
        for path in files:
            size = path.stat().st_size
            with path.open('rb') as f:
                f.seek(-68, 2)
                footer = f.read(68)
                assert footer[-8:] == b'KIBANSST'
                offset, length = struct.unpack_from('<QQ', footer)
                f.seek(offset)
                header = f.read(64)
                assert header[:8] == b'KIBANPH3'
                flags = struct.unpack_from('<I', header, 52)[0]
                assert flags & ~2 == 0
                if flags & 2:
                    f.seek(offset+length-13)
                    row['scan_bytes'] += struct.unpack('<Q', f.read(8))[0]+8
            row['sst_bytes'] += size
            row['point_bytes'] += length
        space.append(row)

sibling = []
for path in OUT.rglob('sibling.jsonl'):
    sibling.extend(json.loads(line)['busy_fraction'] for line in path.read_text().splitlines())
for folder in ['range','cost','maintenance']:
    sibling.extend(json.loads(line)['busy_fraction'] for line in (OUT/folder/'commands.jsonl').read_text().splitlines())
result = dict(latest_k_pass=all(r['after']['p50_ns']['paired'] <= .85 and r['after']['p99_ns']['paired'] <= 1 for r in latest),
              maintenance=cost, ranges=ranges, space=space,
              cpu4_busy=dict(min=min(sibling), median=st.median(sibling), max=max(sibling), processes=len(sibling)))
(OUT/'cost-summary.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result, indent=2))
