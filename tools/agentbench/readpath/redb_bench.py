#!/usr/bin/env python3
"""Prepare, screen and measure the frozen redb comparison on CPU 8."""
import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

from rocksdb_bench import ROOT, SEED, base, digest, rotate, run, ticks

STUDY = ROOT / 'validation/redb'
BINARY = ROOT / 'target/redb-h2h/release/kiban-readbench'
DATA = ROOT / 'target/redb-data'
CPU, SIBLING = 8, 9
TRIALS = 5
CANDIDATES = {
    'as-is': dict(compact='none', quick_repair=False),
    'balanced': dict(compact='balanced', quick_repair=False),
    'maximum': dict(compact='maximum', quick_repair=False),
    'balanced-quick-repair': dict(compact='balanced', quick_repair=True),
}
RULE = (
    'Minimize the geometric mean of the per-stage medians of mean call time. '
    'Use both read stages across four read scenarios, and remember/update/mixed '
    'across both write modes. Choose one configuration per family. Ties use label order.'
)
SCENARIOS = [
    ('read', 1000000, shape, cache, False)
    for shape in ('bulk', 'normal') for cache in (512, None)
] + [('write', 10000, 'bulk', None, durable) for durable in (False, True)]


def read_jsonl(path):
    return [json.loads(s) for s in path.read_text().splitlines()]


def scenario_name(scenario):
    action, n, layout, cache, durable = scenario
    return f'{action}-{n}-{layout}-{cache}-{"durable" if durable else "fast"}'


def fixture(c):
    backend, layout = c['backend'], c['layout']
    if backend == 'kiban':
        return DATA / f'{c["n"]}-kiban-index{int(c["point_index"])}-{layout}'
    if backend == 'redb':
        compact = c['redb']['compact'] if layout == 'bulk' else 'none'
        return DATA / f'{c["n"]}-redb-{layout}-{compact}'
    return DATA / f'{c["n"]}-lmdb-bulk'


def configuration(engine, n, layout='bulk', cache=None, label=None):
    c = base(engine, n, layout, cache)
    if engine == 'redb':
        c['redb'] = dict(CANDIDATES[label])
        if layout == 'normal':
            c['redb']['compact'] = 'none'
    elif engine == 'kiban':
        c['point_index'] = label == 'kiban-index1'
        c['scan_prefix_len'] = 18
    return c


def execute(args, c, out, label, **kwargs):
    run(c, out, label, args.resume, binary=BINARY, data=DATA, cpu=CPU,
        sibling=SIBLING, fixture_path=fixture, invalid=args.out / 'INVALID', **kwargs)


def prepare(args):
    out = args.out / 'prepare'
    for n in (10000, 1000000):
        configs = [configuration('lmdb', n)]
        for shape in ('bulk', 'normal') if n == 1000000 else ('bulk',):
            configs.append(configuration('redb', n, shape, label='as-is'))
            configs.extend(configuration('kiban', n, shape, label=label)
                           for label in ('kiban-index0', 'kiban-index1'))
        for c in configs:
            path = fixture(c)
            execute(args, dict(c, action='prepare', db=str(path)), out, path.name)
        plain = fixture(configuration('redb', n, label='as-is'))
        for label in ('balanced', 'maximum'):
            c = configuration('redb', n, label=label)
            path = fixture(c)
            execute(args, dict(c, action='compact-redb', db=str(path)), out, path.name,
                    copy_from=plain)


def measure(args, screening):
    if not screening:
        selection = json.loads((args.out / 'selection.json').read_text())
    for scenario in SCENARIOS:
        action, n, shape, cache, durable = scenario
        if screening:
            configs = [(label, configuration('redb', n, shape, cache, label))
                       for label in CANDIDATES]
        else:
            configs = [(label, configuration('kiban', n, shape, cache, label))
                       for label in ('kiban-index0', 'kiban-index1')]
            configs.append(('redb', configuration('redb', n, shape, cache, selection[action])))
        configs.append(('lmdb', configuration('lmdb', n, shape, cache)))
        out = args.out / ('screen' if screening else 'measure') / scenario_name(scenario)
        for trial in range(TRIALS):
            for label, conf in rotate(configs, trial):
                c = dict(conf, action=action, durable=durable,
                         operations=100000 if action == 'read' else 1000)
                c['db'] = str(DATA / 'write-trial' if action == 'write' else fixture(c))
                execute(args, c, out, f'r{trial + 1}-{label}')


def choose(raw):
    totals = {action: {label: [] for label in CANDIDATES} for action in ('read', 'write')}
    for scenario in SCENARIOS:
        action = scenario[0]
        folder = raw / 'screen' / scenario_name(scenario)
        receipts = {r['name']: r for r in read_jsonl(folder / 'commands.jsonl')}
        for label in CANDIDATES:
            stages = {}
            for trial in range(1, TRIALS + 1):
                name = f'r{trial}-{label}'
                result = folder / f'{name}.jsonl'
                assert digest(result) == receipts[name]['result_sha256']
                for row in read_jsonl(result):
                    if row['type'] == 'stage' and (action == 'read' or row['workload'] in
                                                  ('remember', 'update', 'mixed-90-10')):
                        stages.setdefault(row['workload'], []).append(
                            row['call_seconds'] / row['latency']['count'])
            assert len(stages) == (2 if action == 'read' else 3)
            assert all(len(v) == TRIALS for v in stages.values())
            totals[action][label].extend(statistics.median(v) for v in stages.values())
    scores = {action: {k: math.exp(statistics.mean(map(math.log, v))) for k, v in rows.items()}
              for action, rows in totals.items()}
    winners = {action: min(rows, key=lambda label: (rows[label], label))
               for action, rows in scores.items()}
    return dict(winners, scores_seconds=scores, rule=RULE)


