#!/usr/bin/env python3
"""Reproduce the integrated Bloom/fuse comparison, with per-process receipts.

Build first, then prepare (12-21/nice10), then measure (CPU8, no builds/tests).
Every before/after command is followed by the identical LMDB control. Read
fixtures are immutable; write trials use disposable reflink copies. Refuses
incomplete results, errors, wrong affinity or unexpected operation counts.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]


def sibling():
    fields = next(x for x in Path('/proc/stat').read_text().splitlines()
                  if x.startswith('cpu9 ')).split()[1:9]
    ticks = list(map(int, fields))
    return dict(total=sum(ticks), idle=ticks[3] + ticks[4])


def config(engine, path, layout, indexed=False, fuse=False, n=1_000_000):
    return dict(backend=engine, db=str(path), n=n, operations=100_000,
                k=10, seed=20261016, output='', action='read', layout=layout,
                cache_mib=None, point_index=indexed, binary_fuse_filter=fuse, rocks={})


def fixture(data, label, engine, layout, indexed, n=1_000_000):
    # LMDB control is independent of the SST format and point-index switch.
    name = f'{label}-{engine}-{layout}-index{int(indexed)}-{n}' if engine == 'kiban' else f'lmdb-{layout}-{n}'
    return data / name


def execute(binary, c, out, name, prepare=False):
    out.mkdir(parents=True, exist_ok=True)
    output = out / (name + '.jsonl')
    receipt = out / (name + '.receipt.json')
    if receipt.exists():
        old = json.loads(receipt.read_text())
        assert old['code'] == 0 and old['sha256'] == hashlib.sha256(binary.read_bytes()).hexdigest()
        assert json.loads((out / (name + '.config.json')).read_text()) == {**c, 'output': str(output)}
        return old
    assert not output.exists(), f'Incomplete output: {output}; inspect before retrying'
    c = {**c, 'output': str(output)}
    cfg = out / (name + '.config.json')
    cfg.write_text(json.dumps(c, indent=2) + '\n')
    command = ['taskset', '-c', '12-21', 'nice', '-n', '10'] if prepare else ['taskset', '-c', '8']
    command += [str(binary), str(cfg)]
    start = sibling()
    tick = time.time()
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=log)
    end = sibling()
    row = dict(name=name, command=command, code=result.returncode,
               sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
               seconds=time.time()-tick, start=start, end=end,
               cpu9_busy_percent=100*(1-(end['idle']-start['idle'])/max(1,end['total']-start['total'])),
               load=os.getloadavg())
    assert result.returncode == 0, (name, (out / (name + '.log')).read_text()[-4000:])
    if prepare:
        prepared = [json.loads(line) for line in (out / (name + '.log')).read_text().splitlines() if line.startswith('{') and '"type":"prepared"' in line]
        assert len(prepared) == 1 and prepared[0]['verified_records'] == c['n']
    else:
        rows = [json.loads(line) for line in output.read_text().splitlines()]
        complete, = [r for r in rows if r['type'] == 'complete']
        assert all(r.get('errors', 0) == 0 for r in rows)
        status = next(r['proc_status'] for r in reversed(rows) if 'proc_status' in r)
        assert 'Cpus_allowed_list:\t8\n' in status
        stages = [r for r in rows if r['type'] == 'stage']
        assert stages and all(r['latency']['count'] == c['operations'] for r in stages)
        if c['action'] in ['read', 'miss']:
            assert complete['verified_records'] == c['n'] and len(stages) == 2
        else:
            assert complete['verified_records'] == c['n'] + c['operations'] and len(stages) == 5
            reopened, = [r for r in rows if r['type'] == 'reopen_verified']
            assert reopened['verified_records'] == c['n'] + c['operations']
            assert sum(s['writes']['count'] for s in stages) == c['operations'] * 2 + c['operations'] // 10
        row['stages'] = stages
    receipt.write_text(json.dumps(row, indent=2) + '\n')
    print(name, f"{row['seconds']:.2f}s sibling={row['cpu9_busy_percent']:.1f}%", flush=True)
    return row


def inventory(data, out):
    tables = []
    for db in sorted(data.iterdir()):
        if 'kiban' not in db.name:
            continue
        for path in sorted(db.glob('*.sst')):
            with path.open('rb') as f:
                f.seek(-44, 2)
                foot = f.read(44)
                version = int.from_bytes(foot[16:20], 'little')
                offset = int.from_bytes(foot[20:28], 'little')
                length = int.from_bytes(foot[28:36], 'little')
                f.seek(offset)
                header = f.read(48)
                algorithm = header[9] if version in [5, 6] else 'bloom'
            tables.append(dict(db=db.name, file=path.name, bytes=path.stat().st_size,
                               version=version, filter_bytes=length, algorithm=algorithm))
    (out / 'sst-inventory.json').write_text(json.dumps(tables, indent=2)+'\n')
    for row in tables:
        if row['db'].startswith('after'):
            assert row['version'] in [5, 6], row
        else:
            assert row['version'] in [3, 4], row


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('phase', choices=['prepare', 'measure', 'maintenance', 'inventory'])
    p.add_argument('--before', type=Path, required=True)
    p.add_argument('--after', type=Path, required=True)
    p.add_argument('--before-test', type=Path)
    p.add_argument('--after-test', type=Path)
    p.add_argument('--data', type=Path, default=ROOT/'target/fuse-engine/data')
    p.add_argument('--out', type=Path, default=ROOT/'validation/fuse-engine/results')
    a = p.parse_args()
    a.data = a.data.resolve()
    a.out = a.out.resolve()
    binaries = {k: v.resolve() for k, v in [('before', a.before), ('after', a.after)]}
    a.data.mkdir(parents=True, exist_ok=True)
    a.out.mkdir(parents=True, exist_ok=True)
    if a.phase == 'inventory':
        inventory(a.data, a.out)
        return
    if a.phase == 'maintenance':
        tests = {k: v.resolve() for k, v in [('before', a.before_test), ('after', a.after_test)]}
        for trial in range(1, 6):
            for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
                name = f'maintenance-r{trial}-{label}'
                receipt = a.out/(name+'.receipt.json')
                if receipt.exists():
                    continue
                start = sibling()
                tick = time.time()
                command = ['taskset', '-c', '8', '/usr/bin/time', '-v', '-o', str(a.out/(name+'.time.txt')), str(tests[label]), 'point_index_maintenance_measurement', '--ignored', '--nocapture', '--test-threads=1']
                env = {**os.environ, 'KIBAN_POINT_BENCH_N': '1000000', 'KIBAN_POINT_BENCH_TRIAL': str(trial), 'LC_ALL': 'C'}
                env.pop('KIBAN_FUSE_BENCH', None)
                if label == 'after':
                    env['KIBAN_FUSE_BENCH'] = '1'
                with (a.out/(name+'.log')).open('w') as log:
                    result = subprocess.run(command, env=env, stdout=log, stderr=log)
                end = sibling()
                records = [json.loads(line[line.index('{"type":"maintenance"'):]) for line in (a.out/(name+'.log')).read_text().splitlines() if '{"type":"maintenance"' in line]
                assert result.returncode == 0 and len(records) == 2
                assert all(r['errors'] == 0 and r['verified_records'] == 1000000 for r in records)
                row = dict(name=name, code=result.returncode, command=command,
                           sha256=hashlib.sha256(tests[label].read_bytes()).hexdigest(), records=records,
                           max_rss_kib=int(next(line.rsplit(':',1)[1] for line in (a.out/(name+'.time.txt')).read_text().splitlines() if 'Maximum resident set size (kbytes)' in line)),
                           seconds=time.time()-tick, start=start, end=end,
                           cpu9_busy_percent=100*(1-(end['idle']-start['idle'])/max(1,end['total']-start['total'])))
                receipt.write_text(json.dumps(row, indent=2)+'\n')
                print(name, flush=True)
        return
    if a.phase == 'prepare':
        for n in [1_000_000, 10_000]:
            for layout in ['bulk', 'normal'] if n == 1_000_000 else ['bulk']:
                for engine in ['kiban', 'lmdb']:
                    for indexed in [False, True] if engine == 'kiban' else [False]:
                        for label in ['before', 'after'] if engine == 'kiban' else ['before']:
                            path = fixture(a.data, label, engine, layout, indexed, n)
                            c = config(engine, path, layout, indexed, label == 'after', n)
                            c['action'] = 'prepare'
                            execute(binaries[label], c, a.out/'prepare', path.name, prepare=True)
        inventory(a.data, a.out)
    else:
        for layout in ['bulk', 'normal']:
            for cache in [32, None]:
                for indexed in [False, True]:
                    for action in ['read', 'miss']:
                        for trial in range(1, 6):
                            for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
                                for engine in ['kiban', 'lmdb']:
                                    path = fixture(a.data, label, engine, layout, indexed)
                                    c = config(engine, path, layout, indexed, label == 'after')
                                    c.update(cache_mib=cache, action=action)
                                    name = f'{layout}-cache{cache}-index{int(indexed)}-{action}-r{trial}-{label}-{engine}'
                                    execute(binaries[label], c, a.out/'reads', name)
        for durable in [False, True]:
            for trial in range(1, 6):
                for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
                    for engine in ['kiban', 'lmdb']:
                        path = fixture(a.data, label, engine, 'bulk', False, 10_000)
                        dest = a.data/f'write-{label}-{engine}'
                        name = f'write-durable{int(durable)}-r{trial}-{label}-{engine}'
                        if (a.out/'writes'/(name+'.receipt.json')).exists():
                            continue
                        assert not dest.exists(), f'Inspect incomplete trial {dest}'
                        subprocess.run(['cp', '-a', '--reflink=auto', str(path), str(dest)], check=True)
                        c = config(engine, dest, 'bulk', fuse=label == 'after', n=10_000)
                        c.update(action='write', operations=10_000, durable=durable)
                        execute(binaries[label], c, a.out/'writes', name)
                        shutil.rmtree(dest)


if __name__ == '__main__':
    main()
