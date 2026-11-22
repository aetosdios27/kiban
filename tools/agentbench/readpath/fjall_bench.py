#!/usr/bin/env python3
"""Run the frozen Fjall campaign on CPU 6. See validation/fjall/PROTOCOL.md."""
import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import re
import select as io_select
import shutil
import signal
import statistics
import subprocess
import time

from rocksdb_bench import ROOT, SEED, base, digest, disk, rotate

BUILD = ROOT / 'target/fjall-h2h'
DATA = ROOT / 'target/fjall-data'
BINARY = BUILD / 'release/kiban-readbench'
CPU, SIBLING = 6, 7
ROUNDS = 5
MUTATIONS = ('remember', 'update', 'mixed-90-10')
STAGES = {
    'read': ('recall-by-id', 'recent-for-agent'),
    'write': ('remember', 'recall-by-id', 'recent-for-agent', 'update', 'mixed-90-10'),
}
CANDIDATES = {
    'block16': dict(block_size=16384, bloom_bits=10, memtable_mib=64, lz4=False),
    'block4': dict(block_size=4096, bloom_bits=10, memtable_mib=64, lz4=False),
    'bloom6': dict(block_size=4096, bloom_bits=6, memtable_mib=64, lz4=False),
    'mem16': dict(block_size=4096, bloom_bits=10, memtable_mib=16, lz4=False),
}
HASH = dict(hash_ratio=0.75)
PIN = dict(pin_blocks=True)
HIT = dict(expect_point_read_hits=True)
RESTART = dict(restart_interval=8)
SUPPLEMENT = {
    'block4': CANDIDATES['block4'],
    'a-hash': dict(CANDIDATES['block4'], **HASH),
    'b-pin': dict(CANDIDATES['block4'], **PIN),
    'c-hit': dict(CANDIDATES['block4'], **HIT),
    'd-restart': dict(CANDIDATES['block4'], **RESTART),
    'e-hash-pin': dict(CANDIDATES['block4'], **HASH, **PIN),
    'f-hash-pin-hit': dict(CANDIDATES['block4'], **HASH, **PIN, **HIT),
    'g-hash-pin-hit-restart': dict(CANDIDATES['block4'], **HASH, **PIN, **HIT, **RESTART),
}
SUPPLEMENT = dict(sorted(SUPPLEMENT.items()))
RULE = ('Minimize the equal-workload geometric mean of five-trial medians of mean call time, '
        'separately for reads (both operations, layouts and cache budgets) and writes '
        '(remember, update and mixed in both modes). Ties break by candidate label.')


def dump(path, value):
    tmp = path.with_suffix(path.suffix + '.tmp')
    tmp.write_text(json.dumps(value, indent=2) + '\n')
    tmp.replace(path)