def select(args):
    result = choose(args.out)
    path = args.out / 'selection.json'
    if path.exists():
        assert json.loads(path.read_text()) == result, 'frozen selection changed'
    else:
        path.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


def probe(args):
    checks = args.out.parent / 'checks'
    checks.mkdir(exist_ok=True)
    output = args.out / 'sync-probe'
    selected = json.loads((args.out / 'selection.json').read_text())['write']
    for label in ('redb', 'kiban', 'lmdb'):
        c = configuration(label, 10000, label=selected if label == 'redb' else 'kiban-index1')
        for durable in (False, True):
            mode = 'durable' if durable else 'fast'
            config = dict(c, db=str(DATA / 'probe-trial'), durable=durable,
                          action='write', operations=1000)
            trace = checks / f'{label}-{mode}.strace.txt'
            execute(args, config, output, f'{label}-{mode}',
                    tracer=(str(args.strace), '-f', '-c', '-e', 'trace=fsync,fdatasync,msync',
                            '-o', str(trace)))
    dest = DATA / 'crash-trial'
    settings = configuration('redb', 10000, label=selected)
    if (checks / 'crash-receipt.json').exists():
        assert args.resume
        return
    if dest.exists() and args.resume:
        shutil.rmtree(dest)
    subprocess.run(['taskset', '-c', str(CPU), 'cp', '-a', '--reflink=auto',
                    str(fixture(settings)), str(dest)], check=True)
    config = dict(settings, db=str(dest), durable=True, operations=32)
    before = ticks((CPU, SIBLING))
    start = time.time()
    for action in ('crash-write', 'crash-verify'):
        path = checks / f'{action}.config.json'
        path.write_text(json.dumps(dict(config, action=action), indent=2) + '\n')
        cmd = ['taskset', '-c', str(CPU), str(BINARY), str(path)]
        with (checks / f'{action}.log').open('w') as log:
            if action == 'crash-verify':
                subprocess.run(cmd, stdout=log, stderr=log, check=True, timeout=60)
                continue
            # A separate pipe for each direction. The child waits on stdin
            # after the last acknowledged, read-back-verified mutation.
            with subprocess.Popen(cmd, text=True, stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=log) as proc:
                try:
                    for i in range(config['operations']):
                        assert proc.stdout.readline().strip() == f'ACK {config["n"] + i}'
                        log.write(f'ACK {config["n"] + i}\n')
                finally:
                    proc.kill()
                assert proc.wait(timeout=10) == -9
    after = ticks((CPU, SIBLING))
    receipt = dict(acknowledgments=config['operations'], signal='SIGKILL', wait_code=-9,
                   affinity=str(CPU), load=os.getloadavg(), started=start, ended=time.time(),
                   cpu_tick_delta={cpu: [b-a for a, b in zip(before[cpu], after[cpu])]
                                   for cpu in before})
    receipt['verified_records'] = config['n'] + config['operations']
    (checks / 'crash-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    shutil.rmtree(dest)


def provenance(args):
    harness = Path(__file__).parent
    tracked = [
        *sorted((ROOT / 'src').rglob('*.rs')), *sorted((harness / 'src').rglob('*.rs')),
        harness / 'Cargo.lock', harness / 'Cargo.toml', harness / 'redb_bench.py',
        harness / 'rocksdb_bench.py',
        ROOT / 'Cargo.toml', ROOT / 'examples/scale/metrics.rs',
        ROOT / 'examples/support/random.rs',
        *(harness.parent / 'src' / f'{name}.rs' for name in ('api', 'model', 'runner')),
    ]
    protocol_path = STUDY / 'PROTOCOL.md'
    path = str(protocol_path.relative_to(ROOT))
    subprocess.run(['git', 'diff', '--quiet', 'HEAD', '--', path], cwd=ROOT, check=True)
    subprocess.run(['git', 'ls-files', '--error-unmatch', '--', path],
                   cwd=ROOT, stdout=subprocess.DEVNULL, check=True)
    metadata = dict(binary=str(BINARY), binary_sha256=digest(BINARY), cpu=CPU,
                    sibling=SIBLING, seed=SEED, trials=TRIALS, selection_rule=RULE,
                    candidates=CANDIDATES, protocol_sha256=digest(protocol_path),
                    sources={str(p.relative_to(ROOT)): digest(p) for p in tracked})
    path = args.out / 'metadata.json'
    if path.exists():
        assert metadata == json.loads(path.read_text()), 'frozen provenance changed'
    else:
        path.write_text(json.dumps(metadata, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'screen', 'select', 'measure', 'probe'])
    parser.add_argument('--out', type=Path, default=STUDY / 'raw')
    parser.add_argument('--resume', action='store_true')
    parser.add_argument('--strace', type=Path,
                        default=Path(shutil.which('strace') or DATA / 'tools/usr/bin/strace'))
    args = parser.parse_args()
    assert set(os.sched_getaffinity(0)) <= set(range(12, 22)), 'pin coordinator to CPUs 12–21'
    assert os.getpriority(os.PRIO_PROCESS, 0) >= 10, 'run coordinator at nice +10'
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    DATA.mkdir(parents=True, exist_ok=True)
    with (DATA / 'campaign.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert not (args.out / 'INVALID').exists(), 'a failed command invalidated this campaign'
        provenance(args)
        if args.action in ('screen', 'measure'):
            measure(args, args.action == 'screen')
        else:
            {'prepare': prepare, 'select': select, 'probe': probe}[args.action](args)


if __name__ == '__main__':
    main()
