#!/usr/bin/env python3
"""Audit the frozen libmdbx campaign and regenerate the detailed result tables."""
import argparse
from collections import defaultdict
import csv
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import subprocess

from libmdbx_bench import (
    CANDIDATES, ROOT, SEED, TRIALS, VALIDATION, diagnostic_labels, digest, dump, load, matrix, rows,
    selection_from_screen,
)

from libmdbx_reads import MAX_ATTEMPTS, major_faults, rounds

READ = {'recall-by-id': (1, 0), 'recent-for-agent': (1, 0)}
WRITE = {'remember': (0, 1000), 'recall-by-id': (1000, 0), 'recent-for-agent': (1000, 0),
         'update': (0, 1000), 'mixed-90-10': (900, 100)}
LABELS = ('kiban-index0', 'kiban-index1', 'libmdbx-tuned', 'lmdb')
COMPARATORS = ('libmdbx-tuned', 'lmdb')


def stats(values):
    values = list(values)
    assert len(values) == TRIALS
    return dict(median=statistics.median(values), min=min(values), max=max(values), values=values)


def fmt(stat, scale=1):
    return '{:.3f} [{:.3f}, {:.3f}]'.format(*(stat[k] / scale for k in ('median', 'min', 'max')))


def logs(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.startswith('{')]


def receipt(folder, r):
    name = r['name']
    assert r['affinity'] == '10' and not r['diagnostic']
    assert r['ended'] >= r['started']
    assert r['config'] == name + '.config.json'
    for suffix, key in (('.config.json', 'config'), ('.log', 'log'), ('.time.txt', 'time')):
        assert digest(folder / (name + suffix)) == r[key + '_sha256']
    rss_text = (folder / (name + '.time.txt')).read_text()
    assert 'Exit status: 0' in rss_text
    rss = re.search(r'Maximum resident set size \(kbytes\):\s*(\d+)', rss_text)
    assert rss
    assert sum(r['disk']['files'].values()) == r['disk']['logical_bytes']
    return load(folder / r['config']), logs(folder / (name + '.log')), int(rss[1])


def backend(log, c, budgets):
    requests = [row for row in log if row['type'] == 'requested_config']
    assert requests
    use_default = c['cache_mib'] is None
    for r in requests:
        for key in ('backend', 'point_index', 'durable'):
            assert r[key] == c[key]
        assert not r['binary_fuse_filter']
        if use_default:
            budgets.add(r['cache_bytes'])
        else:
            assert r['cache_bytes'] == c['cache_mib'] << 20
    configs = [row for row in log if row['type'] == 'backend_config']
    if c['backend'] == 'lmdb':
        assert configs and all(r['details']['heed3'] == '0.22.0' for r in configs)
        flags = 0 if c['durable'] else 0x50000
        assert all(r['details']['lmdb_flags_bits'] == flags for r in configs)
        assert all(r['details']['map_size_bytes'] == 10 << 30 for r in configs)
    if c['backend'] == 'libmdbx':
        assert configs
        for r in configs:
            t = c['libmdbx']
            assert r['crate'] == 'mdbx-sys 14.4.0' and r['version'] == '0.14.4' and r['tuning'] == t
            assert r['sync'] == ('Durable' if c['durable'] else 'SafeNoSync')
            assert r['flags'] & 0x200000  # Preserve NOSTICKYTHREADS.
            assert r['flags'] & 0x150000 == (0 if c['durable'] else 0x50000)
            for mask, knob in ((0x80000, 'writemap'), (0x800000, 'no_rdahead'), (0x4000000, 'liforeclaim')):
                assert bool(r['flags'] & mask) == t[knob], (r['flags'], knob)
            assert r['page_bytes'] == t['page_bytes'] and r['geometry']['growth'] == t['growth_bytes']
            assert r['geometry']['min'] == 1 << 20 and r['geometry']['max'] == 10 << 30
            assert r['geometry']['shrink'] == 0
    return configs