def lines(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def ticks():
    return {words[0]: list(map(int, words[1:]))
            for line in Path('/proc/stat').read_text().splitlines()
            if (words := line.split()) and words[0] in (f'cpu{CPU}', f'cpu{SIBLING}')}


def fixture(c):
    label = c['backend']
    if label == 'kiban':
        label += f'-index{int(c["point_index"])}-{c["layout"]}'
    if label == 'fjall':
        t = c['fjall']
        label += f'-{t["block_size"]}bloom{t["bloom_bits"]}mem{t["memtable_mib"]}'
        label += f'-{"lz4" if t["lz4"] else "none"}-{c["layout"]}'
        if 'hash_ratio' in t:
            label += f'-hash{t["hash_ratio"]}'
        if t.get('pin_blocks'):
            label += '-pin'
        if t.get('expect_point_read_hits'):
            label += '-hit'
        if 'restart_interval' in t:
            label += f'-restart{t["restart_interval"]}'
    return DATA / f'{c["n"]}-{label}'


def configs(action, shape, cache, winners=None, compressed_only=False, candidates=CANDIDATES):
    n = 1000000 if action == 'read' else 10000
    fjall = base('fjall', n, shape, cache)
    if winners is None:
        rows = [(label, dict(fjall, fjall=tune)) for label, tune in candidates.items()]
    else:
        chosen = candidates[winners[action]]
        compressed = candidates[winners['read']] if candidates is SUPPLEMENT else chosen
        rows = [('fjall-lz4', dict(fjall, fjall=dict(compressed, lz4=True)))]
        if compressed_only:
            return rows
        rows = [
            (f'kiban-index{int(index)}',
             dict(base('kiban', n, shape, cache), point_index=index, agent_memory=True))
            for index in (False, True)
        ] + [('fjall-tuned', dict(fjall, fjall=chosen)), *rows]
    return [*rows, ('lmdb', base('lmdb', n, shape, cache))]


def scenarios():
    for shape in ('bulk', 'normal'):
        for cache in (512, None):
            yield f'read-{shape}-{cache or "default"}', 'read', shape, cache, False
    for durable in (False, True):
        yield f'write-{"durable" if durable else "fast"}', 'write', 'bulk', None, durable


def table_files(c, footprint):
    return {name: size for name, size in footprint['files'].items()
            if ('/tables/' in name if c['backend'] == 'fjall' else name.endswith('.sst'))}


def audit_command(folder, receipt):
    name = receipt['name']
    for extension, key in [('config.json', 'config_sha256'), ('log', 'log_sha256'),
                           ('time.txt', 'time_sha256'), ('jsonl', 'result_sha256')]:
        if key in receipt:
            assert digest(folder / f'{name}.{extension}') == receipt[key], (folder, name, key)
    c = json.loads((folder / f'{name}.config.json').read_text())
    assert receipt['affinity'] == CPU
    assert receipt['ended'] >= receipt['started']
    assert set(receipt['cpu_tick_delta']) == {f'cpu{CPU}', f'cpu{SIBLING}'}
    if c['action'] == 'prepare':
        log = (folder / f'{name}.log').read_text()
        assert f'"verified_records":{c["n"]}' in log and '"type":"prepared"' in log
        return c, []
    rows = lines(folder / f'{name}.jsonl')
    assert all(r.get('errors', 0) == 0 for r in rows)
    stages = [r for r in rows if r['type'] == 'stage']
    assert tuple(r['workload'] for r in stages) == STAGES[c['action']]
    assert all(r['latency']['count'] == c['operations'] and r['call_seconds'] > 0
               and r['verified_ops_s'] > 0 for r in stages)
    complete = [r for r in rows if r['type'] == 'complete']
    population = c['n'] + (c['operations'] if c['action'] == 'write' else 0)
    assert len(complete) == 1 and complete[0]['verified_records'] == population
    if c['action'] == 'write':
        reopened = [r for r in rows if r['type'] == 'reopen_verified']
        assert len(reopened) == 1 and reopened[0]['verified_records'] == population
        expected = [c['operations'], 0, 0, c['operations'], c['operations'] // 10]
        assert [row['writes']['count'] for row in stages] == expected
    status = next(r['proc_status'] for r in rows if 'proc_status' in r)
    assert re.search(rf'^Cpus_allowed_list:\s+{CPU}$', status, re.M), status
    return c, rows


def run(c, folder, name, probe=False, trace=()):
    folder.mkdir(parents=True, exist_ok=True)
    receipt_path = folder / 'commands.jsonl'
    journal = {r['name']: r for r in lines(receipt_path)} if receipt_path.exists() else {}
    path = folder / (name + '.config.json')
    output = folder / (name + '.jsonl')
    log_path = folder / (name + '.log')
    timing = folder / (name + '.time.txt')
    source = fixture(c)
    db = DATA / 'write-trial' if c['action'] == 'write' else source
    c = dict(c, output=str(output), db=str(db))
    if name in journal:
        previous, _ = audit_command(folder, journal[name])
        assert previous == c, 'saved command configuration changed'
        return
    assert not path.exists(), f'unreceipted command {path} invalidates this campaign'
    if c['action'] == 'write':
        if db.exists():
            shutil.rmtree(db)
        subprocess.run(['cp', '-a', '--reflink=auto', source, db], check=True)
    before_files = table_files(c, disk(db)) if c['action'] == 'read' else None
    dump(path, c)
    print(f'{folder.name}/{name}', flush=True)
    env = dict(os.environ)
    if probe:
        env['LD_PRELOAD'] = str(BUILD / 'sync-probe.so')
    before, started = ticks(), time.time()
    with log_path.open('w') as log:
        subprocess.run(['taskset', '-c', str(CPU), '/usr/bin/time', '-v', '-o', timing, *trace, BINARY, path],
                       stdout=log, stderr=log, env=env, check=True)
    after, ended = ticks(), time.time()
    footprint = disk(db)
    if before_files is not None:
        assert table_files(c, footprint) == before_files, f'fixture tables changed: {db}'
    receipt = dict(name=name, started=started, ended=ended, affinity=CPU, nice=os.nice(0),
                   load=os.getloadavg(), diagnostic=probe or bool(trace), disk=footprint,
                   cpu_tick_delta={k: [y - x for x, y in zip(before[k], after[k])] for k in before},
                   config_sha256=digest(path), log_sha256=digest(log_path), time_sha256=digest(timing))
    if c['action'] != 'prepare':
        receipt['result_sha256'] = digest(output)
    audit_command(folder, receipt)
    with receipt_path.open('a') as stream:
        stream.write(json.dumps(receipt) + '\n')
        stream.flush()
        os.fsync(stream.fileno())
    if c['action'] == 'write':
        shutil.rmtree(db)


def prepare(raw, winners=None, candidates=CANDIDATES):
    prepared = set()
    for _, action, shape, cache, _ in scenarios():
        rows = configs(action, shape, cache, winners, compressed_only=winners is not None,
                       candidates=candidates)
        if winners is None:
            rows += configs(action, shape, cache, {'read': 'block4', 'write': 'block4'})[:2]
        for _, c in rows:
            name = fixture(c).name
            if name not in prepared:
                run(dict(c, action='prepare', cache_mib=None), raw / 'prepare', name)
                prepared.add(name)


def planned(winners, candidates=CANDIDATES):
    for scenario, action, shape, cache, durable in scenarios():
        rows = configs(action, shape, cache, winners, candidates=candidates)
        for trial in range(ROUNDS):
            for label, c in rotate(rows, trial):
                yield scenario, f'r{trial + 1}-{label}', dict(
                    c, action=action, durable=durable, operations=100000 if action == 'read' else 1000)


def phase_folder(raw, winners, candidates):
    if winners is not None and candidates is SUPPLEMENT:
        return raw.parent / 'measure'
    return raw / ('screen' if winners is None else 'measure')


def measure(raw, winners=None, candidates=CANDIDATES):
    phase = phase_folder(raw, winners, candidates)
    for scenario, name, c in planned(winners, candidates):
        run(c, phase / scenario, name)


def campaign_rows(raw, winners=None, candidates=CANDIDATES):
    records = {}
    seen = {}
    fixtures = {r['name']: r for r in lines(raw / 'prepare/commands.jsonl')}
    phase = phase_folder(raw, winners, candidates)
    for scenario, name, c in planned(winners, candidates):
        folder = phase / scenario
        if scenario not in records:
            saved = lines(folder / 'commands.jsonl')
            records[scenario] = {r['name']: r for r in saved}
            assert len(saved) == len(records[scenario]), 'duplicate command receipts'
            seen[scenario] = []
        receipt = records[scenario][name]
        seen[scenario].append(name)
        saved, rows = audit_command(folder, receipt)
        assert all(saved[k] == v for k, v in c.items() if k not in ('db', 'output'))
        initial_footprint = fixtures[fixture(c).name]['disk']
        if c['action'] == 'read':
            assert table_files(c, receipt['disk']) == table_files(c, initial_footprint)
        yield scenario, name, c, receipt, rows
    assert set(p.name for p in phase.iterdir()) == set(records)
    for scenario, order in seen.items():
        assert order == list(records[scenario]), f'rotation or coverage differs: {scenario}'
        folder = phase / scenario
        for extension in ('config.json', 'log', 'time.txt', 'jsonl'):
            expected = {f'{name}.{extension}' for name in order}
            if extension == 'jsonl':
                expected.add('commands.jsonl')
            assert {p.name for p in folder.glob(f'*.{extension}')} == expected


def select(raw, candidates=CANDIDATES):
    samples = {}
    for scenario, name, c, _, rows in campaign_rows(raw, candidates=candidates):
        _, label = name.split('-', 1)
        if label not in candidates:
            continue
        action = c['action']
        for row in rows:
            if row['type'] != 'stage' or (action == 'write' and row['workload'] not in MUTATIONS):
                continue
            key = (action, label, scenario, row['workload'])
            samples.setdefault(key, []).append(row['call_seconds'] / row['latency']['count'])
    assert all(len(values) == ROUNDS for values in samples.values())
    scores = {}
    for action, count in [('read', 8), ('write', 6)]:
        scores[action] = {}
        for label in candidates:
            medians = [statistics.median(v) for key, v in samples.items() if key[:2] == (action, label)]
            assert len(medians) == count
            scores[action][label] = math.exp(statistics.mean(map(math.log, medians)))
    winners = {action: min(s, key=lambda label: (s[label], label)) for action, s in scores.items()}
    return dict(winners, rule=RULE, scores_seconds=scores)


def probe(checks, winners, candidates=CANDIDATES):
    summary = {}
    for durable in (False, True):
        for label, c in configs('write', 'bulk', None, winners, candidates=candidates):
            if label not in ('kiban-index1', 'fjall-tuned', 'lmdb'):
                continue
            name = f'{label}-{"durable" if durable else "fast"}'
            folder = checks / 'sync'
            run(dict(c, action='write', durable=durable, operations=1000), folder, name, probe=True)
            match = re.search(r'^sync_probe fsync=(\d+) fdatasync=(\d+) failures=(\d+)$',
                              (folder / (name + '.log')).read_text(), re.M)
            assert match, f'missing sync probe result for {name}'
            summary[name] = dict(zip(('fsync', 'fdatasync', 'failures'), map(int, match.groups())))
    for label in ('kiban-index1', 'fjall-tuned', 'lmdb'):
        slow, fast = (summary[f'{label}-{mode}'] for mode in ('durable', 'fast'))
        assert slow['failures'] == fast['failures'] == 0
        assert slow['fdatasync'] - fast['fdatasync'] == 2100, label
    dump(checks / 'sync.json', summary)

    db = DATA / 'crash-trial'
    assert not db.exists()
    _, c = next(row for row in configs('write', 'bulk', None, winners, candidates=candidates) if row[0] == 'fjall-tuned')
    shutil.copytree(fixture(c), db)
    path = checks / 'crash.config.json'
    c = dict(c, action='crash-write', durable=True, operations=25, db=str(db))
    dump(path, c)
    before, started = ticks(), time.time()
    with (checks / 'crash.stderr.log').open('wb') as err, (checks / 'crash.acks.log').open('wb') as ack:
        p = subprocess.Popen(['taskset', '-c', str(CPU), BINARY, path],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err, bufsize=0)
        try:
            for id in range(c['n'], c['n'] + c['operations']):
                assert io_select.select([p.stdout], [], [], 30)[0], 'timed out waiting for ack'
                line = p.stdout.readline()
                ack.write(line)
                assert line == f'ack {id}\n'.encode(), line
        finally:
            p.kill()
            status = p.wait()
            p.stdin.close()
            p.stdout.close()
    assert status == -signal.SIGKILL
    recovering = checks / 'crash-reopen.config.json'
    output = checks / 'crash-reopen.jsonl'
    dump(recovering, dict(c, action='read', n=c['n'] + c['operations'], operations=1, output=str(output)))
    with (checks / 'crash-reopen.log').open('wb') as log:
        subprocess.run(['taskset', '-c', str(CPU), BINARY, recovering], stdout=log, stderr=log, check=True)
    assert lines(output)[-1]['verified_records'] == c['n'] + c['operations']
    after = ticks()
    dump(checks / 'crash.json', dict(acknowledged=c['operations'], returncode=status,
                                    verified_records=c['n'] + c['operations'], cpu=CPU,
                                    load=os.getloadavg(), seconds=time.time() - started,
                                    cpu_tick_delta={k: [y-x for x, y in zip(before[k], after[k])] for k in before}))
    shutil.rmtree(db)


def provenance(candidates=CANDIDATES):
    tracked = subprocess.check_output([
        'git', 'ls-files', 'src', 'Cargo.toml', 'tools/agentbench/src', 'tools/agentbench/readpath/src',
        'tools/agentbench/readpath/Cargo.toml', 'tools/agentbench/readpath/Cargo.lock',
        'tools/agentbench/readpath/fjall_bench.py', 'tools/agentbench/readpath/rocksdb_bench.py',
        'tools/agentbench/readpath/rocksdb_sync_probe.c', 'examples/scale/metrics.rs',
        'examples/support/random.rs', 'validation/fjall/PROTOCOL.md',
    ], cwd=ROOT, text=True).splitlines()
    return dict(binary=str(BINARY), binary_sha256=digest(BINARY), seed=SEED, trials=ROUNDS,
                engine_commit=subprocess.check_output(['git', 'log', '-1', '--format=%H', '--', 'src'],
                                                      cwd=ROOT, text=True).strip(),
                cpu=CPU, sibling=SIBLING, coordinator_affinity=sorted(os.sched_getaffinity(0)),
                nice=os.nice(0), candidates=candidates, rule=RULE,
                sources={p: digest(ROOT / p) for p in tracked})


def host_info(raw, host):
    def command(*args):
        return subprocess.check_output(args, cwd=ROOT, text=True).strip()

    config = raw / 'runtime.config.json'
    dump(config, dict(base('kiban', 10000), action='metadata'))
    runtime = json.loads(command('taskset', '-c', str(CPU), str(BINARY), str(config)))
    topology = Path(f'/sys/devices/system/cpu/cpu{CPU}/topology/thread_siblings_list')
    assert topology.read_text().strip() == f'{CPU}-{SIBLING}'
    fs = command('findmnt', '-T', str(DATA), '-n', '-o', 'FSTYPE,OPTIONS')
    data = dict(uname=command('uname', '-srm'), lscpu=json.loads(command('lscpu', '--json')),
                thread_siblings=topology.read_text().strip(), filesystem=fs,
                load=os.getloadavg(), memory=Path('/proc/meminfo').read_text().splitlines()[0],
                glibc=command('getconf', 'GNU_LIBC_VERSION'), runtime=runtime,
                scaling_driver=Path(f'/sys/devices/system/cpu/cpu{CPU}/cpufreq/scaling_driver').read_text().strip(),
                governor=Path(f'/sys/devices/system/cpu/cpu{CPU}/cpufreq/scaling_governor').read_text().strip(),
                no_turbo=Path('/sys/devices/system/cpu/intel_pstate/no_turbo').read_text().strip())
    if (BUILD / 'build-environment.json').exists():
        data['build'] = json.loads((BUILD / 'build-environment.json').read_text())
    dump(host, data)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('phase', choices=('prepare', 'screen', 'select', 'prepare-lz4', 'measure', 'probe'))
    parser.add_argument('--supplement', action='store_true')
    parser.add_argument('--out', type=Path, default=ROOT / 'validation/fjall/raw')
    args = parser.parse_args()
    allowed = range(13 if args.supplement else 12, 22)
    assert set(os.sched_getaffinity(0)) <= set(allowed) and os.nice(0) >= 10
    root_raw = args.out.resolve()
    candidates = SUPPLEMENT if args.supplement else CANDIDATES
    raw = root_raw / 'screen-supplement' if args.supplement else root_raw
    host = raw / 'host.json' if args.supplement else raw.parent / 'host.json'
    raw.mkdir(parents=True, exist_ok=True)
    DATA.mkdir(parents=True, exist_ok=True)
    with (DATA / '.coordinator-lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        source = provenance(candidates)
        frozen = raw / 'metadata.json'
        if frozen.exists():
            assert json.loads(frozen.read_text()) == source, 'frozen binary or source changed'
        else:
            assert args.phase == 'prepare'
            dump(frozen, source)
            host_info(raw, host)
        if args.phase == 'prepare':
            prepare(raw, candidates=candidates)
        elif args.phase == 'screen':
            measure(raw, candidates=candidates)
        elif args.phase == 'select':
            chosen = select(raw, candidates)
            path = raw / 'selection.json'
            if path.exists():
                assert json.loads(path.read_text()) == chosen
            else:
                dump(path, chosen)
            print(json.dumps(chosen, indent=2))
        else:
            winners = json.loads((raw / 'selection.json').read_text())
            if args.phase == 'prepare-lz4':
                prepare(raw, winners, candidates)
            elif args.phase == 'measure':
                measure(raw, winners, candidates)
            elif args.phase == 'probe':
                probe(root_raw.parent / 'checks', winners, candidates)


if __name__ == '__main__':
    main()
