#!/usr/bin/env python3
"""Validate all paired receipts and generate reviewable integrated-filter tables."""
import argparse
import collections
import csv
import json
from pathlib import Path
import statistics as st


def median_range(values, scale=1):
    return f'{st.median(values)/scale:.3f} [{min(values)/scale:.3f}, {max(values)/scale:.3f}]'


def summary(groups):
    rows = []
    for group, sides in sorted(groups.items()):
        assert set(sides) == {'before', 'after'}, group
        before, after = sides['before'], sides['after']
        assert set(before) == set(after) == set(range(1, 6)), group
        a, b = list(before.values()), list(after.values())
        ratios = [after[t]['call_seconds']/before[t]['call_seconds'] for t in range(1, 6)]
        rows.append(dict(group=group,
                         before_p50=median_range([r['latency']['p50_ns'] for r in a], 1000),
                         after_p50=median_range([r['latency']['p50_ns'] for r in b], 1000),
                         before_p99=median_range([r['latency']['p99_ns'] for r in a], 1000),
                         after_p99=median_range([r['latency']['p99_ns'] for r in b], 1000),
                         before_call_ns=median_range([r['call_seconds']*1e9/r['latency']['count'] for r in a]),
                         after_call_ns=median_range([r['call_seconds']*1e9/r['latency']['count'] for r in b]),
                         paired_ratio=median_range(ratios), ratios=ratios))
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[3])
    a = p.parse_args()
    out = a.root/'validation/fuse-engine'
    results = out/'results'
    groups = collections.defaultdict(lambda: collections.defaultdict(dict))
    receipts = list((results/'reads').glob('*.receipt.json')) + list((results/'writes').glob('*.receipt.json'))
    assert len(receipts) == 360, f'Expected 320 read/miss + 40 write/control processes, got {len(receipts)}'
    sibling = []
    for path in receipts:
        r = json.loads(path.read_text())
        assert r['code'] == 0
        sibling.append(r['cpu9_busy_percent'])
        c = json.loads(path.with_name(path.name.replace('.receipt.json', '.config.json')).read_text())
        fields = r['name'].split('-')
        trial = int(next(v[1:] for v in fields if v.startswith('r') and v[1:].isdigit()))
        label = fields[-2]
        for stage in r['stages']:
            assert stage['errors'] == 0 and stage['latency']['count'] == c['operations']
            if c['action'] == 'write':
                group = ('writes', c['backend'], str(c['durable']), stage['workload'])
            else:
                group = (c['layout'], c['backend'], str(c['cache_mib']), str(c['point_index']), stage['workload'])
            assert trial not in groups[group][label]
            groups[group][label][trial] = stage
    rows = summary(groups)
    by_group = {tuple(r['group']): r for r in rows}
    for r in rows:
        if r['group'][1] == 'kiban':
            control = by_group[(r['group'][0], 'lmdb', *r['group'][2:])]
            r['control_adjusted_ratio'] = median_range([a/b for a,b in zip(r['ratios'],control['ratios'])])
        else:
            r['control_adjusted_ratio'] = '—'
    (out/'paired-summary.json').write_text(json.dumps(rows, indent=2)+'\n')
    with (out/'paired-summary.csv').open('w') as f:
        writer = csv.DictWriter(f, fieldnames=[k for k in rows[0] if k != 'ratios'])
        writer.writeheader()
        for r in rows:
            writer.writerow({**{k: v for k, v in r.items() if k != 'ratios'}, 'group': '/'.join(r['group'])})
    text = ['# Integrated filter paired tables', '',
            'Five paired trials; every side followed by LMDB control. Values are median [min, max] of five per-run statistics. Latency units: µs. Paired ratio uses total timed call seconds, matched by trial. All 360 processes returned success, consumed every result, and checked their expected work counts.', '',
            f'CPU 9 busy percentage over timing processes: {median_range(sibling)}. Timing processes were pinned to CPU 8; sibling activity is measured over each whole process, including verification.', '',
            '| Layout | Backend | Cache MiB | PH3 | Workload | Before p50 µs | After p50 µs | Before p99 µs | After p99 µs | Paired call-time ratio | Relative to paired LMDB |',
            '|---|---|---|---|---|---:|---:|---:|---:|---:|---:|']
    for r in rows:
        group = list(r['group'])
        if group[0] == 'writes':
            group = ['writes', group[1], 'default', 'off', f'{group[3]} (durable={group[2]})']
        else:
            group[2] = 'default' if group[2] == 'None' else group[2]
            group[3] = 'on' if group[3] == 'True' else 'off'
        text.append('| '+' | '.join(group+[r[k] for k in ['before_p50','after_p50','before_p99','after_p99','paired_ratio','control_adjusted_ratio']])+' |')
    inv = json.loads((results/'sst-inventory.json').read_text())
    totals = collections.defaultdict(lambda: dict(files=0, bytes=0, filter_bytes=0, fallbacks=0))
    for r in inv:
        total = totals[r['db']]
        total['files'] += 1
        total['bytes'] += r['bytes']
        total['filter_bytes'] += r['filter_bytes']
        total['fallbacks'] += int(r['algorithm'] == 0)
    text += ['', '## SST fixture costs', '', '| Fixture | SST files | SST bytes | Filter bytes | Positive fallbacks |','|---|---:|---:|---:|---:|']
    for name, r in sorted(totals.items()):
        text.append(f"| {name} | {r['files']} | {r['bytes']} | {r['filter_bytes']} | {r['fallbacks']} |")
    maintenance = collections.defaultdict(lambda: collections.defaultdict(dict))
    maint_receipts = list(results.glob('maintenance-*.receipt.json'))
    assert len(maint_receipts) == 10
    for path in maint_receipts:
        r = json.loads(path.read_text())
        label = r['name'].split('-')[-1]
        for row in r['records']:
            assert row['errors'] == 0 and row['verified_records'] == 1_000_000
            maintenance[row['point_index']][label][row['trial']] = row
    text += ['', '## Maintenance', '', 'One million binary keys and 296-byte values; two 500k-key flushes then forced L0 merge, five paired trials. Construction, CRC, full member verification, sync and reopening are included in flush/build; fill and final full-member checks are excluded. Build includes atomic SST writes and open; manifest commit is separate.', '',
             '| PH3 | Metric | Before median [min, max] | After median [min, max] | Paired ratio |','|---|---|---:|---:|---:|']
    maint_json = []
    for indexed, sides in sorted(maintenance.items()):
        assert set(sides['before']) == set(sides['after']) == set(range(1, 6))
        for field in ['flush_ns','compaction_build_ns','compaction_commit_ns','input_bytes','output_bytes','point_resident_bytes']:
            scale = 1e6 if field.endswith('_ns') else 1
            before = [sides['before'][t][field] for t in range(1, 6)]
            after = [sides['after'][t][field] for t in range(1, 6)]
            ratios = [b/a for a,b in zip(before, after)] if any(before) else [1]*5
            row = dict(indexed=indexed, field=field, before=median_range(before,scale),after=median_range(after,scale),paired_ratio=median_range(ratios))
            maint_json.append(row)
            text.append(f"| {'on' if indexed else 'off'} | {field.replace('_ns',' ms')} | {row['before']} | {row['after']} | {row['paired_ratio']} |")
    (out/'maintenance-summary.json').write_text(json.dumps(maint_json, indent=2)+'\n')
    (out/'TABLES.md').write_text('\n'.join(text)+'\n')
    gates = json.loads((out/'gates-final/status.json').read_text())
    assert len(gates) == 11 and all(r['code'] == 0 for r in gates)
    assert json.loads((out/'gates-final/initial-source-hashes.json').read_text()) == json.loads((out/'gates-final/source-hashes.json').read_text())
    (out/'analysis-check.json').write_text(json.dumps(dict(processes=len(receipts), trials=5, maintenance_processes=len(maint_receipts), errors=0, gates=len(gates), sibling_median=st.median(sibling), sibling_min=min(sibling), sibling_max=max(sibling), fixtures=totals),indent=2)+'\n')
    print('Validated 360 paired read/write/control processes, ten maintenance processes, and all eleven gates.')


if __name__ == '__main__':
    main()