def layouts(phase, budgets, binary=None):
    fixtures = {}
    for r in rows(phase / 'commands.jsonl'):
        c, log, _ = receipt(phase, r)
        assert c['action'] == 'prepare' and c['seed'] == SEED and c['k'] == 10
        if binary:
            audit_memory(r, c, binary)
        name = r['name']
        assert Path(c['db']).name == name and name not in fixtures
        assert all(row.get('errors', 0) == 0 for row in log)
        result, = [row for row in log if row['type'] == 'prepared']
        assert result['verified_records'] == c['n']
        configs = backend(log, c, budgets) if any(row['type'] == 'requested_config' for row in log) else []
        if c['backend'] == 'libmdbx':
            assert c['durable'] and configs[-1]['entries'] == c['n']
            assert c['libmdbx'] == CANDIDATES[name.split('-', 2)[2]]
        fixtures[name] = dict(layout=result['layout'], config=configs[-1] if configs else None,
                              disk=r['disk'])
    return fixtures


def audit_memory(r, c, binary):
    assert r['command'] == [
        '/usr/bin/time', '-v', '-o', str(Path(c['output']).with_suffix('.time.txt')),
        'taskset', '-c', '10', binary,
        str(Path(c['output']).with_suffix('.config.json')),
    ]
    totals = []
    for key in ('memory_before', 'memory_after'):
        sample = r[key]
        assert type(sample['mem_available_kib']) is int and sample['mem_available_kib'] > 0
        fields = r' avg10=\d+\.\d+ avg60=\d+\.\d+ avg300=\d+\.\d+ total=(\d+)\n'
        pressure = re.fullmatch('some' + fields + 'full' + fields, sample['pressure'])
        assert pressure
        totals.append(tuple(map(int, pressure.groups())))
    assert all(before <= after for before, after in zip(*totals, strict=True))


def commands_in(folder, expected):
    receipts = rows(folder / 'commands.jsonl')
    assert [r['name'] for r in receipts] == [name for name, _ in expected], f'Rotation: {folder}'
    assert not list(folder.glob('*.pending'))
    return [(folder, r, c) for r, (_, c) in zip(receipts, expected, strict=True)]


def quiet_commands(raw):
    phase = raw / 'measure-reads-quiet'
    assert not (phase / 'in-progress').exists()
    accepted, discarded = defaultdict(list), defaultdict(list)
    attempts, exhausted = [], []
    expected_folders = set()
    for scenario, trial, commands in rounds(raw):
        folder = phase / scenario / trial
        if folder.exists():
            limit = load(folder / 'gate.json')['attempt']
        else:
            exhausted.append(dict(scenario=scenario, trial=trial))
            limit = MAX_ATTEMPTS
        assert 1 <= limit <= MAX_ATTEMPTS
        for attempt in range(1, limit + 1):
            keep = folder.exists() and attempt == limit
            source = folder if keep else phase / 'discarded' / scenario / f'{trial}-a{attempt}'
            expected_folders.add(source)
            faults = {name: major_faults(source / f'{name}.time.txt') for name, _ in commands}
            assert load(source / 'gate.json') == dict(attempt=attempt, major_faults=faults)
            assert any(faults.values()) == (not keep), source
            group = commands_in(source, commands)
            (accepted if keep else discarded)[scenario].extend(group)
            if not keep:
                attempts.append(dict(scenario=scenario, trial=trial, attempt=attempt, major_faults=faults))
    assert {p.parent for p in phase.rglob('gate.json')} == expected_folders
    prepared = {phase / 'prepare'} if (phase / 'prepare/commands.jsonl').exists() else set()
    assert {p.parent for p in phase.rglob('commands.jsonl')} == expected_folders | prepared
    return accepted, discarded, attempts, exhausted


