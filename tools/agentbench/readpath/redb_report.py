#!/usr/bin/env python3
"""Audit redb comparison receipts and render the full results tables."""
import argparse
from collections import defaultdict
import csv
import json
from pathlib import Path
import statistics

from redb_bench import (
    CANDIDATES, CPU, ROOT, SCENARIOS, SEED, SIBLING, STUDY, TRIALS,
    choose, configuration, digest, fixture, read_jsonl, rotate, scenario_name,
)

READ_STAGES = ('recall-by-id', 'recent-for-agent')
WRITE_STAGES = ('remember', *READ_STAGES, 'update', 'mixed-90-10')
PRIMARY = ('kiban-index0', 'kiban-index1', 'redb', 'lmdb')
METRICS = ('p50_ns', 'p99_ns', 'call_ops_s', 'verified_ops_s')
RESOURCES = ('rss_kib', 'logical_bytes', 'allocated_bytes')


def stats(values):
    return dict(median=statistics.median(values), min=min(values), max=max(values), values=values)


def recorded_file(folder, receipt, role, suffix):
    path = folder / f'{receipt["name"]}.{suffix}'
    assert digest(path) == receipt[f'{role}_sha256'], path
    return path


def audit_command(folder, receipt, c, default_caches):
    assert receipt['affinity'] == str(CPU)
    assert set(receipt['cpu_tick_delta']) == {f'cpu{CPU}', f'cpu{SIBLING}'}
    assert not receipt['diagnostic']
    config = recorded_file(folder, receipt, 'config', 'config.json')
    expected_db = fixture(c).with_name('write-trial') if c['action'] == 'write' else fixture(c)
    observed = json.loads(config.read_text())
    assert Path(observed.pop('db')).name == expected_db.name
    output = Path(observed.pop('output'))
    assert output.name == f'{receipt["name"]}.jsonl' and output.parent.name == folder.name
    assert observed == {k: v for k, v in c.items() if k not in ('db', 'output')}, config
    log = recorded_file(folder, receipt, 'log', 'log').read_text()
    time = recorded_file(folder, receipt, 'time', 'time.txt').read_text()
    rows = read_jsonl(recorded_file(folder, receipt, 'result', 'jsonl'))
    assert all(r.get('errors', 0) == 0 for r in rows)
    write = c['action'] == 'write'
    stages = [r for r in rows if r['type'] == 'stage']
    assert tuple(r['workload'] for r in stages) == (WRITE_STAGES if write else READ_STAGES)
    assert all(r['latency']['count'] == c['operations'] for r in stages)
    assert all(r['reads']['count'] + r['writes']['count'] == c['operations'] for r in stages)
    assert sum(r['writes']['count'] for r in stages) == (2100 if write else 0)
    complete, = (r for r in rows if r['type'] == 'complete')
    assert complete['verified_records'] == c['n'] + (c['operations'] if write else 0)
    if write:
        reopened, = (r for r in rows if r['type'] == 'reopen_verified')
        assert reopened['verified_records'] == complete['verified_records']
    status, = (r['proc_status'] for r in rows if 'proc_status' in r)
    assert next(s.split(':', 1)[1].strip() for s in status.splitlines()
                if s.startswith('Cpus_allowed_list:')) == str(CPU)
    logged = [json.loads(s) for s in log.splitlines() if s.startswith('{')]
    requested = [r for r in logged if r['type'] == 'requested_config']
    assert len(requested) == (2 if write else 1)
    budget = c['cache_mib'] * 1024**2 if c['cache_mib'] is not None else requested[0]['cache_bytes']
    if c['cache_mib'] is None:
        default_caches.add(budget)
    assert all(r['cache_bytes'] == budget and r['durable'] == c['durable'] and
               r['point_index'] == c['point_index'] and r['backend'] == c['backend'] and
               not r['binary_fuse_filter'] for r in requested)
    actual = [r for r in logged if r['type'] == 'backend_config']
    if c['backend'] == 'redb':
        assert len(actual) == len(requested)
        for row in actual:
            assert row['version'] == '5.0.0'
            assert row['cache_bytes'] == budget
            assert row['tuning'] == c['redb']
            assert row['durability'] == ('Immediate' if c['durable'] else 'None')
    if c['backend'] == 'lmdb':
        assert len(actual) == len(requested)
        for row in actual:
            flags = row['details']['lmdb_flags']
            assert ('NO_SYNC' in flags) == ('NO_META_SYNC' in flags) == (not c['durable'])
            assert row['details']['map_size_bytes'] == 10 * 1024**3
    rss = int(next(s.split(':', 1)[1] for s in time.splitlines()
                   if 'Maximum resident set size' in s))
    return stages, complete.get('layout'), dict(
        rss_kib=rss, logical_bytes=receipt['disk']['logical_bytes'],
        allocated_bytes=receipt['disk']['allocated_bytes'])


