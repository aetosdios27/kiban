#!/usr/bin/env python3
"""Pinned PH2/PH3 cost comparison using the unchanged readpath workloads.

Build/freeze binaries separately. Run prepare, read, maintenance, summarize in
order; each command records its config, CPU affinity, binary hash and status.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import time
import tempfile

import run

ROOT = run.ROOT
OUT = ROOT / 'validation/pointindex-cost'
DATA = ROOT / 'target/pointindex-cost/data'
BIN = ROOT / 'target/pointindex-cost/bin'
run.CPU = 8


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cpu9():
    fields = next(line for line in Path('/proc/stat').read_text().splitlines() if line.startswith('cpu9 ')).split()[1:]
    ticks = [int(v) for v in fields[:8]]
    return dict(total=sum(ticks), idle=ticks[3]+ticks[4])


def measure(label, engine, layout, cache, folder, name, action='read'):
    run.BINARY = BIN / label
    config = run.config(engine, 1000000, layout, 512 if cache == 'default' else int(cache))
    config.update(db=str(DATA / label / f'{engine}-{layout}'), point_index=True, action=action)
    if cache == 'default':
        config['cache_mib'] = None
    if (folder / (name+'.jsonl')).exists():
        raise RuntimeError('refusing to overwrite '+name)
    started = cpu9()
    run.run(config, folder, name)
    ended = cpu9()
    with (folder / 'sibling.jsonl').open('a') as log:
        log.write(json.dumps(dict(name=name, start=started, end=ended,
            busy_fraction=1-(ended['idle']-started['idle'])/max(1,ended['total']-started['total'])))+'\n')


def space():
    rows = []
    for label in ['before', 'after']:
        for layout in ['bulk', 'normal']:
            sizes, points = [], []
            for path in (DATA / label / f'kiban-{layout}').glob('*.sst'):
                with path.open('rb') as stream:
                    stream.seek(-68, 2)
                    footer = stream.read()
                assert int.from_bytes(footer[-28:-24], 'little') == 4
                points.append(int.from_bytes(footer[8:16], 'little'))
                sizes.append(path.stat().st_size)
            assert sizes
            rows.append(dict(label=label, layout=layout, sst_bytes=sum(sizes),
                             point_bytes=sum(points), files=len(sizes), bytes_per_key=sum(points)/1000000))
    (OUT / 'space.json').write_text(json.dumps(rows, indent=2)+'\n')


def summarize():
    result = {'maintenance': {}, 'reads': []}
    for label in ['before', 'after']:
        text = (OUT / f'{label}-maintenance.log').read_text()
        records = [json.loads(m.group()) for m in re.finditer(r'\{"type":"maintenance"[^\n]+', text)]
        assert len(records) == 10
        assert all(r['errors'] == 0 and r['verified_records'] == 1000000 for r in records)
        result['maintenance'][label] = records
    for layout in ['bulk', 'normal']:
        for cache in ['512', '32', 'default']:
            samples = {}
            folder = OUT / 'reads' / f'{layout}-{cache}'
            for trial in range(1, 6):
                for label in ['before', 'after']:
                    for engine in ['kiban', 'lmdb']:
                        name = f'r{trial}-{label}-{engine}'
                        rows = [json.loads(line) for line in (folder / (name+'.jsonl')).read_text().splitlines()]
                        complete, = [r for r in rows if r['type'] == 'complete']
                        assert complete['errors'] == 0 and complete['verified_records'] == 1000000
                        assert 'Cpus_allowed_list:\t8\n' in complete['proc_status']
                        for row in rows:
                            if row['type'] == 'stage':
                                assert row['errors'] == 0 and row['latency']['count'] == 100000
                                samples[trial, label, engine, row['workload']] = row['latency']
            for workload in ['recall-by-id', 'recent-for-agent']:
                row = dict(layout=layout, cache=cache, workload=workload, raw={}, normalized={})
                for pct in ['p50_ns', 'p99_ns']:
                    for label in ['before', 'after']:
                        row['raw'][label+'-'+pct] = [samples[t,label,'kiban',workload][pct] for t in range(1,6)]
                        row['raw'][label+'-lmdb-'+pct] = [samples[t,label,'lmdb',workload][pct] for t in range(1,6)]
                    row['normalized'][pct] = [
                        (samples[t,'after','kiban',workload][pct]/samples[t,'after','lmdb',workload][pct]) /
                        (samples[t,'before','kiban',workload][pct]/samples[t,'before','lmdb',workload][pct])
                        for t in range(1,6)]
                result['reads'].append(row)
    (OUT / 'summary.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({r['layout']+'/'+r['cache']+'/'+r['workload']:
                     {p:statistics.median(v) for p,v in r['normalized'].items()} for r in result['reads']}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare-before', 'prepare-after', 'read', 'maintenance', 'profile', 'summarize'])
    parser.add_argument("--label", choices=["before", "after"], default="after")
    parser.add_argument("--perf", type=Path, help="perf executable for the BUILD-only profile")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if args.action.startswith('prepare-'):
        label = args.action.removeprefix('prepare-')
        for layout in ['bulk', 'normal']:
            for engine in ['kiban', 'lmdb']:
                measure(label, engine, layout, '512', OUT / 'prepare', f'{label}-{engine}-{layout}', 'prepare')
        if label == 'after':
            space()
    elif args.action == 'read':
        checks = json.loads((OUT/'checks/status.json').read_text())
        assert len(checks) == 11 and all(row['code'] == 0 for row in checks)
        sources = json.loads((OUT/'checks/source-hashes.json').read_text())
        assert all(digest(ROOT/name) == sha for name,sha in sources.items())
        hashes = {label:digest(BIN / label) for label in ['before', 'after']}
        (OUT / 'read-provenance.json').write_text(json.dumps(dict(binaries=hashes,engine_sources=sources,cpu=8,trials=5,started=time.time(),load=os.getloadavg()),indent=2)+'\n')
        for layout in ['bulk', 'normal']:
            for cache in ['512', '32', 'default']:
                for trial in range(1, 6):
                    for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
                        for engine in ['kiban', 'lmdb']:
                            name = f'r{trial}-{label}-{engine}'
                            print(layout, cache, name, flush=True)
                            measure(label, engine, layout, cache, OUT/'reads'/f'{layout}-{cache}', name)
    elif args.action == 'profile':
        assert args.perf and args.perf.is_file(), '--perf must name the perf executable'
        stem = args.label+'-build'
        binary = BIN/('before-profile-test' if args.label == 'before' else 'after-test')
        assert not (OUT/(stem+'-profile-maintenance.log')).exists(), 'refusing to overwrite profile'
        with tempfile.TemporaryDirectory(dir=ROOT/'target/pointindex-cost', prefix='perf-') as temp:
            ctl, ack = Path(temp)/'ctl', Path(temp)/'ack'
            os.mkfifo(ctl)
            os.mkfifo(ack)
            command = ['taskset','-c','8',str(args.perf),'record','-m','32','-e','cpu-clock:u','-F','499',
                       '--call-graph','dwarf,4096','--user-callchains','--control',f'fifo:{ctl},{ack}',
                       '-D','-1','-o',str(ROOT/'target/pointindex-cost'/(stem+'.perf.data')),'--',
                       str(binary),'point_index_maintenance_measurement','--ignored','--nocapture','--test-threads=1']
            env = {**os.environ,'KIBAN_POINT_BENCH_N':'1000000','KIBAN_POINT_BENCH_PROFILE':'1',
                   'READBENCH_PERF_CTL':str(ctl),'READBENCH_PERF_ACK':str(ack),
                   'TMPDIR':str(ROOT/'target/pointindex-cost/tmp')}
            with (OUT/(stem+'-profile-maintenance.log')).open('w') as log, (OUT/(stem+'-perf.log')).open('w') as err:
                result = subprocess.run(command,env=env,stdout=log,stderr=err)
            (OUT/(stem+'-profile-command.json')).write_text(json.dumps(dict(command=command,code=result.returncode,
                clock_ticks_per_second=os.sysconf('SC_CLK_TCK'),binary_sha256=digest(binary)),indent=2)+'\n')
            assert result.returncode == 0, result.returncode
    elif args.action == 'maintenance':
        assert not (OUT/'after-maintenance.log').exists(), 'refusing to overwrite maintenance trials'
        with (OUT/'after-maintenance.log').open('w') as log:
            subprocess.run(['/usr/bin/time','-v','-o',str(OUT/'after-maintenance-resources.log'),'taskset','-c','8',str(BIN/'after-test'), 'point_index_maintenance_measurement','--ignored','--nocapture','--test-threads=1'],
                           env={**os.environ,'KIBAN_POINT_BENCH_N':'1000000','TMPDIR':str(ROOT/'target/pointindex-cost/tmp')},
                           stdout=log,stderr=log,check=True)
    else:
        summarize()


if __name__ == '__main__':
    main()