def audit_matrix(raw, phase_name, fixtures, budgets, commands=None, binary=None, summarize=True):
    if commands is None:
        screen = phase_name == 'screen'
        archived = phase_name == 'superseded-read-faults'
        phase = (raw.parent if archived else raw) / phase_name
        expected = defaultdict(list)
        for scenario, name, c in matrix(raw, screen):
            if screen or (c['action'] == 'read') == archived:
                expected[scenario].append((name, c))
        assert set(expected) == {p.name for p in phase.iterdir() if p.is_dir()}
        commands = {scenario: commands_in(phase / scenario, wanted) for scenario, wanted in expected.items()}
    command_rows = []
    values = {}
    trial_rows = []
    for scenario, records in commands.items():
        by_engine = defaultdict(dict)
        for folder, r, want in records:
            name = r['name']
            c, log, rss = receipt(folder, r)
            if binary:
                audit_memory(r, c, binary)
            for key in want.keys() - {'db', 'output'}:
                assert c[key] == want[key], (folder, name, key)
            trial, label = name.split('-', 1)
            assert Path(c['db']).name == Path(want['db']).name
            path = folder / (name + '.jsonl')
            assert digest(path) == r['result_sha256']
            result = rows(path)
            assert all(row.get('errors', 0) == 0 for row in result)
            assert len(result) == (3 if c['action'] == 'read' else 8)
            status, = [row['proc_status'] for row in result if 'proc_status' in row]
            assert re.search(r'^Cpus_allowed_list:\s*10$', status, re.M)
            desired = ({k: (c['operations'], 0) for k in READ} if c['action'] == 'read' else WRITE)
            complete, = [row for row in result if row['type'] == 'complete']
            stages = [row for row in result if row['type'] == 'stage']
            assert [s['workload'] for s in stages] == list(desired)
            configs = backend(log, c, budgets)
            if c['action'] == 'read':
                assert complete['verified_records'] == c['n'] and complete['timed_writes'] == 0
                fixture = fixtures[Path(c['db']).name]
                assert complete['layout'] == fixture['layout']
                assert r['disk']['files'] == fixture['disk']['files']
                if c['backend'] == 'libmdbx':
                    for key in ('entries', 'txnid', 'geometry', 'leaf_pages', 'branch_pages', 'overflow_pages'):
                        assert configs[0][key] == fixture['config'][key]
            else:
                reopened, = [row for row in result if row['type'] == 'reopen_verified']
                assert complete['verified_records'] == reopened['verified_records'] == c['n'] + c['operations']
            for stage in stages:
                workload = stage['workload']
                read_count, write_count = desired[workload]
                latency = stage['latency']
                count = latency['count']
                assert (stage['reads']['count'], stage['writes']['count']) == (read_count, write_count)
                assert count == read_count + write_count == c['operations']
                assert latency['p50_ns'] <= latency['p99_ns'] and stage['call_seconds'] > 0
                assert math.isclose(stage['call_ops_s'], count / stage['call_seconds'])
                assert math.isclose(stage['verified_ops_s'], count / stage['wall_seconds'])
                data = dict(p50_ns=latency['p50_ns'], p99_ns=latency['p99_ns'],
                            mean_ns=stage['call_seconds'] * 1e9 / count,
                            call_ops_s=stage['call_ops_s'], verified_ops_s=stage['verified_ops_s'],
                            logical_bytes=r['disk']['logical_bytes'], allocated_bytes=r['disk']['allocated_bytes'],
                            peak_rss_kib=rss, count=count)
                by_engine[workload, label][trial] = data
                trial_rows.append(dict(phase=phase_name, scenario=scenario, trial=trial,
                                       engine=label, workload=workload, **data))
            command_rows.append(r)
        if not summarize or (binary and len(records) != TRIALS * len(LABELS)):
            continue
        grouped = {}
        for (workload, label), trials in by_engine.items():
            assert set(trials) == {f'r{i}' for i in range(1, TRIALS + 1)}
            metrics = {k: stats(t[k] for t in trials.values()) for k in next(iter(trials.values()))}
            ratios = {}
            for cmp in COMPARATORS:
                control = by_engine.get((workload, cmp))
                if control:
                    ratios[cmp] = {k: stats(trials[t][k] / control[t][k] for t in trials)
                                   for k in ('p50_ns', 'p99_ns', 'mean_ns', 'call_ops_s', 'verified_ops_s')}
            grouped.setdefault(workload, {})[label] = dict(metrics=metrics, ratios=ratios)
        values[scenario] = grouped
    return values, command_rows, trial_rows


