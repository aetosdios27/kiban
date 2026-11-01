#!/usr/bin/env python3
"""Audit every receipt and generate all comparison tables from raw data."""
import collections
import json
from pathlib import Path
import statistics
import sys

ROOT = Path(__file__).resolve().parents[3]
raw = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'validation/sqlite/raw'
out = raw.parent
trial_rows = []


def stats(values):
    if all(v is None for v in values):
        return dict(median=None, min=None, max=None, values=values)
    assert all(v is not None for v in values)
    return dict(median=statistics.median(values), min=min(values), max=max(values), values=values)


def cell(s, scale=1, digits=3, ranges=True):
    mid = f"{s['median']/scale:.{digits}f}"
    return mid + (f" [{s['min']/scale:.{digits}f}, {s['max']/scale:.{digits}f}]" if ranges else '')


def load(folder):
    entries = {}
    commands = [json.loads(s) for s in (folder / 'commands.jsonl').read_text().splitlines()]
    first_order = [c['name'].split('-',1)[1] for c in commands if c['name'].startswith('r1-')]
    for i in range(5):
        actual = [c['name'].split('-',1)[1] for c in commands if c['name'].startswith(f'r{i+1}-')]
        shift = i % len(first_order)
        assert actual == first_order[shift:] + first_order[:shift], 'engine order was not rotated'
    for c in commands:
        trial, label = c['name'].split('-', 1)
        path = folder / (c['name'] + '.jsonl')
        import hashlib
        assert hashlib.sha256(path.read_bytes()).hexdigest() == c['result_sha256']
        config = json.loads((folder / c['config']).read_text())
        rows = list(map(json.loads, path.read_text().splitlines()))
        assert len([r for r in rows if r['type'] == 'stage']) == (2 if config['action'] == 'read' else 5)
        assert any(r['type'] == 'complete' for r in rows)
        assert all(r.get('errors', 0) == 0 for r in rows)
        requested = [json.loads(s) for s in (folder / (c['name'] + '.log')).read_text().splitlines()
                     if s.startswith('{') and 'requested_config' in s]
        expected_budget = config['cache_mib']*1024**2 if config['cache_mib'] is not None else 449030592
        assert requested and all(v['cache_bytes'] == expected_budget for v in requested)
        status = next(r['proc_status'] for r in rows if 'proc_status' in r)
        affinity = next(line.split(':', 1)[1].strip() for line in status.splitlines()
                        if line.startswith('Cpus_allowed_list:'))
        assert affinity == '10', f'wrong measured process affinity: {affinity}'
        expected_count = 100000 if config['action'] == 'read' else 1000
        assert all(r['latency']['count'] == expected_count for r in rows if r['type'] == 'stage')
        if config['backend'] == 'sqlite':
            configs = [json.loads(s) for s in (folder / (c['name'] + '.log')).read_text().splitlines()
                       if s.startswith('{') and 'backend_config' in s]
            assert configs and all(v['sqlite_version'] == '3.53.2' for v in configs)
            for v in configs:
                p = v['pragmas']
                assert p['synchronous'] == {'OFF':0, 'NORMAL':1, 'FULL':2}[config['sqlite_sync']]
                assert p['journal_mode'] == 'wal' and p['mmap_size'] == config['sqlite_mmap_bytes']
                assert v['layout'] == config['sqlite_layout']
                assert all('SEARCH' in plan for group in v['query_plans'] for plan in group)
        if config['backend'] == 'lmdb':
            configs = [json.loads(s) for s in (folder / (c['name'] + '.log')).read_text().splitlines()
                       if s.startswith('{') and 'backend_config' in s]
            assert configs
            for v in configs:
                flags = v['details']['lmdb_flags']
                assert ('NO_SYNC' in flags) == (not config['durable'])
                assert ('NO_META_SYNC' in flags) == (not config['durable'])
        if config['action'] == 'write':
            assert any(r['type'] == 'reopen_verified' for r in rows)
        log = (folder / (c['name'] + '.time.txt')).read_text()
        rss = int(next(s.split(':', 1)[1] for s in log.splitlines() if 'Maximum resident set size' in s))
        for r in rows:
            if r['type'] == 'stage':
                entries.setdefault((label, r['workload']), {})[trial] = dict(
                    p50_ns=r['latency']['p50_ns'], p99_ns=r['latency']['p99_ns'],
                    call_ops_s=r['call_ops_s'], verified_ops_s=r['verified_ops_s'],
                    read_p50_ns=r['reads']['p50_ns'], read_p99_ns=r['reads']['p99_ns'],
                    write_p50_ns=r['writes']['p50_ns'], write_p99_ns=r['writes']['p99_ns'],
                    count=r['latency']['count'], rss_kib=rss,
                    logical_bytes=c['disk']['logical_bytes'], allocated_bytes=c['disk']['allocated_bytes'])
                trial_rows.append(dict(phase=folder.parent.name, scenario=folder.name,
                    engine=label, trial=trial, workload=r['workload'],
                    **entries[label,r['workload']][trial]))
    assert all(set(v) == {f'r{i}' for i in range(1,6)} for v in entries.values())
    assert len(commands) == len({c['name'] for c in commands})
    return entries, commands