def audit_preparation(raw):
    folder = raw / 'prepare'
    records = {}
    for receipt in read_jsonl(folder / 'commands.jsonl'):
        assert receipt['affinity'] == str(CPU)
        config = json.loads(recorded_file(folder, receipt, 'config', 'config.json').read_text())
        recorded_file(folder, receipt, 'time', 'time.txt')
        log = recorded_file(folder, receipt, 'log', 'log').read_text()
        rows = [json.loads(s) for s in log.splitlines() if s.startswith('{')]
        prepared, = (r for r in rows if r['type'] == 'prepared')
        assert prepared['verified_records'] == config['n']
        key = receipt['name']
        assert key == fixture(config).name and key not in records
        records[key] = dict(receipt['disk'], layout=prepared.get('layout'),
                            storage=[r for r in rows if r['type'] == 'redb_storage'])
    assert len(records) == 15
    return records


def busy_sibling(receipt):
    user, nice, system, idle, iowait, irq, softirq, steal, *_ = receipt['cpu_tick_delta'][f'cpu{SIBLING}']
    busy = sum((user, nice, system, irq, softirq, steal))
    total = busy + idle + iowait
    return 100 * busy / total if total else 0.0


def audit_phase(raw, phase, selection, default_caches, preparation):
    totals = dict(commands=0, timed_calls=0, timed_mutations=0, busy=[], load_1m=[])
    trials = []
    result = {}
    engines = (*CANDIDATES, 'lmdb') if phase == 'screen' else PRIMARY
    assert set(p.name for p in (raw / phase).iterdir()) == set(map(scenario_name, SCENARIOS))
    for scenario in SCENARIOS:
        action, n, shape, cache, durable = scenario
        title = scenario_name(scenario)
        folder = raw / phase / title
        receipts = read_jsonl(folder / 'commands.jsonl')
        expected = [(trial + 1, label) for trial in range(TRIALS)
                    for label in rotate(engines, trial)]
        assert [r['name'] for r in receipts] == [f'r{trial}-{label}' for trial, label in expected]
        groups = defaultdict(list)
        for (trial, label), receipt in zip(expected, receipts):
            engine = 'redb' if label in CANDIDATES or label == 'redb' else label.split('-')[0]
            tuning = selection[action] if label == 'redb' else label
            c = dict(configuration(engine, n, shape, cache, tuning),
                     action=action, durable=durable, operations=100000 if action == 'read' else 1000)
            stages, layout, resources = audit_command(folder, receipt, c, default_caches)
            if action == 'read':
                prepared = preparation[fixture(c).name]
                assert layout == prepared['layout'], 'fixture layout changed'
                if c['backend'] == 'redb':
                    # Even a read-only workload closes a read-write handle.
                    # redb persists the allocator and can trim free tail pages.
                    sizes = prepared.setdefault('read_logical_bytes', [])
                    if resources['logical_bytes'] not in sizes:
                        sizes.append(resources['logical_bytes'])
                else:
                    assert resources['logical_bytes'] == prepared['logical_bytes']
            totals['commands'] += 1
            totals['busy'].append(busy_sibling(receipt))
            totals['load_1m'].append(receipt['load'][0])
            for row in stages:
                count = row['latency']['count']
                totals['timed_calls'] += count
                totals['timed_mutations'] += row['writes']['count']
                res = dict(p50_ns=row['latency']['p50_ns'], p99_ns=row['latency']['p99_ns'],
                           call_ops_s=row['call_ops_s'], verified_ops_s=row['verified_ops_s'],
                           reads_p50_ns=row['reads']['p50_ns'], reads_p99_ns=row['reads']['p99_ns'],
                           writes_p50_ns=row['writes']['p50_ns'], writes_p99_ns=row['writes']['p99_ns'],
                           count=count, **resources)
                groups[label, row['workload']].append(res)
                trials.append(dict(phase=phase, scenario=title, trial=trial,
                                   engine=label, workload=row['workload'], **res))
        result[title] = {}
        for (label, workload), data in groups.items():
            values = {k: stats([d[k] for d in data]) for k in (*METRICS, *RESOURCES)}
            if phase == 'measure':
                values['paired'] = {}
                for baseline in ('redb', 'lmdb'):
                    values['paired'][baseline] = {
                        key: stats([(b[key] / own[key]) if key.endswith('_ns')
                                    else (own[key] / b[key])
                                    for own, b in zip(data, groups[baseline, workload])])
                        for key in METRICS
                    }
            result[title].setdefault(workload, {})[label] = values
    totals['sibling_busy_percent'] = stats(totals.pop('busy'))
    totals['host_load_1m'] = stats(totals.pop('load_1m'))
    return result, trials, totals


