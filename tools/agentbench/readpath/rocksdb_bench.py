#!/usr/bin/env python3
"""Frozen RocksDB/Kiban/LMDB campaign; no compilation during this driver."""
import argparse
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT / 'tools/agentbench/readpath/target/release/kiban-readbench'
DATA = ROOT / 'target/rocksdb/data'
SEED = 20261016
CANDIDATES = {
    f'{"ribbon" if ribbon else "bloom"}-{block}-{"total" if total else "prefix"}':
    dict(block_size=block, ribbon=ribbon, total_order=total)
    for ribbon, block, total in itertools.product((False, True), (4096, 16384), (False, True))}
RULE = ('For reads and writes separately, minimize the equal-workload geometric mean of '
        'five-trial medians of mean call seconds. Reads include both operations, both layouts '
        'and both cache budgets; writes include remember/update/mixed in fast and durable. '
        'Select one whole configuration per family; ties break by candidate label. '
        'Optional mmap and row-cache rows are omitted. LZ4 is separately labeled, uses '
        'the frozen winner, and never enters selection. No-WAL uses the write winner.')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ticks():
    return {p[0]: list(map(int, p[1:])) for line in Path('/proc/stat').read_text().splitlines()
            if (p := line.split()) and p[0] in ('cpu10', 'cpu11')}


def disk(path):
    files = [p for p in path.rglob('*') if p.is_file()]
    return dict(logical_bytes=sum(p.stat().st_size for p in files),
                allocated_bytes=sum(p.stat().st_blocks * 512 for p in files),
                files={str(p.relative_to(path)): p.stat().st_size for p in files})


def base(engine, n, layout='bulk', cache=None, tuning=None):
    return dict(backend=engine, n=n, operations=100000, k=10, seed=SEED,
                db='', output='', action='read', layout=layout, cache_mib=cache,
                point_index=False, durable=False, rocks=tuning or {})


def fixture(c):
    label = c['backend']
    if label == 'kiban':
        label += f'-index{int(c["point_index"])}-{c["layout"]}'
    if label == 'rocksdb':
        t = c['rocks']
        # Seek/WAL choices do not change fixture bytes or table settings.
        label += f'-{"ribbon" if t.get("ribbon") else "bloom"}-{t.get("block_size",4096)}-{"lz4" if t.get("lz4") else "none"}-{c["layout"]}'
    return DATA / f'{c["n"]}-{label}'


def run(c, out, name, resume=False, probe=None):
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
    if c['action'] == 'prepare' and Path(c['db']).exists():
        assert resume and Path(c['db']).parent == DATA, 'unexpected existing fixture'
        shutil.rmtree(Path(c['db']))
    if c['action'] == 'write':
        dest = DATA / ('probe-trial' if probe else 'write-trial')
        if dest.exists():
            assert resume, 'unexpected trial directory'
            shutil.rmtree(dest)
        subprocess.run(['cp', '-a', '--reflink=auto', str(fixture(c)), str(dest)], check=True)
        assert c['db'] == str(dest)
    Path(c['output']).unlink(missing_ok=True)
    path.write_text(json.dumps(c, indent=2) + '\n')
    before = ticks()
    started = time.time()
    print(str(out.relative_to(out.parents[1])) + '/' + name, flush=True)
    env = dict(os.environ)
    if probe:
        env['LD_PRELOAD'] = str(probe)
    with (out / (name + '.log')).open('w') as log:
        subprocess.run(['/usr/bin/time', '-v', '-o', str(out / (name + '.time.txt')),
                        'taskset', '-c', '10', str(BINARY), str(path)],
                       stdout=log, stderr=log, check=True, env=env)
    after = ticks()
    record = dict(name=name, config=path.name, started=started, ended=time.time(),
                  affinity='10', load=os.getloadavg(),
                  cpu_tick_delta={cpu: [b-a for a, b in zip(before[cpu], after[cpu])]
                                  for cpu in before}, disk=disk(Path(c['db'])), diagnostic=bool(probe))
    if c['action'] != 'prepare':
        rows = [json.loads(s) for s in Path(c['output']).read_text().splitlines()]
        assert all(r.get('errors', 0) == 0 for r in rows)
        assert any(r['type'] == 'complete' for r in rows)
        assert c['action'] != 'write' or any(r['type'] == 'reopen_verified' for r in rows)
        record['result_sha256'] = digest(Path(c['output']))
    else:
        assert '"type":"prepared"' in (out / (name + '.log')).read_text()
        # Exact RocksDB effective options, including untouched defaults.
        if c['backend'] == 'rocksdb':
            options = sorted(Path(c['db']).glob('OPTIONS-*'))[-1]
            shutil.copyfile(options, out / (name + '.OPTIONS.txt'))
            shutil.copyfile(Path(c['db']) / 'LOG', out / (name + '.ROCKSDB-LOG.txt'))
    record['config_sha256'] = digest(path)
    record['log_sha256'] = digest(out / (name + '.log'))
    record['time_sha256'] = digest(out / (name + '.time.txt'))
    with receipts.open('a') as f:
        f.write(json.dumps(record) + '\n')
    if c['action'] == 'write':
        shutil.rmtree(Path(c['db']))