summary = {}
all_commands = []
for phase in ('screen', 'mmap-screen', 'measure', 'fdatasync'):
    tables = []
    resources = []
    if not (raw / phase).exists():
        continue
    for folder in sorted((raw / phase).iterdir()):
        if not folder.is_dir(): continue
        data, commands = load(folder)
        all_commands.extend(commands)
        sqlite_baseline = 'sqlite-full' if folder.name.endswith('durable') else 'sqlite-normal'
        scenario = {}
        tables += [f'## {folder.name}', '',
            '| Workload | Engine | p50 µs [min,max] | p99 µs [min,max] | Call ops/s | Verified ops/s | p50 / SQLite | p99 / SQLite | p50 / LMDB | p99 / LMDB |',
            '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|']
        for (label, work), trials in sorted(data.items(), key=lambda item:(item[0][1], item[0][0])):
            s = {field:stats([trials[f'r{i}'][field] for i in range(1,6)]) for field in next(iter(trials.values()))}
            ratios = {}
            for base, comp in [('sqlite',sqlite_baseline), ('sqlite_mmap','sqlite-mmap'), ('lmdb','lmdb')]:
                if (comp, work) in data:
                    ratios[base] = {field:stats([trials[f'r{i}'][field] / data[comp, work][f'r{i}'][field]
                                     for i in range(1,6)]) for field in ('p50_ns','p99_ns','call_ops_s')}
            s['ratios'] = ratios
            scenario[label + '/' + work] = s
            r = [cell(ratios[base][field], digits=3, ranges=False) if base in ratios else '—'
                 for base in ('sqlite','lmdb') for field in ('p50_ns','p99_ns')]
            tables += ['| ' + ' | '.join([work, label, cell(s['p50_ns'],1000),cell(s['p99_ns'],1000),
                        cell(s['call_ops_s'],digits=0,ranges=False),cell(s['verified_ops_s'],digits=0,ranges=False)] + r) + ' |']
        tables += ['', 'Ratios pair trial numbers before taking the median. Lower latency ratios are better.', '']
        resources += [f'## {folder.name}', '', '| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |',
                      '|---|---:|---:|---:|']
        for label in sorted({k[0] for k in data}):
            row = next(v for k,v in scenario.items() if k.split('/')[0] == label)
            resources += ['| ' + ' | '.join([label, cell(row['rss_kib'],1024),
                           cell(row['logical_bytes'],1024**2,ranges=False),
                           cell(row['allocated_bytes'],1024**2,ranges=False)]) + ' |']
        resources += ['']
        summary[phase + '/' + folder.name] = scenario
    (out / (phase.upper() + '.md')).write_text('# ' + phase.capitalize() + ' latency results\n\n' + '\n'.join(tables))
    (out / (phase.upper() + '-RESOURCES.md')).write_text('# ' + phase.capitalize() + ' resources\n\n' + '\n'.join(resources))

