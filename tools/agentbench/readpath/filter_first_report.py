#!/usr/bin/env python3
"""Fetch/validate paired read+miss receipts from one results directory and
emit the index-on paired table rows for the filter-before-point-index cell
of the integrated filter experiment.

Usage: filter_first_report.py [--results DIR] [--md OUT.md]
Validation mirrors fuse_engine_report.py; rows cover every layout, cache,
index switch (on and off) and workload, so the caller picks the subset it
publishes. Never replaces a frozen fixture set: it reads receipts only.
"""
import argparse
import collections
import csv
import json
from pathlib import Path
import statistics as st


def median_range(values, scale=1):
    return f'{st.median(values)/scale:.3f} [{min(values)/scale:.3f}, {max(values)/scale:.3f}]'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--results', type=Path,
                   default=Path(__file__).resolve().parents[3].joinpath('validation/fuse-engine/results2'))
    p.add_argument('--md', type=Path)
    a = p.parse_args()
    results = a.results
    groups = collections.defaultdict(lambda: collections.defaultdict(dict))
    receipts = sorted((results / 'reads').glob('*.receipt.json'))
    assert receipts, f'no receipts under {results}/reads'
    sibling = []
    for path in receipts:
        r = json.loads(path.read_text())
        assert r['code'] == 0
        assert r['command'][:3] == ['taskset', '-c', '8'], r['command']
        sibling.append(r['cpu9_busy_percent'])
        c = json.loads(path.with_name(path.name.replace('.receipt.json', '.config.json')).read_text())
        assert c['action'] in ['read', 'miss'], path.name
        fields = r['name'].split('-')
        trial = int(next(v[1:] for v in fields if v.startswith('r') and v[1:].isdigit()))
        label = fields[-2]
        for stage in r['stages']:
            assert stage['errors'] == 0 and stage['latency']['count'] == c['operations'], path.name
            group = (c['layout'], c['backend'], str(c['cache_mib']), str(c['point_index']), stage['workload'])
            assert trial not in groups[group][label], (group, trial)
            groups[group][label][trial] = stage
    rows = []
    for group, sides in sorted(groups.items()):
        assert set(sides) == {'before', 'after'}, group
        before, after = sides['before'], sides['after']
        assert set(before) == set(after) == set(range(1, 6)), group
        a_, b_ = list(after.values()), list(before.values())
        ratios = [after[t]['call_seconds'] / before[t]['call_seconds'] for t in range(1, 6)]
        rows.append(dict(group=group,
                         before_p50=median_range([r['latency']['p50_ns'] for r in b_], 1000),
                         after_p50=median_range([r['latency']['p50_ns'] for r in a_], 1000),
                         before_p99=median_range([r['latency']['p99_ns'] for r in b_], 1000),
                         after_p99=median_range([r['latency']['p99_ns'] for r in a_], 1000),
                         paired_ratio=median_range(ratios), ratios=ratios))
    by_group = {tuple(r['group']): r for r in rows}
    for r in rows:
        if r['group'][1] == 'kiban':
            control = by_group[(r['group'][0], 'lmdb', *r['group'][2:])]
            r['control_adjusted_ratio'] = median_range([a / b for a, b in zip(r['ratios'], control['ratios'])])
        else:
            r['control_adjusted_ratio'] = '—'
    out_rows = [{**{k: v for k, v in r.items() if k != 'ratios'}, 'group': '/'.join(r['group'])} for r in rows]
    assert out_rows, 'no paired rows'
    destination = a.md
    if destination:
        text = ['# Filter before point index on misses'] + []
        header = '| Layout | Backend | Cache MiB | PH3 | Workload | Before p50 µs | After p50 µs | Before p99 µs | After p99 µs | Paired call-time ratio | Relative to paired LMDB |'
        text += ['Five paired trials, alternating before/after order; every Kiban side followed by the identical LMDB control in the same trial. Values are median [min, max] of five per-run statistics. Latency units: µs. Paired ratio uses total timed call seconds, matched by trial. Every process returned success, consumed every result and checked its expected work counts.', '',
                 f'CPU 9 busy percentage over the timing processes: {median_range(sibling)}. Timing children were pinned to CPU 8; sibling accounting covers each whole process, including verification.', '',
                 header, '|---|---|---|---|---|---:|---:|---:|---:|---:|---:|']
        for r in out_rows:
            group = r['group'].split('/')
            group[2] = 'default' if group[2] == 'None' else group[2]
            group[3] = 'on' if group[3] == 'True' else 'off'
            text.append('| ' + ' | '.join(group + [r[k] for k in ['before_p50', 'after_p50', 'before_p99', 'after_p99', 'paired_ratio', 'control_adjusted_ratio']]) + ' |')
        with destination.open('w') as f:
            f.write('\n'.join(text) + '\n')
    with (results / 'paired-summary.csv').open('w') as f:
        writer = csv.DictWriter(f, fieldnames=list(out_rows[0].keys()))
        writer.writeheader()
        writer.writerows(out_rows)
    print(f'Validated {len(receipts)} paired read/miss receipts across {len(rows)} cells.'
          f' CPU 9 sibling median {st.median(sibling):.3f}%.')


if __name__ == '__main__':
    main()