def prepare(args, compressed=False):
    if compressed:
        selection = json.loads((args.out / 'selection.json').read_text())
        configs = [base('rocksdb', 1000000, shape, tuning=dict(CANDIDATES[selection['read']], lz4=True))
                   for shape in ('bulk', 'normal')]
        configs.append(base('rocksdb', 10000, tuning=dict(CANDIDATES[selection['write']], lz4=True)))
    else:
        configs = []
        for n in (10000, 1000000):
            configs.append(base('lmdb', n))
            configs += [dict(base('kiban', n, shape), point_index=index)
                        for shape in (('bulk',) if n == 10000 else ('bulk', 'normal'))
                        for index in (False, True)]
            configs += [base('rocksdb', n, shape, tuning=dict(block_size=block, ribbon=ribbon))
                        for shape in (('bulk',) if n == 10000 else ('bulk', 'normal'))
                        for ribbon, block in itertools.product((False, True), (4096, 16384))]
    for c in configs:
        c.update(action='prepare', db=str(fixture(c)))
        run(c, args.out / 'prepare', fixture(c).name, args.resume)


def rotate(items, trial):
    offset = trial % len(items)
    return items[offset:] + items[:offset]


def measure(args, screening=False):
    selection = None if screening else json.loads((args.out / 'selection.json').read_text())
    scenarios = [('read', 1000000, cache, False, shape)
                 for shape in ('bulk', 'normal') for cache in (512, None)]
    scenarios += [('write', 10000, None, durable, 'bulk') for durable in (False, True)]
    for action, n, cache, durable, shape in scenarios:
        scenario = f'{action}-{n}-{shape}-{cache}-{"durable" if durable else "fast"}'
        if screening:
            configs = [(label, base('rocksdb', n, shape, cache, tuning)) for label, tuning in CANDIDATES.items()]
        else:
            configs = [('kiban-index' + str(int(index)), dict(base('kiban', n, shape, cache), point_index=index)) for index in (False, True)]
            chosen = CANDIDATES[selection[action]]
            configs += [('rocksdb-tuned', base('rocksdb', n, shape, cache, chosen))]
            configs += [('rocksdb-lz4', base('rocksdb', n, shape, cache, dict(chosen, lz4=True)))]
            if action == 'write' and not durable:
                configs += [('rocksdb-no-wal', base('rocksdb', n, shape, cache, dict(chosen, disable_wal=True)))]
        configs.append(('lmdb', base('lmdb', n, shape, cache)))
        for trial in range(5):
            for label, conf in rotate(configs, trial):
                c = dict(conf, action=action, durable=durable, operations=100000 if action == 'read' else 1000)
                c['db'] = str(DATA / 'write-trial') if action == 'write' else str(fixture(c))
                run(c, args.out / ('screen' if screening else 'measure') / scenario, f'r{trial+1}-{label}', args.resume)