def durabilities(checks, selection):
    verified = {}
    for label in diagnostic_labels(selection):
        base = checks / f'durable-{label}'
        metadata = load(base.with_suffix('.json'))
        trace = base.with_suffix('.strace')
        log = base.with_suffix('.log')
        assert digest(trace) == metadata['trace_sha256'] and digest(log) == metadata['log_sha256']
        assert metadata['affinity'] == '10' and metadata['signal'] == 'SIGKILL'
        expected = [i for offset in range(16) for i in (10000 + offset, 9999 - offset)]
        assert metadata['acknowledgments'] == expected
        result, = [r for r in logs(log) if r['type'] == 'crash_verified']
        assert result['records'] == 10016 and result['mutations'] == 32
        pid = metadata['killed_pid']
        assert metadata['ready_tids'] == metadata['last_ack_tids'] == [pid]
        trace_lines = trace.read_text().splitlines()
        assert f'{pid} +++ killed by SIGKILL +++' in trace_lines
        assert all(line.startswith(f'{pid} ') for line in trace_lines)
        assert not any(re.search(r'\bclone3?\(', line) for line in trace_lines)

        dsync_fds = set()
        mechanisms = defaultdict(int)
        per_ack = []
        current = set()
        ready = False
        for line in trace_lines:
            open_fd = re.search(r'openat\(.*mdbx\.dat".*O_(?:D)?SYNC.*\)\s+= (\d+)<[^>]*mdbx\.dat>', line)
            if open_fd:
                dsync_fds.add(int(open_fd[1]))
            close_fd = re.search(r'close\((\d+)<[^>]*mdbx\.dat>\)\s+= 0$', line)
            if close_fd:
                dsync_fds.discard(int(close_fd[1]))
            if re.search(r'write\(1<[^>]*>, "ready\\n"', line):
                ready = True
                continue
            if not ready:
                continue
            sync = re.search(r'\b(fsync|fdatasync)\(\d+<[^>]*mdbx\.dat>\)\s+= 0$', line)
            if sync:
                current.add(sync[1])
            if re.search(r'\bmsync\(.*MS_SYNC\)\s+= 0$', line):
                current.add('msync(MS_SYNC)')
            write = re.search(r'\b(?:write|pwrite64|pwritev2?)\((\d+)<[^>]*mdbx\.dat>.*= [1-9]\d*$', line)
            if write and (int(write[1]) in dsync_fds or 'RWF_DSYNC' in line or 'RWF_SYNC' in line):
                current.add('write-through')
            ack = re.search(r'write\(1<[^>]*>, "ack (\d+)\\n"', line)
            if ack:
                assert current, f'{label}: no durable I/O before ack {ack[1]}'
                per_ack.append(int(ack[1]))
                for mechanism in current:
                    mechanisms[mechanism] += 1
                current.clear()
        assert per_ack == expected
        verified[label] = dict(acks_with_sync=len(per_ack), verified_records=result['records'],
                               native_mechanisms=dict(sorted(mechanisms.items())),
                               writer_tid=pid, writer_thread_count=1)
    return verified


