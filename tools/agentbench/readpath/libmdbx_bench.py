#!/usr/bin/env python3
"""Prepare, screen and benchmark Kiban, libmdbx and LMDB on CPU 10."""
import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import shutil
import signal
import statistics
import subprocess
import time

from rocksdb_bench import ROOT, SEED, base, digest, rotate, run, ticks

BINARY = ROOT / 'target/libmdbx-h2h/release/kiban-readbench'
DATA = ROOT / 'target/libmdbx-data'
VALIDATION = ROOT / 'validation/libmdbx'
TRIALS = 5
BASELINE = dict(page_bytes=4096, growth_bytes=16 << 20, writemap=False,
                no_rdahead=False, liforeclaim=False)
CANDIDATES = {
    'baseline': BASELINE,
    'growth-1m': dict(BASELINE, growth_bytes=1 << 20),
    'lifo': dict(BASELINE, liforeclaim=True),
    'no-readahead': dict(BASELINE, no_rdahead=True),
    'page-16k': dict(BASELINE, page_bytes=16384),
    'writemap': dict(BASELINE, writemap=True),
    'writemap-16k': dict(BASELINE, writemap=True, page_bytes=16384),
}
RULE = ('Minimize the equal-workload geometric mean of five-trial medians of mean call seconds. '
        'Reads include recall-by-id and recent-for-agent on the single mmap layout. '
        'Writes include remember, update and mixed-90-10 in fast and durable mode. '
        'Select one complete configuration per family; break ties by label.')


def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def load(path):
    return json.loads(path.read_text())


def rows(path):
    return list(map(json.loads, path.read_text().splitlines()))


def config(engine, n, shape='bulk', cache=None, **updates):
    return dict(base(engine, n, shape, cache), **updates)


def fixture(c):
    engine = c['backend']
    if engine == 'kiban':
        engine += f'-index{int(c["point_index"])}-{c["layout"]}'
    if engine == 'libmdbx':
        engine += '-' + next(label for label, tuning in CANDIDATES.items() if tuning == c['libmdbx'])
    return DATA / f'{c["n"]}-{engine}'


def benchmark(c, out, name, resume):
    out.mkdir(parents=True, exist_ok=True)
    pending = out / (name + '.pending')
    receipts = out / 'commands.jsonl'
    if pending.exists():
        assert receipts.exists() and any(r['name'] == name for r in rows(receipts)), (
            f'{pending}: prior command did not complete. Preserve it and investigate before continuing.')
    else:
        pending.touch()
    run(c, out, name, resume, binary=BINARY, data=DATA, fixture_path=fixture)
    pending.unlink()


def prepare(out, resume):
    for n in (10000, 1000000):
        configs = [config('lmdb', n)]
        configs += [config('libmdbx', n, libmdbx=tuning, durable=True) for tuning in CANDIDATES.values()]
        configs += [config('kiban', n, layout, point_index=index)
                    for layout in (('bulk',) if n == 10000 else ('bulk', 'normal'))
                    for index in (False, True)]
        for c in configs:
            db = fixture(c)
            benchmark(dict(c, action='prepare', db=str(db)), out / 'prepare', db.name, resume)


def scenarios(screen):
    if screen:
        yield 'read', 1000000, 'bulk', None, False, 'read-1000000-single-mmap'
    else:
        for shape in ('bulk', 'normal'):
            for cache in (512, None):
                yield 'read', 1000000, shape, cache, False, f'read-1000000-{shape}-{cache}'
    for durable in (False, True):
        yield 'write', 10000, 'bulk', None, durable, f'write-10000-{"durable" if durable else "fast"}'


def matrix(out, screen):
    selection = None if screen else load(out / 'selection.json')
    for action, n, shape, cache, durable, scenario in scenarios(screen):
        if screen:
            configs = [(label, config('libmdbx', n, libmdbx=tuning)) for label, tuning in CANDIDATES.items()]
        else:
            configs = [(f'kiban-index{int(index)}', config('kiban', n, shape, cache, point_index=index))
                       for index in (False, True)]
            configs.append(('libmdbx-tuned', config('libmdbx', n, libmdbx=CANDIDATES[selection[action]])))
        configs.append(('lmdb', config('lmdb', n)))
        for trial in range(TRIALS):
            for label, c in rotate(configs, trial):
                db = DATA / 'write-trial' if action == 'write' else fixture(c)
                yield scenario, f'r{trial+1}-{label}', dict(
                    c, db=str(db), action=action, durable=durable,
                    operations=1000 if action == 'write' else 100000)