(out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
counts = dict(commands=len(all_commands), timed_calls=sum(
    sum(v['count']['values']) for scenario in summary.values() for v in scenario.values()), errors=0)
for phase in ('screen','mmap-screen','measure','fdatasync'):
    counts[phase + '_commands'] = sum(1 for p in (raw / phase).glob('*/r*.config.json'))
(out / 'audit.json').write_text(json.dumps(counts, indent=2)+'\n')
print(json.dumps(counts, indent=2))

import csv
with (out / 'TRIALS.csv').open('w', newline='') as f:
    writer = csv.DictWriter(f, fieldnames=list(trial_rows[0]), lineterminator='\n')
    writer.writeheader()
    writer.writerows(trial_rows)

# Compact tables for the report. Each cell keeps both latency and mean-based
# call throughput, since p50 can hide checkpoint stalls and durable mixed writes.
compact = ['# Comparison tables', '',
           'Cells are **p50 µs / p99 µs / call kops/s**, each the median of five trials. '
           'All ranges, verified throughput and paired ratios are in MEASURE.md, '
           'FDATASYNC.md and summary.json.', '', '## 1M reads', '',
           '| Layout/cache | Operation | Kiban index off | Kiban index on | SQLite page cache | SQLite mmap | LMDB |',
           '|---|---|---:|---:|---:|---:|---:|']


def triple(row):
    return f"{row['p50_ns']['median']/1000:.3f} / {row['p99_ns']['median']/1000:.3f} / {row['call_ops_s']['median']/1000:.1f}"


read_keys = sorted(k for k in summary if k.startswith('measure/read-'))
for key in read_keys:
    scenario=key.split('/',1)[1]
    shape,cache=scenario.split('-')[2:4]
    for work in ('recall-by-id','recent-for-agent'):
        compact += ['| '+' | '.join([f'{shape}/{"default" if cache=="None" else cache}',work]+[
            triple(summary[key][label+'/'+work]) for label in
            ('kiban-index0','kiban-index1','sqlite-normal','sqlite-mmap','lmdb')])+' |']
compact += ['', '## 10k writes', '',
            'NORMAL is relaxed in both groups. FULL is the durable SQLite comparison. '
            'Fast Kiban acknowledgments can be lost on process death.', '',
            '| Mode | Operation | Kiban index off | Kiban index on | SQLite OFF or FULL | SQLite NORMAL | LMDB |',
            '|---|---|---:|---:|---:|---:|---:|']
for mode in ('fast','durable'):
    key='measure/write-10000-bulk-None-'+mode
    sql='sqlite-off' if mode=='fast' else 'sqlite-full'
    for work in ('remember','update','mixed-90-10'):
        compact += ['| '+' | '.join([mode,work]+[triple(summary[key][label+'/'+work]) for label in
                    ('kiban-index0','kiban-index1',sql,'sqlite-normal','lmdb')])+' |']
fdkey='fdatasync/write-10000-bulk-None-durable'
if fdkey in summary:
    compact += ['', '## Linux fdatasync follow-up', '',
            '| Operation | Kiban index off | Kiban index on | SQLite FULL fsync | SQLite FULL fdatasync | LMDB |',
            '|---|---:|---:|---:|---:|---:|']
    for work in ('remember','update','mixed-90-10'):
        compact += ['| '+' | '.join([work]+[triple(summary[fdkey][label+'/'+work]) for label in
            ('kiban-index0','kiban-index1','sqlite-full','sqlite-full-fdatasync','lmdb')])+' |']
compact += ['', '## Paired 1M latency ratios', '',
            'Each cell is **p50 ratio / p99 ratio** for Kiban divided by its same-round comparator. '
            'Below 1 favors Kiban.', '',
            '| Layout/cache | Operation | Index | / SQLite page cache | / SQLite mmap | / LMDB |',
            '|---|---|---|---:|---:|---:|']
for key in read_keys:
    shape,cache=key.split('/')[1].split('-')[2:4]
    for work in ('recall-by-id','recent-for-agent'):
        for index in (0,1):
            row=summary[key][f'kiban-index{index}/'+work]
            compact += ['| '+' | '.join([f'{shape}/{"default" if cache=="None" else cache}',work,str(index)]+[
                f"{row['ratios'][base]['p50_ns']['median']:.3f} / {row['ratios'][base]['p99_ns']['median']:.3f}"
                for base in ('sqlite','sqlite_mmap','lmdb')])+' |']
compact += ['', '## 1M disk and memory', '',
            'Each cell is **logical disk MiB / peak RSS MiB**, median of five process runs. '
            'Full resource ranges and allocated bytes are in MEASURE-RESOURCES.md.', '',
            '| Layout/cache | Kiban index off | Kiban index on | SQLite page cache | SQLite mmap | LMDB |',
            '|---|---:|---:|---:|---:|---:|']
for key in read_keys:
    shape,cache=key.split('/')[1].split('-')[2:4]
    compact += ['| '+' | '.join([f'{shape}/{"default" if cache=="None" else cache}']+[
        f"{summary[key][label+'/recall-by-id']['logical_bytes']['median']/1024**2:.3f} / "
        f"{summary[key][label+'/recall-by-id']['rss_kib']['median']/1024:.3f}"
        for label in ('kiban-index0','kiban-index1','sqlite-normal','sqlite-mmap','lmdb')])+' |']
(out/'COMPARISON.md').write_text('\n'.join(compact)+'\n')
