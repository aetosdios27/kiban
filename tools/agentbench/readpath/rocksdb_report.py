#!/usr/bin/env python3
"""Independently audit all campaign receipts and derive tables from raw evidence."""
import collections
import csv
import hashlib
import json
from pathlib import Path
import statistics
import sys

ROOT = Path(__file__).resolve().parents[3]
RAW = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'validation/rocksdb/raw'
OUT = RAW.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stat(values):
    if all(v is None for v in values):
        return dict(median=None,min=None,max=None,values=values)
    assert all(v is not None for v in values)
    return dict(median=statistics.median(values), min=min(values), max=max(values), values=values)


def fmt(s, scale=1, digits=3, ranges=True):
    s = {k: v/scale for k,v in s.items() if k != 'values'}
    return f'{s["median"]:.{digits}f}' + (f' [{s["min"]:.{digits}f}, {s["max"]:.{digits}f}]' if ranges else '')


def jsonlines(path):
    return list(map(json.loads, path.read_text().splitlines()))


summary = {}
trials = []
commands = []
fixture_layouts = {}
default_budgets = set()
for phase in ('screen', 'measure'):
    tables = []
    resources = []
    for folder in sorted((RAW / phase).iterdir()):
        if not folder.is_dir():
            continue
        receipts = jsonlines(folder/'commands.jsonl')
        names = [r['name'] for r in receipts]
        assert len(names) == len(set(names))
        first = [n.split('-',1)[1] for n in names if n.startswith('r1-')]
        if phase == 'screen':
            expected = set(json.loads((RAW/'metadata.json').read_text())['candidates']) | {'lmdb'}
        else:
            expected = {'kiban-index0','kiban-index1','rocksdb-tuned','rocksdb-lz4','lmdb'}
            if folder.name.startswith('write') and folder.name.endswith('-fast'):
                expected.add('rocksdb-no-wal')
        assert set(first) == expected, 'missing/extra engine rows'
        assert len(receipts) == 5*len(expected)
        for i in range(5):
            actual = [n.split('-',1)[1] for n in names if n.startswith(f'r{i+1}-')]
            shift = i % len(first)
            assert actual == first[shift:] + first[:shift], f'rotation: {folder}'
        values = collections.defaultdict(dict)
        for command in receipts:
            trial, label = command['name'].split('-',1)
            assert sha(folder/command['config']) == command['config_sha256']
            assert sha(folder/(command['name']+'.log')) == command['log_sha256']
            assert sha(folder/(command['name']+'.time.txt')) == command['time_sha256']
            config = json.loads((folder/command['config']).read_text())
            result = folder/(command['name']+'.jsonl')
            assert sha(result) == command['result_sha256']
            rows = jsonlines(result)
            stages = [r for r in rows if r['type'] == 'stage']
            assert all(r.get('errors',0) == 0 for r in rows)
            assert len(stages) == (2 if config['action'] == 'read' else 5)
            assert any(r['type'] == 'complete' for r in rows)
            if config['action'] == 'write':
                assert any(r['type'] == 'reopen_verified' for r in rows)
            status = next(r['proc_status'] for r in rows if 'proc_status' in r)
            assert next(s.split(':',1)[1].strip() for s in status.splitlines() if s.startswith('Cpus_allowed_list:')) == '10'
            log = (folder/(command['name']+'.log')).read_text()
            logged = [json.loads(s) for s in log.splitlines() if s.startswith('{')]
            requested = [r for r in logged if r.get('type') == 'requested_config']
            assert requested
            expected_cache = config['cache_mib']*1024**2 if config['cache_mib'] is not None else requested[0]['cache_bytes']
            assert all(r['cache_bytes'] == expected_cache and r['durable'] == config['durable'] for r in requested)
            if config['cache_mib'] is None:
                default_budgets.add(expected_cache)
            if config['backend'] == 'rocksdb':
                configs = [r for r in logged if r.get('type') == 'backend_config']
                assert configs and all(r['version'] == '11.8.1' and r['cache_bytes'] == expected_cache and r['sync'] == config['durable'] for r in configs)
                for c in configs:
                    assert c['state']['rocksdb.block-cache-capacity'] == expected_cache
                    for k,v in config['rocks'].items():
                        assert c['tuning'][k] == v
                    if config['action'] == 'read':
                        files = sorted(c['state']['live_files'], key=lambda f:f['name'])
                        previous = fixture_layouts.setdefault(config['db'],files)
                        assert previous == files, 'RocksDB read fixture layout changed across trials'
                        assert all(c['state'][k] == 0 for k in ('rocksdb.num-running-compactions','rocksdb.num-running-flushes','rocksdb.compaction-pending','rocksdb.mem-table-flush-pending','rocksdb.estimate-pending-compaction-bytes'))
            if config['backend'] == 'kiban' and config['action'] == 'read':
                layout = next(r['layout'] for r in rows if r['type'] == 'complete')
                previous = fixture_layouts.setdefault(config['db'],layout)
                assert previous == layout, 'Kiban read fixture layout changed across trials'
            if config['backend'] == 'lmdb':
                configs = [r for r in logged if r.get('type') == 'backend_config']
                assert configs
                assert all(('NO_SYNC' in r['details']['lmdb_flags']) == (not config['durable']) for r in configs)
            rss_text = (folder/(command['name']+'.time.txt')).read_text()
            rss = int(next(s.split(':',1)[1] for s in rss_text.splitlines() if 'Maximum resident set size' in s))
            for row in stages:
                assert row['latency']['count'] == (100000 if config['action'] == 'read' else 1000)
                data = dict(p50_ns=row['latency']['p50_ns'],p99_ns=row['latency']['p99_ns'],max_ns=row['latency']['max_ns'],read_p50_ns=row['reads']['p50_ns'],read_p99_ns=row['reads']['p99_ns'],write_p50_ns=row['writes']['p50_ns'],write_p99_ns=row['writes']['p99_ns'],call_ops_s=row['call_ops_s'],verified_ops_s=row['verified_ops_s'],count=row['latency']['count'],rss_kib=rss,sst_bytes=sum(size for name,size in command['disk']['files'].items() if name.endswith('.sst')),logical_bytes=command['disk']['logical_bytes'],allocated_bytes=command['disk']['allocated_bytes'])
                values[label,row['workload']][trial] = data
                trials.append(dict(phase=phase,scenario=folder.name,engine=label,trial=trial,workload=row['workload'],**data))
            commands.append(dict(phase=phase,scenario=folder.name,**command))
        assert all(set(v) == {f'r{i}' for i in range(1,6)} for v in values.values())
        grouped = {}
        for (label,workload), data in values.items():
            metrics = {k:stat([data[f'r{i}'][k] for i in range(1,6)]) for k in next(iter(data.values()))}
            ratios = {}
            for comparator in ('rocksdb-tuned','lmdb'):
                if (comparator,workload) in values:
                    ratios[comparator] = {k:stat([data[f'r{i}'][k]/values[comparator,workload][f'r{i}'][k] for i in range(1,6)]) for k in ('p50_ns','p99_ns','call_ops_s')}
            grouped.setdefault(workload,{})[label] = dict(metrics=metrics,ratios=ratios)
        summary.setdefault(phase,{})[folder.name] = grouped
        tables += [f'## {folder.name}', '', '| Operation | Engine | p50 µs median [min,max] | p99 µs median [min,max] | Call kops/s median [min,max] | Verified kops/s median [min,max] | / Rocks p50 / p99 | / LMDB p50 / p99 |', '|---|---|---:|---:|---:|---:|---:|---:|']
        for work, engines in grouped.items():
            for label, result in engines.items():
                m = result['metrics']
                ratios = [' / '.join(fmt(result['ratios'][cmp][k],ranges=False) for k in ('p50_ns','p99_ns')) if cmp in result['ratios'] else '—' for cmp in ('rocksdb-tuned','lmdb')]
                tables.append('| '+' | '.join([work,label,fmt(m['p50_ns'],1000),fmt(m['p99_ns'],1000),fmt(m['call_ops_s'],1000),fmt(m['verified_ops_s'],1000)]+ratios)+' |')
        tables.append('')
        resources += [f'## {folder.name}', '', '| Engine | Logical MiB [min,max] | Allocated MiB [min,max] | Peak RSS MiB [min,max] |', '|---|---:|---:|---:|']
        for label, result in next(iter(grouped.values())).items():
            m = result['metrics']
            resources.append('| '+' | '.join([label,fmt(m['logical_bytes'],1024**2),fmt(m['allocated_bytes'],1024**2),fmt(m['rss_kib'],1024)])+' |')
        resources.append('')
    (OUT/(phase.upper()+'.md')).write_text('\n'.join(tables).rstrip()+'\n')
    (OUT/(phase.upper()+'-RESOURCES.md')).write_text('\n'.join(resources).rstrip()+'\n')

(OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
with (OUT/'TRIALS.csv').open('w') as f:
    writer = csv.DictWriter(f,fieldnames=list(trials[0]),lineterminator='\n');writer.writeheader();writer.writerows(trials)

def sibling(command):
    d = command['cpu_tick_delta']['cpu11']
    total = sum(d[:8])
    return 100*(total-d[3]-d[4])/total if total else 0

primary = [c for c in commands if c['phase'] == 'measure']
assert len(default_budgets) == 1, 'default cache capacity changed between engines or commands'
assert len(commands) == 425 and len(primary) == 155
assert sum(t['count'] for t in trials) == 56725000
audit = dict(default_cache_bytes=next(iter(default_budgets)),commands=len(commands),timed_calls=sum(t['count'] for t in trials),errors=0,primary_commands=len(primary),primary_timed_calls=sum(t['count'] for t in trials if t['phase']=='measure'),sibling11_busy_percent=stat([sibling(c) for c in primary]),load1=stat([c['load'][0] for c in primary]),all_affinities='10',normal_rocksdb_pending_compaction_bytes=0)
meta = json.loads((RAW/'metadata.json').read_text())
assert all(sha(ROOT/p) == h for p,h in meta['engine_sources'].items()), 'engine sources changed'
assert all(sha(ROOT/p) == h for p,h in meta['harness_sources'].items()), 'frozen harness changed'
assert sha(Path(meta['binary'])) == meta['binary_sha256']
(OUT/'fixture-layouts.json').write_text(json.dumps(fixture_layouts,indent=2)+'\n')
(OUT/'audit.json').write_text(json.dumps(audit,indent=2)+'\n')
print(json.dumps({k:v for k,v in audit.items() if k not in ('sibling11_busy_percent','load1')},indent=2))
print('Sibling busy median/range', {k:v for k,v in audit['sibling11_busy_percent'].items() if k!='values'})

# Concise matrices for the narrative report; every number derives from summary above.
selection = json.loads((RAW/'selection.json').read_text())
lines = ['## Frozen screening choice', '', '| Complete configuration | Read score µs | Write score µs | Selected family |', '|---|---:|---:|---|']
for label in sorted(selection['scores_seconds']['read']):
    selected = ', '.join(k for k in ('read','write') if selection[k] == label) or ''
    lines.append(f'| {label} | {selection["scores_seconds"]["read"][label]*1e6:.3f} | {selection["scores_seconds"]["write"][label]*1e6:.3f} | {selected} |')
lines += ['', 'Scores are geometric aggregates of mean call times, not public-operation latencies. All losing rows and ranges are in SCREEN.md.', '', '## 1M reads', '', 'Cells are **p50 µs / p99 µs / call kops/s**, each the median of five trials.', '', '| Layout/cache | Operation | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | LMDB |', '|---|---|---:|---:|---:|---:|---:|']
read_labels = ['kiban-index0','kiban-index1','rocksdb-tuned','rocksdb-lz4','lmdb']
write_labels = ['kiban-index0','kiban-index1','rocksdb-tuned','rocksdb-lz4','rocksdb-no-wal','lmdb']

def triple(data):
    m = data['metrics']
    return f'{m["p50_ns"]["median"]/1000:.3f} / {m["p99_ns"]["median"]/1000:.3f} / {m["call_ops_s"]["median"]/1000:.1f}'

for shape in ('bulk','normal'):
    for cache in (512,None):
        group = summary['measure'][f'read-1000000-{shape}-{cache}-fast']
        for work in ('recall-by-id','recent-for-agent'):
            lines.append('| '+' | '.join([f'{shape}/{cache if cache else "default"}',work]+[triple(group[work][k]) for k in read_labels])+' |')
lines += ['', '## 10k writes', '', 'Same cell convention. RocksDB fast keeps WAL enabled; no-WAL explicitly removes that acknowledgment guarantee.', '', '| Mode | Operation | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | RocksDB no-WAL | LMDB |', '|---|---|---:|---:|---:|---:|---:|---:|']
for mode in ('fast','durable'):
    group = summary['measure'][f'write-10000-bulk-None-{mode}']
    for work in ('remember','update','mixed-90-10'):
        lines.append('| '+' | '.join([mode,work]+[triple(group[work][k]) if k in group[work] else '—' for k in write_labels])+' |')
lines += ['', '## Paired Kiban latency ratios', '', 'Each cell is **p50 ratio / p99 ratio**, Kiban divided by its same-round comparator, then median across five rounds. Below 1 favors Kiban.', '', '| Layout/cache | Operation | Index | / RocksDB tuned | / LMDB |', '|---|---|---|---:|---:|']
for scenario,group in summary['measure'].items():
    for work in ('recall-by-id','recent-for-agent') if scenario.startswith('read') else ('remember','update','mixed-90-10'):
        for index in (0,1):
            r = group[work][f'kiban-index{index}']['ratios']
            ratios = [' / '.join(f'{r[c][k]["median"]:.3f}' for k in ('p50_ns','p99_ns')) for c in ('rocksdb-tuned','lmdb')]
            lines.append('| '+' | '.join([scenario.replace('read-1000000-','').replace('write-10000-',''),work,str(index)]+ratios)+' |')
lines += ['', '## 1M disk and memory', '', 'Cells are **logical disk MiB / whole-process peak RSS MiB**, median of five runs. Allocated bytes and ranges are in MEASURE-RESOURCES.md.', '', '| Layout/cache | Kiban index off | Kiban index on | RocksDB tuned | RocksDB LZ4 | LMDB |', '|---|---:|---:|---:|---:|---:|']
for shape in ('bulk','normal'):
    for cache in (512,None):
        group = summary['measure'][f'read-1000000-{shape}-{cache}-fast']['recall-by-id']
        vals = [f'{group[k]["metrics"]["logical_bytes"]["median"]/1024**2:.3f} / {group[k]["metrics"]["rss_kib"]["median"]/1024:.3f}' for k in read_labels]
        lines.append('| '+' | '.join([f'{shape}/{cache if cache else "default"}']+vals)+' |')
(OUT/'RESULTS.md').write_text('\n'.join(lines)+'\n')

page = ['<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:var(--font-sans,system-ui);color:var(--foreground,#20242b);margin:0}table{width:100%;border-collapse:collapse;font-size:12px;margin:8px 0 20px}caption{text-align:left;font-weight:650;font-size:15px;margin:8px 0}th,td{border-bottom:1px solid var(--border,#d5d9df);padding:8px 4px;text-align:right;vertical-align:top}th{font-weight:600}th:first-child,td:first-child{text-align:left}small{color:var(--muted-foreground,#666);font-size:11px}p{font-size:12px;line-height:1.45;margin:8px 0}td span{white-space:nowrap}td b{font-weight:550}tbody tr:nth-child(even){background:var(--secondary,#f6f7f9)}.scroll{overflow-x:auto}</style></head><body>', '<p>Per cell, top to bottom: <b>p50 µs / p99 µs / call kops/s</b>. Medians of five rotated trials; all timed results verified. Full ranges, paired ratios, resources, and compaction states are in the report.</p>']

def html_cell(result):
    if result is None:
        return '<td>—</td>'
    m = result['metrics']
    return f'<td><b>{m["p50_ns"]["median"]/1000:.3f}</b><br><span>{m["p99_ns"]["median"]/1000:.3f}</span><br><small>{m["call_ops_s"]["median"]/1000:.1f}</small></td>'

page += ['<div class="scroll"><table><caption>1M warm reads</caption><thead><tr><th>Scenario / call</th><th>Kiban off</th><th>Kiban on</th><th>Rocks tuned</th><th>Rocks LZ4</th><th>LMDB</th></tr></thead><tbody>']
for shape in ('bulk','normal'):
    for cache in (512,None):
        group = summary['measure'][f'read-1000000-{shape}-{cache}-fast']
        for work in ('recall-by-id','recent-for-agent'):
            label = 'ID' if work == 'recall-by-id' else 'Latest 10'
            page.append(f'<tr><td>{shape}, {cache if cache else "default"}<br><small>{label}</small></td>'+''.join(html_cell(group[work][k]) for k in read_labels)+'</tr>')
page += ['</tbody></table></div>', '<div class="scroll"><table><caption>10k fixture writes</caption><thead><tr><th>Mode / call</th><th>Kiban off</th><th>Kiban on</th><th>Rocks tuned</th><th>Rocks LZ4</th><th>No WAL</th><th>LMDB</th></tr></thead><tbody>']
for mode in ('fast','durable'):
    group = summary['measure'][f'write-10000-bulk-None-{mode}']
    for work in ('remember','update','mixed-90-10'):
        page.append(f'<tr><td>{mode}<br><small>{work}</small></td>'+''.join(html_cell(group[work].get(k)) for k in write_labels)+'</tr>')
page += ['</tbody></table></div>', '<p>RocksDB uses the separately frozen read/write winners. Kiban on/off refers to the perfect-hash point index. Fast acknowledgment guarantees differ; no-WAL has no per-write durability. These are warm, single-worker, shared-host measurements; CPU 11 background activity prevents an exclusive-core claim.</p>', '</body></html>']
(OUT/'RESULTS.html').write_text(''.join(page))

footprint = ['| Layout/cache | Kiban off SST MiB | Kiban on SST MiB | RocksDB tuned SST MiB | RocksDB LZ4 SST MiB |', '|---|---:|---:|---:|---:|']
for shape in ('bulk','normal'):
    for cache in (512,None):
        group=summary['measure'][f'read-1000000-{shape}-{cache}-fast']['recall-by-id']
        footprint.append('| '+' | '.join([f'{shape}/{cache if cache else "default"}']+[f'{group[k]["metrics"]["sst_bytes"]["median"]/1024**2:.3f}' for k in read_labels[:-1]])+' |')
(OUT/'SST-FOOTPRINT.md').write_text('\n'.join(footprint)+'\n')

completion = collections.defaultdict(list)
for folder in (RAW/'measure').glob('write-*'):
    for command in jsonlines(folder/'commands.jsonl'):
        result=jsonlines(folder/(command['name']+'.jsonl'))
        value=next(r['end_sync_flush_verify_seconds'] for r in result if r['type']=='complete')
        completion[folder.name,command['name'].split('-',1)[1]].append(value)
lines=['| Scenario | Engine | Completion seconds median [min,max] |','|---|---|---:|']
for (scenario,label),values in sorted(completion.items()):
    assert len(values)==5
    lines.append(f'| {scenario} | {label} | {statistics.median(values):.4f} [{min(values):.4f}, {max(values):.4f}] |')
(OUT/'COMPLETION.md').write_text('\n'.join(lines)+'\n')
