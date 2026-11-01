#!/usr/bin/env python3
import collections
import json
from pathlib import Path
import statistics
import sys
folder=Path(sys.argv[1]); rows=collections.defaultdict(list)
commands=[json.loads(line) for line in (folder/'commands.jsonl').read_text().splitlines()]
assert len(commands)==30
for command in commands:
    path=folder/command['config'].replace('.config.json','.jsonl')
    row=json.loads(path.read_text())
    assert row['errors']==0 and row['verified_calls']==row['operations']==10000
    assert row['storage_read_bytes']>0, 'cold probe did not perform storage reads'
    if row['backend']=='lmdb' or row['mmap_max_level'] is not None:
        assert row['evicted_mappings']>0, 'no dataset mapping was evicted'
        assert row['major_faults']>0, 'mapped probe did not fault data from storage'
    rows[row['readers'],path.stem.split('-')[-1]].append(row)
print('Cold-start distributions after file-specific DONTNEED, 10k verified point reads per process. '
      'All reader threads share CPU 3. These are not claims that every call remained cold: '
      'major faults and storage reads below prove that real cold I/O occurred. '
      'No global page-cache drop was used. Five alternating trials; medians [min, max].\n')
print('| Readers | Path | p50 µs | p99 µs | p999 µs | Major faults | Storage read MiB |')
print('| ---: | --- | ---: | ---: | ---: | ---: | ---: |')
for (readers,case),values in sorted(rows.items()):
    assert len(values)==5
    cells=[]
    for field,scale in [('p50_ns',1000),('p99_ns',1000),('p999_ns',1000),('major_faults',1),('storage_read_bytes',1<<20)]:
        numbers=[(r['latency'][field] if field.endswith('_ns') else r[field])/scale for r in values]
        cells.append(f'{statistics.median(numbers):.3f} [{min(numbers):.3f}, {max(numbers):.3f}]')
    print(f'| {readers} | {case} | '+' | '.join(cells)+' |')