def audit_checks(out):
    checks = out / 'checks'
    syncs = {}
    for engine in ('kiban', 'redb', 'lmdb'):
        for mode in ('fast', 'durable'):
            label = f'{engine}-{mode}'
            lines = (checks / f'{label}.strace.txt').read_text().splitlines()
            counts = {}
            for line in lines:
                parts = line.split()
                if parts and parts[-1] in ('fsync', 'fdatasync', 'msync'):
                    assert len(parts) == 5, f'failure or malformed syscall receipt: {line}'
                    counts[parts[-1]] = int(parts[3])
            syncs[label] = counts
        delta = sum(syncs[f'{engine}-durable'].values()) - sum(syncs[f'{engine}-fast'].values())
        # redb's fast-mode final checkpoint adds two syncs after the timed stages.
        assert delta == (2098 if engine == 'redb' else 2100), (engine, delta)
    receipt = json.loads((checks / 'crash-receipt.json').read_text())
    assert receipt['signal'] == 'SIGKILL' and receipt['wait_code'] == -9
    assert receipt['affinity'] == str(CPU) and receipt['acknowledgments'] == 32
    acks = [s for s in (checks / 'crash-write.log').read_text().splitlines() if s.startswith('ACK ')]
    assert acks == [f'ACK {n}' for n in range(10000, 10032)]
    reopened, = (r for r in read_jsonl(checks / 'crash-verify.log')
                 if r['type'] == 'reopen_verified')
    assert reopened['verified_records'] == receipt['verified_records'] == 10032
    result = dict(sync_syscalls=syncs, crash=receipt)
    windows_path = checks / 'redb-commit-windows.json'
    if windows_path.is_file():
        windows = json.loads(windows_path.read_text())
        assert all(record['completed'] and record['affinity'] == str(CPU)
                   for record in windows.values())
        observed = {mode: row['fdatasync_calls'] for mode, row in windows.items()}
        expected = dict(zip((*WRITE_STAGES, 'after_mixed'), (1000, 0, 0, 1000, 100, -2)))
        assert {stage: observed['durable'][stage] - observed['fast'][stage]
                for stage in expected} == expected
        result['redb_commit_windows'] = windows
    return result


