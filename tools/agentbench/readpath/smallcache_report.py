#!/usr/bin/env python3
"""Validate every paired receipt and report trial medians and ranges."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import statistics

PATTERN = re.compile(r'(bulk|normal)-index([01])-cache(32|512|None)-readers([14])-durable([01])-r([1-5])-(.+)-(kiban|lmdb)')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('folder', type=Path)
    parser.add_argument('--after', default='after')
    args = parser.parse_args()
    groups = defaultdict(dict)
    commands = [json.loads(s) for s in (args.folder/'commands.jsonl').read_text().splitlines()]
    for command in commands:
        assert command['affinity'] == '6'
        path = args.folder/command['config']
        match = PATTERN.fullmatch(path.name.removesuffix('.config.json'))
        assert match, path
        layout, indexed, cache, readers, durable, trial, label, engine = match.groups()
        config = json.loads(path.read_text())
        assert config['backend'] == engine
        rows = [json.loads(s) for s in path.with_name(path.name.replace('.config.json', '.jsonl')).read_text().splitlines()]
        assert all(row.get('errors', 0) == 0 for row in rows)
        action = config['action']
        if action == 'cold':
            assert len(rows) == 1
            row = rows[0]
            assert row['operations'] == row['verified_calls'] == 10000
            retained = engine == 'kiban' and indexed == '1' and cache in ['None', '512']
            if not retained:
                assert row['storage_read_bytes'] > 0, ('file pages were not cold', path)
            stages = [('point', row)]
        else:
            complete = next(row for row in rows if row['type'] == 'complete')
            assert complete['verified_records'] == (1000000 if action == 'read' else config['n'] + config['operations'])
            if action == 'read':
                assert 'Cpus_allowed_list:\t6\n' in complete['proc_status']
            stages = [(row['workload'], row) for row in rows if row['type'] == 'stage']
            assert len(stages) == (2 if action == 'read' else 5)
            assert all(row['latency']['count'] == config['operations'] for _, row in stages)
        for workload, row in stages:
            key = (layout, indexed, cache, readers, durable, workload)
            item = (label, engine, int(trial))
            assert item not in groups[key], (key, item)
            groups[key][item] = row
    summary = []
    lines = ['# Paired results', '',
             'Microseconds. Median [min, max] of five alternating trials per side with adjacent LMDB controls. CPU 6 only.', '',
             '| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |',
             '| --- | ---: | ---: | ---: | ---: | ---: |']
    details = ['', '## Trial ranges', '', '| Scenario / workload | Side | p50 | p99 |', '| --- | --- | ---: | ---: |']
    for key, data in sorted(groups.items()):
        expected = {(label, engine, t) for label in ['before', args.after] for engine in ['kiban', 'lmdb'] for t in range(1, 6)}
        assert set(data) == expected, (key, set(data) ^ expected)
        distributions = {}
        ratios = {}
        for label in ['before', args.after]:
            ratios[label] = [data[label, 'kiban', t]['latency']['p50_ns']/data[label, 'lmdb', t]['latency']['p50_ns'] for t in range(1, 6)]
            for engine in ['kiban', 'lmdb']:
                distributions[label+'-'+engine] = {
                    pct: [data[label, engine, t]['latency'][pct] for t in range(1, 6)]
                    for pct in ['p50_ns', 'p99_ns']}
                if next(iter(data.values()))['type'] == 'cold-start':
                    distributions[label+'-'+engine].update({
                        field: [data[label, engine, t][field] for t in range(1, 6)]
                        for field in ['storage_read_bytes', 'minor_faults', 'major_faults']})
                    distributions[label+'-'+engine]['p999_ns'] = [data[label, engine, t]['latency']['p999_ns'] for t in range(1, 6)]
        def pair(label, engine):
            d = distributions[label+'-'+engine]
            return ' / '.join(f'{statistics.median(d[pct])/1000:.3f}' for pct in ['p50_ns', 'p99_ns'])
        scenario = '/'.join(key)
        lines.append(f'| {scenario} | {pair("before", "kiban")} | {pair(args.after, "kiban")} | {pair("before", "lmdb")} | {pair(args.after, "lmdb")} | {statistics.median(ratios["before"]):.3f} / {statistics.median(ratios[args.after]):.3f} |')
        for side, values in distributions.items():
            cells = []
            for pct in ['p50_ns', 'p99_ns']:
                vals = values[pct]
                cells.append(f'{statistics.median(vals)/1000:.3f} [{min(vals)/1000:.3f}, {max(vals)/1000:.3f}]')
            details.append(f'| {scenario} | {side} | {cells[0]} | {cells[1]} |')
        summary.append(dict(scenario=key, distributions=distributions, ratios=ratios))
    (args.folder/'SUMMARY.md').write_text('\n'.join(lines+details)+'\n')
    (args.folder/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(f'Validated {len(commands)} commands and {len(summary)} scenario/workload groups.')


if __name__ == '__main__':
    main()
