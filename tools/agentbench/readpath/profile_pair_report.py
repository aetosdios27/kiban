#!/usr/bin/env python3
"""Summarize full-read counters with contemporary LMDB controls."""
import json
from pathlib import Path
import statistics
import sys

folder = Path(sys.argv[1])
metadata = json.loads((folder / 'metadata.json').read_text())
rows = {}
for engine in ('kiban', 'lmdb'):
    for label in ('before', 'after'):
        values = []
        for trial in range(1, metadata['trials'] + 1):
            stem = f'r{trial}-{label}-{engine}'
            data = [json.loads(line) for line in (folder / (stem+'.log')).read_text().splitlines()
                    if line.startswith('{')]
            assert len(data) == 1 and data[0]['errors'] == 0 and data[0]['operations'] == 1000000
            r = {'ns/op': data[0]['ns'] / data[0]['operations']}
            for line in (folder / (stem+'.stat')).read_text().splitlines():
                if not line or line.startswith('#'):
                    continue
                fields = line.split(';')
                assert float(fields[4]) == 100, fields
                r[fields[2].split('/')[1]+'/op'] = float(fields[0]) / data[0]['operations']
            r['IPC'] = r['instructions/op'] / r['cycles/op']
            values.append(r)
        rows[engine, label] = values
print('| Measure | K before [min, max] | K candidate [min, max] | LMDB before [min, max] | LMDB candidate control [min, max] |')
print('| --- | ---: | ---: | ---: | ---: |')
for field in rows['kiban', 'before'][0]:
    cells = []
    for engine, label in [('kiban','before'), ('kiban','after'), ('lmdb','before'), ('lmdb','after')]:
        values = [r[field] for r in rows[engine, label]]
        cells.append(f'{statistics.median(values):.3f} [{min(values):.3f}, {max(values):.3f}]')
    print('| '+field+' | '+' | '.join(cells)+' |')