def measure(out, resume, screen):
    for scenario, name, c in matrix(out, screen):
        benchmark(c, out / ('screen' if screen else 'measure') / scenario, name, resume)


def selection_from_screen(out):
    scores = {}
    for action in ('read', 'write'):
        totals = {label: [] for label in CANDIDATES}
        for folder in sorted((out / 'screen').glob(action + '-*')):
            for label in totals:
                stages = {}
                for trial in range(1, TRIALS + 1):
                    for row in rows(folder / f'r{trial}-{label}.jsonl'):
                        if row['type'] != 'stage':
                            continue
                        if action == 'write' and row['workload'] not in ('remember', 'update', 'mixed-90-10'):
                            continue
                        stages.setdefault(row['workload'], []).append(row['call_seconds'] / row['latency']['count'])
                assert len(stages) == (2 if action == 'read' else 3)
                assert all(len(v) == TRIALS for v in stages.values())
                totals[label].extend(statistics.median(v) for v in stages.values())
        assert all(len(v) == (2 if action == 'read' else 6) for v in totals.values())
        scores[action] = {label: statistics.geometric_mean(v) for label, v in totals.items()}
    return {**{action: min(s, key=lambda label: (s[label], label)) for action, s in scores.items()},
            'scores_seconds': scores, 'rule': RULE}


def select(out):
    result = selection_from_screen(out)
    path = out / 'selection.json'
    if path.exists():
        assert load(path) == result, 'frozen selection changed'
    else:
        dump(path, result)
    print(json.dumps(result, indent=2))


def diagnostic_labels(selection):
    # Repeat the original three checks, and cover the new winner as well.
    return dict.fromkeys(('baseline', 'writemap', 'no-readahead', selection['write']))


def diagnostics(out, checks, strace):
    checks.mkdir(parents=True, exist_ok=True)
    selection = load(out / 'selection.json')
    for label in diagnostic_labels(selection):
        c = config('libmdbx', 10000, libmdbx=CANDIDATES[label], durable=True, operations=16)
        diagnostic_label = f'durable-{label}'
        dest = DATA / diagnostic_label
        assert not dest.exists()
        shutil.copytree(fixture(c), dest)
        c['db'] = str(dest)
        c['action'] = 'libmdbx-crash-write'
        writer = checks / (diagnostic_label + '-writer.config.json')
        dump(writer, c)
        trace = checks / (diagnostic_label + '.strace')
        log = checks / (diagnostic_label + '.log')
        before = ticks()
        start = time.time()
        command = ['taskset', '-c', '10', str(strace), '-f', '-y', '-s', '64',
                   '-e', 'trace=clone,clone3,openat,close,write,read,pwrite64,pwritev,pwritev2,fsync,fdatasync,msync',
                   '-o', str(trace), str(BINARY), str(writer)]
        with log.open('w') as output:
            with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=output, text=True, start_new_session=True) as child:
                try:
                    assert child.stdout.readline().strip() == 'ready'
                    # strace is the group leader and its sole child is the writer.
                    children = Path(f'/proc/{child.pid}/task/{child.pid}/children')
                    pid, = map(int, children.read_text().split())
                    tasks = Path(f'/proc/{pid}/task')
                    ready_tids = sorted(int(task.name) for task in tasks.iterdir())
                    assert ready_tids == [pid], ready_tids
                    acknowledgments = []
                    for offset in range(c['operations']):
                        for record in (c['n'] + offset, c['n'] - 1 - offset):
                            child.stdin.write('put\n')
                            child.stdin.flush()
                            ack = child.stdout.readline().strip()
                            assert ack == f'ack {record}', ack
                            acknowledgments.append(record)
                    assert child.poll() is None
                    last_ack_tids = sorted(int(task.name) for task in tasks.iterdir())
                    assert last_ack_tids == [pid], last_ack_tids
                    # Kill the tracee alone so strace can finish.
                    os.kill(pid, signal.SIGKILL)
                    assert child.wait(timeout=10) == -signal.SIGKILL
                finally:
                    if child.poll() is None:
                        os.killpg(child.pid, signal.SIGKILL)
            verify = checks / (diagnostic_label + '-verify.config.json')
            dump(verify, dict(c, action='libmdbx-crash-verify'))
            subprocess.run(['taskset', '-c', '10', str(BINARY), str(verify)],
                           stdout=output, stderr=output, check=True, timeout=60)
        after = ticks()
        dump(checks / (diagnostic_label + '.json'), dict(
            acknowledgments=acknowledgments, trace_sha256=digest(trace), log_sha256=digest(log),
            command=command, killed_pid=pid, signal='SIGKILL',
            ready_tids=ready_tids, last_ack_tids=last_ack_tids,
            cpu_tick_delta={cpu: [b-a for a, b in zip(before[cpu], after[cpu])] for cpu in before},
            affinity='10', started=start, ended=time.time(), load=os.getloadavg()))
        shutil.rmtree(dest)