def span(s, scale=1, digits=3):
    return ('{0:.{d}f} [{1:.{d}f}, {2:.{d}f}]'
            .format(s['median']/scale, s['min']/scale, s['max']/scale, d=digits))


def tables(path, result, primary):
    with path.open('w') as f:
        print('# ' + ('Primary' if primary else 'Screening') + ' results\n', file=f)
        print('Five-trial median [min, max]. Ratios above one favor the row engine. '
              'Paired ratios are computed within rounds before aggregation.\n', file=f)
        for scenario, workloads in result.items():
            for workload, labels in workloads.items():
                print(f'## {scenario} · {workload}\n', file=f)
                print('| Engine | p50 µs | p99 µs | Call kops/s | Verified kops/s |', file=f)
                print('|---|---:|---:|---:|---:|', file=f)
                for label, row in labels.items():
                    print('| ' + ' | '.join((label, *(span(row[k], 1000) for k in METRICS))) + ' |', file=f)
                if primary:
                    for baseline in ('redb', 'lmdb'):
                        print(f'\n| Engine vs {baseline} | p50 ratio | p99 ratio | Call rate ratio | Verified rate ratio |', file=f)
                        print('|---|---:|---:|---:|---:|', file=f)
                        for label, row in labels.items():
                            print('| ' + ' | '.join((label, *(span(row['paired'][baseline][k])
                                                            for k in METRICS))) + ' |', file=f)
                print(file=f)
            print(f'## {scenario} · resources\n', file=f)
            print('| Engine | Peak RSS MiB | Logical MiB | Allocated MiB |', file=f)
            print('|---|---:|---:|---:|', file=f)
            for label, row in next(iter(workloads.values())).items():
                print('| ' + ' | '.join((label, *(span(row[k], scale) for k, scale in
                                                zip(RESOURCES, (1024, 1024**2, 1024**2))))) + ' |', file=f)
            if scenario != next(reversed(result)):
                print(file=f)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', nargs='?', type=Path, default=STUDY / 'raw')
    args = parser.parse_args()
    raw = args.raw.resolve()
    out = raw.parent
    assert not (raw / 'INVALID').exists()
    metadata = json.loads((raw / 'metadata.json').read_text())
    assert metadata['cpu'] == CPU and metadata['sibling'] == SIBLING
    assert metadata['seed'] == SEED and metadata['trials'] == TRIALS
    assert metadata['protocol_sha256'] == digest(STUDY / 'PROTOCOL.md')
    binary = Path(metadata['binary'])
    if binary.is_file():
        assert digest(binary) == metadata['binary_sha256'], binary
    for rel, sha in metadata['sources'].items():
        assert digest(ROOT / rel) == sha, rel
    preparation = audit_preparation(raw)
    selection = choose(raw)
    assert selection == json.loads((raw / 'selection.json').read_text())
    checks = audit_checks(out)
    summary, audit, trials = {}, {}, []
    default_caches = set()
    for phase in ('screen', 'measure'):
        summary[phase], rows, audit[phase] = audit_phase(raw, phase, selection, default_caches, preparation)
        trials.extend(rows)
    default_cache, = default_caches
    audit.update(default_cache_bytes=default_cache, diagnostic_checks=checks)
    for filename, data in (('summary.json', summary), ('audit.json', audit),
                           ('fixture-layouts.json', preparation)):
        (out / filename).write_text(json.dumps(data, indent=2) + '\n')
    with (out / 'TRIALS.csv').open('w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=trials[0], lineterminator='\n')
        writer.writeheader()
        writer.writerows(trials)
    tables(out / 'SCREEN.md', summary['screen'], primary=False)
    tables(out / 'MEASURE.md', summary['measure'], primary=True)
    for phase in ('screen', 'measure'):
        print(phase, [(k, audit[phase][k]) for k in ('commands', 'timed_calls', 'timed_mutations')])


if __name__ == '__main__':
    main()