def select(args):
    scores = {}
    for action in ('read', 'write'):
        totals = {label: [] for label in CANDIDATES}
        for folder in sorted((args.out / 'screen').glob(action + '-*')):
            for label in totals:
                stages = {}
                for path in folder.glob('r*-' + label + '.jsonl'):
                    for r in map(json.loads, path.read_text().splitlines()):
                        if r['type'] == 'stage' and (action == 'read' or r['workload'] in ('remember', 'update', 'mixed-90-10')):
                            stages.setdefault(r['workload'], []).append(r['call_seconds'] / r['latency']['count'])
                assert len(stages) == (2 if action == 'read' else 3) and all(len(v) == 5 for v in stages.values())
                totals[label].extend(statistics.median(v) for v in stages.values())
        assert all(len(v) == (8 if action == 'read' else 6) for v in totals.values())
        scores[action] = {k: math.exp(statistics.mean(map(math.log, v))) for k, v in totals.items()}
    result = {action: min(s, key=lambda k: (s[k], k)) for action, s in scores.items()}
    result.update(scores_seconds=scores, rule=RULE)
    path = args.out / 'selection.json'
    if path.exists():
        assert json.loads(path.read_text()) == result, 'frozen selection changed'
    else:
        path.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


def probe(args):
    selection = json.loads((args.out / 'selection.json').read_text())
    for durable in (False, True):
        configs = [('kiban', base('kiban', 10000)), ('lmdb', base('lmdb', 10000)),
                   ('rocksdb', base('rocksdb', 10000, tuning=CANDIDATES[selection['write']]))]
        if not durable:
            configs.append(('rocksdb-no-wal', base('rocksdb', 10000, tuning=dict(CANDIDATES[selection['write']], disable_wal=True))))
        for label, c in configs:
            c.update(action='write', operations=1000, durable=durable, db=str(DATA / 'probe-trial'))
            run(c, args.out / 'sync-probe', label + ('-durable' if durable else '-fast'), args.resume, ROOT / 'target/rocksdb/sync-probe.so')


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('action', choices=['prepare', 'screen', 'select', 'prepare-lz4', 'measure', 'probe'])
    parser.add_argument('--out', type=Path, default=ROOT / 'validation/rocksdb/raw')
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args()
    assert set(os.sched_getaffinity(0)) <= set(range(12,22)), 'pin coordinator to 12-21'
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    sources = sorted((ROOT / 'src').rglob('*.rs'))
    harness = sorted((ROOT / 'tools/agentbench/readpath/src').glob('*.rs'))
    harness += [ROOT / p for p in ('tools/agentbench/src/api.rs', 'examples/scale/metrics.rs', 'examples/support/random.rs', 'tools/agentbench/src/runner.rs', 'tools/agentbench/src/model.rs', 'tools/agentbench/src/embedded.rs', 'tools/agentbench/readpath/Cargo.lock', 'tools/agentbench/readpath/Cargo.toml', 'tools/agentbench/readpath/rocksdb_bench.py', 'tools/agentbench/readpath/rocksdb_sync_probe.c')]
    provenance = dict(engine_head=subprocess.check_output(['git', 'log', '-1', '--format=%H', '--', 'src'], text=True).strip(),
                      binary=str(BINARY), binary_sha256=digest(BINARY), seed=SEED, trials=5,
                      cpu=10, sibling=11, build_test_cpus='12-21', selection_rule=RULE,
                      candidates=CANDIDATES,
                      engine_sources={str(p.relative_to(ROOT)): digest(p) for p in sources},
                      harness_sources={str(p.relative_to(ROOT)): digest(p) for p in harness})
    metadata = args.out / 'metadata.json'
    if metadata.exists():
        assert json.loads(metadata.read_text()) == provenance, 'frozen binary/source provenance changed'
    else:
        metadata.write_text(json.dumps(provenance, indent=2) + '\n')
    if args.action in ('prepare', 'prepare-lz4'): prepare(args, args.action == 'prepare-lz4')
    elif args.action == 'select': select(args)
    elif args.action == 'probe': probe(args)
    else: measure(args, args.action == 'screen')


if __name__ == '__main__':
    main()