def host():
    def command(*args):
        return subprocess.check_output(args, text=True).strip()

    def read(path):
        p = Path(path)
        return p.read_text().strip() if p.exists() else None

    return dict(captured_utc=datetime.now(timezone.utc).isoformat(), uname=command('uname', '-a'),
                cpu=command('lscpu'), cpu_topology=command('lscpu', '-e=CPU,CORE,SOCKET,ONLINE'),
                sibling=read('/sys/devices/system/cpu/cpu10/topology/thread_siblings_list'),
                meminfo=read('/proc/meminfo'), memory_max=read('/sys/fs/cgroup/memory.max'),
                cpu10_governor=read('/sys/devices/system/cpu/cpu10/cpufreq/scaling_governor'),
                no_turbo=read('/sys/devices/system/cpu/intel_pstate/no_turbo'),
                filesystem=command('findmnt', '-n', '-o', 'FSTYPE,OPTIONS', '-T', str(DATA)),
                coordinator_affinity=sorted(os.sched_getaffinity(0)),
                coordinator_nice=os.getpriority(os.PRIO_PROCESS, 0),
                initial_ticks=ticks(), load=os.getloadavg(),
                rustc=command('rustc', '-vV'), cargo=command('cargo', '-V'),
                native_cc=command('cc', '--version').splitlines()[0])


def provenance():
    harness = ROOT / 'tools/agentbench/readpath'
    hashed = [
        *sorted((ROOT / 'src').rglob('*.rs')), ROOT / 'Cargo.toml',
        *sorted((harness / 'src').rglob('*.rs')),
        *sorted((ROOT / 'tools/agentbench/src').glob('*.rs')),
        ROOT / 'examples/scale/metrics.rs', ROOT / 'examples/support/random.rs',
        harness / 'Cargo.toml', harness / 'Cargo.lock', harness / 'rocksdb_bench.py',
        harness / 'libmdbx_bench.py', VALIDATION / 'PROTOCOL.md',
    ]
    return dict(engine_head=subprocess.check_output(
                    ['git', 'log', '-1', '--format=%H', '--', 'src'], cwd=ROOT, text=True).strip(),
                binary=str(BINARY), binary_sha256=digest(BINARY), trials=TRIALS, seed=SEED,
                cpu=10, sibling=11, build_test_cpus='12-21', candidates=CANDIDATES, rule=RULE,
                sources={str(path.relative_to(ROOT)): digest(path) for path in hashed})


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('action', choices=('prepare', 'screen', 'select', 'measure', 'diagnostics'))
    parser.add_argument('--out', type=Path, default=VALIDATION / 'raw')
    parser.add_argument('--resume', action='store_true')
    parser.add_argument('--strace', type=Path, default=Path('/usr/bin/strace'))
    args = parser.parse_args()
    assert os.sched_getaffinity(0) <= set(range(12, 22)), 'pin coordinator to 12–21'
    assert os.getpriority(os.PRIO_PROCESS, 0) >= 10, 'run coordinator at nice +10'
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    DATA.mkdir(parents=True, exist_ok=True)
    metadata = out / 'metadata.json'
    if metadata.exists():
        assert load(metadata) == provenance(), 'frozen provenance changed'
    else:
        assert args.action == 'prepare', 'start with fixture preparation'
        dump(metadata, provenance())
        dump(out.parent / 'host.json', host())
    if args.action == 'prepare':
        prepare(out, args.resume)
    elif args.action == 'select':
        select(out)
    elif args.action == 'diagnostics':
        diagnostics(out, out.parent / 'checks', args.strace.resolve())
    else:
        measure(out, args.resume, args.action == 'screen')


if __name__ == '__main__':
    main()
