#!/usr/bin/env python3
"""SQLite/Kiban/LMDB experiment. Build first; all measured children use CPU 10.

Prepare private fixtures, screen both SQLite layouts with LMDB each round,
then measure the selected layout in five rotated rounds. Never builds/tests.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT / 'tools/agentbench/readpath/target/release/kiban-readbench'
DATA = ROOT / 'target/sqlite/data'
SEED = 20261016


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cpu_ticks():
    return {p[0]: list(map(int, p[1:])) for line in Path('/proc/stat').read_text().splitlines()
            if (p := line.split()) and p[0] in ('cpu10', 'cpu11')}


def disk(path):
    files = [p for p in path.rglob('*') if p.is_file()]
    return dict(logical_bytes=sum(p.stat().st_size for p in files),
                allocated_bytes=sum(p.stat().st_blocks * 512 for p in files),
                files={str(p.relative_to(path)): p.stat().st_size for p in files})


def base(engine, n, layout='bulk', cache=None):
    return dict(backend=engine, n=n, operations=100000, k=10, seed=SEED,
                db='', output='', action='read', layout=layout, cache_mib=cache,
                point_index=False, durable=False, sqlite_layout='rowid',
                sqlite_sync='NORMAL', sqlite_mmap_bytes=0)


def fixture(c):
    label = c['backend']
    if label == 'kiban':
        label += '-index' + str(int(c['point_index'])) + '-' + c['layout']
    if label == 'sqlite':
        label += '-' + c['sqlite_layout']
    return DATA / f"{c['n']}-{label}"


def run(c, out, name, resume):
    out.mkdir(parents=True, exist_ok=True)
    path = out / (name + '.config.json')
    c = dict(c, output=str((out / (name + '.jsonl')).resolve()))
    receipts = out / 'commands.jsonl'
    completed = [json.loads(s) for s in receipts.read_text().splitlines()] if receipts.exists() else []
    old = next((r for r in completed if r['name'] == name), None)
    if old:
        assert resume, 'use --resume or a fresh output directory'
        assert json.loads(path.read_text()) == c, 'resume configuration changed'
        if c['action'] != 'prepare':
            assert digest(Path(c['output'])) == old['result_sha256'], 'saved result changed'
        return
    if c['action'] == 'write':
        source = fixture(c)
        dest = DATA / 'write-trial'
        if dest.exists():
            assert resume, 'unexpected trial directory'
            shutil.rmtree(dest)
        subprocess.run(['cp', '-a', '--reflink=auto', str(source), str(dest)], check=True)
        assert c['db'] == str(dest)
    Path(c['output']).unlink(missing_ok=True)
    path.write_text(json.dumps(c, indent=2) + '\n')
    before = cpu_ticks()
    started = time.time()
    print(name, flush=True)
    with (out / (name + '.log')).open('w') as log:
        subprocess.run(['/usr/bin/time', '-v', '-o', str(out / (name + '.time.txt')),
                        'taskset', '-c', '10', str(BINARY), str(path)],
                       stdout=log, stderr=log, check=True)
    after = cpu_ticks()
    record = dict(name=name, config=path.name, started=started, ended=time.time(),
                  affinity='10', load=os.getloadavg(),
                  cpu_tick_delta={cpu: [b-a for a, b in zip(before[cpu], after[cpu])]
                                  for cpu in before}, disk=disk(Path(c['db'])))
    if c['action'] != 'prepare':
        rows = [json.loads(s) for s in Path(c['output']).read_text().splitlines()]
        assert all(r.get('errors', 0) == 0 for r in rows)
        assert any(r['type'] == 'complete' for r in rows)
        assert c['action'] != 'write' or any(r['type'] == 'reopen_verified' for r in rows)
        record['result_sha256'] = digest(Path(c['output']))
    else:
        assert '"type":"prepared"' in (out / (name + '.log')).read_text()
    with receipts.open('a') as f:
        f.write(json.dumps(record) + '\n')
    if c['action'] == 'write':
        shutil.rmtree(Path(c['db']))


def prepare(args):
    for n in (10000, 1000000):
        configs = [base('lmdb', n)]
        configs += [dict(base('sqlite', n), sqlite_layout=s) for s in ('rowid', 'clustered')]
        configs += [dict(base('kiban', n, shape), point_index=index)
                    for shape in (('bulk',) if n == 10000 else ('bulk', 'normal'))
                    for index in (False, True)]
        for c in configs:
            c.update(action='prepare', db=str(fixture(c)))
            run(c, args.out / 'prepare', fixture(c).name, args.resume)


def rotate(items, trial):
    # Rotation gives every entry every position for five-entry read scenarios.
    offset = trial % len(items)
    return items[offset:] + items[:offset]


def measure(args, screening):
    selection = None if screening else json.loads((args.out / 'selection.json').read_text())
    scenarios = [('read', 1000000, cache, False, shape)
                 for shape in ('bulk', 'normal') for cache in (512, None)]
    scenarios += [('write', 10000, None, durable, 'bulk') for durable in (False, True)]
    for action, n, cache, durable, shape in scenarios:
        scenario = f"{action}-{n}-{shape}-{cache}-{'durable' if durable else 'fast'}"
        configs = []
        if screening:
            configs = [(s, dict(base('sqlite', n, shape, cache), sqlite_layout=s))
                       for s in ('rowid', 'clustered')]
        else:
            configs += [('kiban-index' + str(int(index)),
                         dict(base('kiban', n, shape, cache), point_index=index))
                        for index in (False, True)]
            chosen = selection['read' if action == 'read' else 'write']
            syncs = ('NORMAL',) if action == 'read' else (('FULL', 'NORMAL') if durable else ('OFF', 'NORMAL'))
            configs += [('sqlite-' + sync.lower(), dict(base('sqlite', n, shape, cache),
                        sqlite_layout=chosen, sqlite_sync=sync)) for sync in syncs]
            if action == 'read':
                configs += [('sqlite-mmap', dict(base('sqlite', n, shape, cache),
                             sqlite_layout=selection['read_mmap'], sqlite_mmap_bytes=1024**3))]
        configs += [('lmdb', base('lmdb', n, shape, cache))]
        for trial in range(5):
            for label, c in rotate(configs, trial):
                c.update(action=action, durable=durable,
                         operations=100000 if action == 'read' else 1000)
                c['db'] = str(fixture(c) if action == 'read' else DATA / 'write-trial')
                # Both layouts use the same WAL durability during screening.
                if screening:
                    c['sqlite_sync'] = 'FULL' if durable else 'NORMAL'
                run(c, args.out / ('screen' if screening else 'measure') / scenario,
                    f'r{trial+1}-{label}', args.resume)


def select(args):
    # mmap changes the I/O path and can change the winning schema. Screen
    # both layouts before freezing the final mmap configuration, too.
    mmap_selection = args.out / 'mmap-screen' / 'selection.json'
    if not mmap_selection.exists():
        subprocess.run([sys.executable, str(Path(__file__).with_name('sqlite_mmap_screen.py')),
                        str(args.out)], check=True)
    # Minimize equal-workload geometric mean of median call time. No selection
    # from a single lucky trial; preserve each workload's tradeoff in the report.
    import math
    scores = {}
    for action in ('read', 'write'):
        totals = {s: [] for s in ('rowid', 'clustered')}
        for folder in sorted((args.out / 'screen').glob(action + '-*')):
            for label in totals:
                stages = {}
                for path in folder.glob('r*-' + label + '.jsonl'):
                    for r in map(json.loads, path.read_text().splitlines()):
                        if r['type'] == 'stage' and (action == 'read' or r['workload'] in ('remember', 'update', 'mixed-90-10')):
                            stages.setdefault(r['workload'], []).append(r['call_seconds'] / r['latency']['count'])
                assert stages and all(len(v) == 5 for v in stages.values())
                totals[label].extend(statistics.median(v) for v in stages.values())
        scores[action] = {k: math.exp(statistics.mean(map(math.log, v))) for k, v in totals.items()}
    result = {action: min(s, key=s.get) for action, s in scores.items()}
    result['scores_seconds'] = scores
    result['read_mmap'] = json.loads(mmap_selection.read_text())['read_mmap']
    result['mmap_scores_seconds'] = json.loads(mmap_selection.read_text())['scores_seconds']
    (args.out / 'selection.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('action', choices=['prepare', 'screen', 'select', 'measure'])
    parser.add_argument('--out', type=Path, default=ROOT / 'validation/sqlite/raw')
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    source_files = sorted((ROOT / 'src').rglob('*.rs'))
    provenance = dict(engine_head=subprocess.check_output(['git', 'log', '-1', '--format=%H', '--', 'src'], text=True).strip(),
                      binary=str(BINARY), binary_sha256=digest(BINARY), seed=SEED, trials=5,
                      cpu=10, sibling=11, build_test_cpus='12-21',
                      engine_sources={str(p.relative_to(ROOT)): digest(p) for p in source_files})
    metadata = args.out / 'metadata.json'
    if metadata.exists():
        assert json.loads(metadata.read_text()) == provenance, 'frozen binary/source provenance changed'
    else:
        metadata.write_text(json.dumps(provenance, indent=2) + '\n')
    if args.action == 'prepare': prepare(args)
    elif args.action == 'select': select(args)
    else: measure(args, args.action == 'screen')


if __name__ == '__main__':
    main()
