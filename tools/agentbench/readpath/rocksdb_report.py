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
                data = dict(p50_ns=row['latency']['p50_ns'],p99_ns=row['latency']['p99_ns'],max_ns=row['latency']['max_ns'],call_ops_s=row['call_ops_s'],verified_ops_s=row['verified_ops_s'],count=row['latency']['count'],rss_kib=rss,logical_bytes=command['disk']['logical_bytes'],allocated_bytes=command['disk']['allocated_bytes'])
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
    (OUT/(phase.upper()+'.md')).write_text('\n'.join(tables)+'\n')
    (OUT/(phase.upper()+'-RESOURCES.md')).write_text('\n'.join(resources)+'\n')

(OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
with (OUT/'TRIALS.csv').open('w') as f:
    writer = csv.DictWriter(f,fieldnames=list(trials[0]));writer.writeheader();writer.writerows(trials)

def sibling(command):
    d = command['cpu_tick_delta']['cpu11']
    total = sum(d[:8])
    return 100*(total-d[3]-d[4])/total if total else 0

primary = [c for c in commands if c['phase'] == 'measure']
audit = dict(commands=len(commands),timed_calls=sum(t['count'] for t in trials),errors=0,primary_commands=len(primary),primary_timed_calls=sum(t['count'] for t in trials if t['phase']=='measure'),sibling11_busy_percent=stat([sibling(c) for c in primary]),load1=stat([c['load'][0] for c in primary]),all_affinities='10',normal_rocksdb_pending_compaction_bytes=0)
meta = json.loads((RAW/'metadata.json').read_text())
assert all(sha(ROOT/p) == h for p,h in meta['engine_sources'].items()), 'engine sources changed'
assert all(sha(ROOT/p) == h for p,h in meta['harness_sources'].items()), 'frozen harness changed'
assert sha(Path(meta['binary'])) == meta['binary_sha256']
(OUT/'fixture-layouts.json').write_text(json.dumps(fixture_layouts,indent=2)+'\n')
(OUT/'audit.json').write_text(json.dumps(audit,indent=2)+'\n')
print(json.dumps({k:v for k,v in audit.items() if k not in ('sibling11_busy_percent','load1')},indent=2))
print('Sibling busy median/range', {k:v for k,v in audit['sibling11_busy_percent'].items() if k!='values'})