def details(phase, values):
    lines = [f'# {phase.capitalize()} results', '',
             'Every cell is the median [min, max] of five trials. Ratios use corresponding rounds.', '']
    for scenario, workloads in values.items():
        lines += [f'## {scenario}', '',
                  '| Workload | Engine | p50 µs | p99 µs | Mean µs | Call kops/s | Verified kops/s |',
                  '|---|---|---:|---:|---:|---:|---:|']
        for work, engines in workloads.items():
            for engine, data in engines.items():
                m = data['metrics']
                cells = [fmt(m[k], 1000) for k in ('p50_ns', 'p99_ns', 'mean_ns', 'call_ops_s', 'verified_ops_s')]
                lines.append('| ' + ' | '.join([work, engine, *cells]) + ' |')
        lines += ['', '| Engine | Logical MiB | Allocated MiB | Peak RSS MiB |',
                  '|---|---:|---:|---:|']
        for engine, data in next(iter(workloads.values())).items():
            m = data['metrics']
            lines.append('| ' + ' | '.join([engine, fmt(m['logical_bytes'], 1 << 20),
                                          fmt(m['allocated_bytes'], 1 << 20), fmt(m['peak_rss_kib'], 1024)]) + ' |')
        lines.append('')
        if phase == 'measure':
            lines += ['| Workload | Engine | Comparator | p50 ratio | p99 ratio | Mean ratio | Call rate ratio | Verified rate ratio |',
                      '|---|---|---|---:|---:|---:|---:|---:|']
            for work, engines in workloads.items():
                for engine, data in engines.items():
                    for cmp, r in data['ratios'].items():
                        if engine == cmp:
                            continue
                        cells = [fmt(r[k]) for k in ('p50_ns', 'p99_ns', 'mean_ns', 'call_ops_s', 'verified_ops_s')]
                        lines.append('| ' + ' | '.join([work, engine, cmp, *cells]) + ' |')
            lines.append('')
    return '\n'.join(lines)


def headline(values):
    lines = ['Cells show **p50 µs / p99 µs / call kops/s**, medians across five rounds.', '',
             '| Scenario | Operation | Kiban index off | Kiban agent-memory | libmdbx tuned | LMDB |',
             '|---|---|---:|---:|---:|---:|']
    for scenario, workloads in values.items():
        for work, group in workloads.items():
            cells = []
            for label in LABELS:
                m = group[label]['metrics']
                cells.append(' / '.join(f'{m[k]["median"]/1e3:.3f}' for k in ('p50_ns', 'p99_ns', 'call_ops_s')))
            lines.append('| ' + ' | '.join([scenario, work, *cells]) + ' |')
    return '\n'.join(lines) + '\n'


def spread(values):
    values = list(values)
    return dict(median=statistics.median(values), min=min(values), max=max(values)) if values else None


def sibling_busy(receipt):
    user, nice, system, idle, iowait, irq, softirq, steal = receipt['cpu_tick_delta']['cpu11'][:8]
    busy = user + nice + system + irq + softirq + steal
    total = busy + idle + iowait
    return 100 * busy / total if total else 0


def gate_details(attempts, exhausted, read_count, old_reads, reads):
    scenario = 'read-1000000-bulk-None'
    work = 'recall-by-id'
    old = old_reads[scenario][work]['libmdbx-tuned']['metrics']
    lines = ['## Read page-fault gate', '',
             f'{read_count} accepted commands recorded zero major faults in GNU time. '
             'The earlier primary recorded 2,177,499 libmdbx faults in 12 of 20 commands. '
             '[Superseded receipts](superseded-read-faults/) · [Accepted receipts](raw/measure-reads-quiet/)', '',
             f'{len(attempts)} discarded attempts.']
    if scenario in reads:
        new = reads[scenario][work]['libmdbx-tuned']['metrics']
        lines += ['', f'Bulk/default libmdbx recall p50 moved from {old["p50_ns"]["median"]/1000:.3f} to '
                   f'{new["p50_ns"]["median"]/1000:.3f} µs and p99 from {old["p99_ns"]["median"]/1000:.3f} to '
                   f'{new["p99_ns"]["median"]/1000:.3f} µs.']
    if exhausted:
        groups = ', '.join(f'{entry["scenario"]}/{entry["trial"]}' for entry in exhausted)
        lines += ['', f'Rounds that reached the {MAX_ATTEMPTS}-attempt limit: {groups}. '
                   'Groups with an exhausted round have per-trial receipts and no five-round summary.']
    if attempts:
        lines += ['', '| Scenario | Round | Attempt | GNU time major faults by command |', '|---|---|---:|---|']
        for entry in attempts:
            faults = ', '.join(f'{name} {n:,}' for name, n in entry['major_faults'].items())
            scenario, trial, attempt = (entry[key] for key in ('scenario', 'trial', 'attempt'))
            link = f'raw/measure-reads-quiet/discarded/{scenario}/{trial}-a{attempt}/gate.json'
            lines.append(f'| {scenario} | {trial} | [{attempt}]({link}) | {faults} |')
    return '\n'.join(lines) + '\n'


