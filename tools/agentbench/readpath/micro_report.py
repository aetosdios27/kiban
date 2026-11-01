#!/usr/bin/env python3
"""Validate and summarize every alternating component trial and counter."""
import json
from pathlib import Path
import statistics
import sys

folder = Path(sys.argv[1])
metadata = json.loads((folder / 'metadata.json').read_text())
trials = metadata['trials']
rows = {}
for label, case in metadata['cases'].items():
    values = []
    for trial in range(1, trials + 1):
        stem = f'r{trial}-{label}-{case["component"]}'
        data = [json.loads(line) for line in (folder / (stem+'.log')).read_text().splitlines()
                if line.startswith('{')]
        assert len(data) == 1 and data[0]['errors'] == 0 and data[0]['operations'] == 1000000
        result = {'ns/op': data[0]['ns_per_op']}
        for line in (folder / (stem+'.stat')).read_text().splitlines():
            if not line or line.startswith('#'):
                continue
            fields = line.split(';')
            assert float(fields[4]) == 100, fields
            event = fields[2].split('/')[1]
            result[event+'/op'] = float(fields[0]) / data[0]['operations']
        values.append(result)
    rows[label] = values
print('| Measure | Before median [min, max] | After median [min, max] |')
print('| --- | ---: | ---: |')
for field in rows['before'][0]:
    cells = []
    for label in ('before', 'after'):
        values = [r[field] for r in rows[label]]
        cells.append(f'{statistics.median(values):.3f} [{min(values):.3f}, {max(values):.3f}]')
    print('| '+field+' | '+' | '.join(cells)+' |')
