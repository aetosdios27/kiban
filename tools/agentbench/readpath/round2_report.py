#!/usr/bin/env python3
"""Audit complete paired runs and print their median read/write latencies."""
import argparse
import json
from pathlib import Path
import statistics

parser = argparse.ArgumentParser()
parser.add_argument('folder', type=Path)
args = parser.parse_args()
print('Five alternating before/after trials, each followed by its LMDB control. '
      'Latencies are median trial p50 / p99 in microseconds. All timed results '
      'are checked outside their timer, with full dataset verification before '
      'and after the reads. Write trials use fresh copies and verify the final state.\n')
print('| Scenario | Workload | Before Kiban p50 / p99 | After Kiban p50 / p99 | After LMDB p50 / p99 | Paired after K/LMDB p50 [range] |')
print('| --- | --- | ---: | ---: | ---: | ---: |')
for folder in sorted(args.folder.iterdir()):
    if not folder.is_dir() or not (folder/'commands.jsonl').exists():
        continue
    if (folder/'EXCLUDED.md').exists():
        print(f'\nExcluded {folder.name}: see `{folder.name}/EXCLUDED.md`.\n')
        continue
    commands = [json.loads(line) for line in (folder/'commands.jsonl').read_text().splitlines()]
    assert len(commands) == 20, (folder, len(commands))
    rows = {}
    for command in commands:
        name = command['config'].replace('.config.json', '')
        _, _, _, trial, label, engine = name.split('-')
        records = [json.loads(line) for line in (folder/(name+'.jsonl')).read_text().splitlines()]
        assert records[-1]['type'] == 'complete', (folder, name)
        config=json.loads((folder/command['config']).read_text())
        expected=config['n'] + (config['operations'] if config['action']=='write' else 0)
        assert records[-1]['verified_records'] == expected, (folder, name, 'verification count')
        assert all(row['latency']['count'] == config['operations'] for row in records if row['type']=='stage'), (folder, name, 'operation count')
        assert all(row.get('errors', 0) == 0 for row in records)
        assert command['affinity'] == '3'
        for row in records:
            if row['type'] == 'stage':
                rows[row['workload'], trial, label, engine] = row
    for work in sorted({key[0] for key in rows}):
        cells = []
        for label, engine in [('before', 'kiban'), ('after', 'kiban'), ('after', 'lmdb')]:
            cells.append(' / '.join(f'{statistics.median(rows[work, f"r{i}", label, engine]["latency"][field]/1000 for i in range(1, 6)):.3f}' for field in ['p50_ns', 'p99_ns']))
        ratios = [rows[work, f'r{i}', 'after', 'kiban']['latency']['p50_ns']/rows[work, f'r{i}', 'after', 'lmdb']['latency']['p50_ns'] for i in range(1, 6)]
        print(f'| {folder.name} | {work} | '+ ' | '.join(cells) + f' | {statistics.median(ratios):.3f} [{min(ratios):.3f}, {max(ratios):.3f}] |')