def verify_sources(meta, original=False, initial_driver=False):
    # The original campaign froze these files before the appended protocol
    # deviation and the optional memory sampler in the shared runner.
    historical = {'validation/libmdbx/PROTOCOL.md', 'tools/agentbench/readpath/rocksdb_bench.py'}
    for path, checksum in meta['sources'].items():
        if initial_driver and path == 'tools/agentbench/readpath/libmdbx_reads.py':
            old = subprocess.check_output(['git', 'show', f'a8854e54:{path}'], cwd=ROOT)
            assert hashlib.sha256(old).hexdigest() == checksum, path
        elif original and path in historical:
            old = subprocess.check_output(['git', 'show', f'460f9829:{path}'], cwd=ROOT)
            assert hashlib.sha256(old).hexdigest() == checksum, path
            if path.endswith('PROTOCOL.md'):
                assert (ROOT / path).read_bytes().startswith(old)
        else:
            assert digest(ROOT / path) == checksum, path
    binary = Path(meta['binary'])
    if binary.exists():
        assert digest(binary) == meta['binary_sha256']


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('raw', nargs='?', type=Path, default=VALIDATION / 'raw')
    parser.add_argument('--screen-only', action='store_true')
    args = parser.parse_args()
    raw = args.raw.resolve()
    out = raw.parent
    meta = load(raw / 'metadata.json')
    assert meta['candidates'] == CANDIDATES and meta['trials'] == TRIALS and meta['cpu'] == 10
    verify_sources(meta, original=True)
    budgets = set()
    fixtures = layouts(raw / 'prepare', budgets)
    assert len(fixtures) == 22
    screen, screening, screen_trials = audit_matrix(raw, 'screen', fixtures, budgets)
    assert len(screening) == 120 and sum(t['count'] for t in screen_trials) == 8400000
    selection = load(raw / 'selection.json')
    assert selection == selection_from_screen(raw)
    (out / 'SCREEN.md').write_text(details('screen', screen))
    if args.screen_only:
        print(f'Screen verified: {len(screening)} rotated commands, winners {selection["read"]} / {selection["write"]}')
        return
    read_phase = 'measure-reads-quiet'
    quiet = load(raw / read_phase / 'metadata.json')
    read_host = load(raw / read_phase / 'host.json')
    assert read_host['sibling'] == '10-11'
    assert set(read_host['coordinator_affinity']) <= set(range(12, 22))
    assert read_host['coordinator_nice'] >= 10
    assert quiet['max_attempts'] == MAX_ATTEMPTS and quiet['selection_sha256'] == digest(raw / 'selection.json')
    assert quiet['binary_sha256'] == meta['binary_sha256']
    continuation = raw / read_phase / 'continuation-metadata.json'
    verify_sources(quiet, initial_driver=continuation.exists())
    if continuation.exists():
        continued = load(continuation)
        assert continued == dict(quiet, sources=continued['sources'])
        verify_sources(continued)
    binary = quiet['binary']
    prep = raw / read_phase / 'prepare'
    rebuilt = layouts(prep, budgets, binary) if (prep / 'commands.jsonl').exists() else {}
    names = {Path(c['db']).name for _, _, commands in rounds(raw) for _, c in commands}
    assert rebuilt.keys() <= names
    for name, new in rebuilt.items():
        old = fixtures[name]
        assert new['disk']['files'] == old['disk']['files'] and new['layout'] == old['layout']
    accepted, discarded, attempts, exhausted = quiet_commands(raw)
    read_fixtures = fixtures | rebuilt
    reads, read_receipts, read_trials = audit_matrix(raw, read_phase, read_fixtures, budgets, accepted, binary)
    _, discarded_receipts, _ = audit_matrix(raw, read_phase, read_fixtures, budgets, discarded, binary, summarize=False)
    assert len(read_receipts) == 80 - 4 * len(exhausted) and len(discarded_receipts) == 4 * len(attempts)
    writes, write_receipts, write_trials = audit_matrix(raw, 'measure', fixtures, budgets)
    old_reads, old_receipts, _ = audit_matrix(raw, 'superseded-read-faults', fixtures, budgets)
    assert len(old_receipts) == 80 and len(write_receipts) == 40
    archived_faults = {}
    for label in LABELS:
        paths = list((out / 'superseded-read-faults').glob(f'read-*/r*-{label}.time.txt'))
        assert len(paths) == len(old_receipts) // 4
        faults = [major_faults(path) for path in paths]
        archived_faults[label] = dict(total=sum(faults), commands_with_faults=sum(n > 0 for n in faults),
                                     max_per_command=max(faults))
    measure = reads | writes
    primary = read_receipts + write_receipts
    primary_trials = read_trials + write_trials
    assert len(primary) == 120 - 4 * len(exhausted)
    assert sum(t['count'] for t in primary_trials) == 16200000 - 800000 * len(exhausted)
    assert len(budgets) == 1
    durable = durabilities(out / 'checks', selection)
    audit = dict(screen_commands=len(screening), primary_commands=len(primary),
                 screen_timed_calls=sum(t['count'] for t in screen_trials),
                 primary_timed_calls=sum(t['count'] for t in primary_trials),
                 prepared_fixtures=len(fixtures), rebuilt_read_fixtures=len(rebuilt), errors=0, affinity='10',
                 read_page_fault_gate=dict(accepted_rounds=len(read_receipts) // 4,
                                           discarded_rounds=len(attempts), discarded_commands=len(discarded_receipts),
                                           discarded_attempts=attempts, exhausted_rounds=exhausted,
                                           major_faults_in_accepted_commands=0,
                                           superseded_major_faults=archived_faults,
                                           cpu11_busy_percent=spread(map(sibling_busy, read_receipts)),
                                           load1=spread(r['load'][0] for r in read_receipts),
                                           mem_available_mib=spread(r[k]['mem_available_kib'] / 1024
                                                                   for r in read_receipts
                                                                   for k in ('memory_before', 'memory_after'))),
                 default_cache_bytes=next(iter(budgets)),
                 cpu11_busy_percent=spread(map(sibling_busy, primary)),
                 load1=spread(r['load'][0] for r in primary), durability=durable)
    dump(out / 'audit.json', audit)
    dump(out / 'fixture-layouts.json', fixtures)
    dump(out / 'summary.json', dict(screen=screen, measure=measure))
    with (out / 'TRIALS.csv').open('w') as file:
        writer = csv.DictWriter(file, fieldnames=list(primary_trials[0]), lineterminator='\n')
        writer.writeheader()
        writer.writerows(screen_trials + primary_trials)
    (out / 'MEASURE.md').write_text(details('measure', measure) + '\n' + gate_details(attempts, exhausted, len(read_receipts), old_reads, reads))
    (out / 'RESULTS.md').write_text(headline(measure))
    print(json.dumps(audit, indent=2))


if __name__ == '__main__':
    main()
