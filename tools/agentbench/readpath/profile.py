#!/usr/bin/env python3
"""Collect counters and user call stacks only inside the isolated work loop."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import hashlib
import gzip
from fold import restore_ips

from run import ROOT, BINARY, config
import json

# Offline stack decoding and SVG generation also stay off the E-core sweep.
os.sched_setaffinity(0, {2, 3, 4, 5})

PERF = ROOT / 'target/read-path-tools/root/usr/bin/perf'
INFERNO = ROOT / 'target/read-path-tools/inferno/bin'
EVENTS = ','.join('cpu_core/' + name + '/u' for name in (
    'cycles', 'instructions', 'L1-dcache-load-misses', 'LLC-load-misses',
    'branch-misses', 'dTLB-load-misses'))


def collect(c, folder, name, record=False):
    path = (folder / (name + '.config.json')).resolve()
    path.write_text(json.dumps(c, indent=2)+'\n')
    with tempfile.TemporaryDirectory(prefix='kiban-perf-') as tmp:
        ctl, ack = Path(tmp)/'ctl', Path(tmp)/'ack'
        os.mkfifo(ctl)
        os.mkfifo(ack)
        env = {**os.environ, 'READBENCH_PERF_CTL':str(ctl), 'READBENCH_PERF_ACK':str(ack)}
        if record:
            command = ['record', '-e', 'cpu_core/cycles/u', '--no-inherit', '--user-callchains',
                       '-m', '128', '-c', '1000000', '-g', '--call-graph', 'dwarf,8192',
                       '-o', str(folder/(name+'.data'))]
        else:
            command = ['stat', '-x', ';', '-e', EVENTS, '-o', str(folder/(name+'.stat'))]
        with (folder/(name+'.log')).open('w') as log:
            subprocess.run(['taskset','-c','3', str(PERF), *command,
                            '--control', f'fifo:{ctl},{ack}', '-D', '-1', '--', str(BINARY), str(path)],
                           check=True, env=env, stdout=log, stderr=log)
    if record:
        with (folder/(name+'.perf')).open('w') as out:
            subprocess.run([str(PERF), 'script', '-i', str(folder/(name+'.data'))], check=True, stdout=out)
        with (folder/(name+'.ips')).open('w') as out:
            subprocess.run([str(PERF), 'script', '-G', '-i', str(folder/(name+'.data'))], check=True, stdout=out)
        periods=restore_ips(folder/(name+'.perf'),folder/(name+'.ips'),folder/(name+'.merged'),folder/(name+'.coverage.json'))
        with (folder/(name+'.merged')).open() as src, (folder/(name+'.folded')).open('w') as out:
            subprocess.run([str(INFERNO/'inferno-collapse-perf')], stdin=src, stdout=out, check=True)
        assert sum(int(line.rsplit(' ',1)[1]) for line in (folder/(name+'.folded')).read_text().splitlines())==periods
        with (folder/(name+'.folded')).open() as src, (folder/(name+'.svg')).open('w') as out:
            subprocess.run([str(INFERNO/'inferno-flamegraph'), '--title', name], stdin=src, stdout=out, check=True)
        with (folder/(name+'.report')).open('w') as out:
            subprocess.run([str(PERF), 'report', '--stdio', '--no-children', '-i', str(folder/(name+'.data'))], check=True, stdout=out)
        (folder/(name+'.data')).unlink()
        (folder/(name+'.perf')).unlink()
        (folder/(name+'.ips')).unlink()
        with gzip.open(folder/(name+'.perf.gz'),'wb') as out:
            out.write((folder/(name+'.merged')).read_bytes())
        (folder/(name+'.merged')).unlink()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--micro-only', action='store_true')
    parser.add_argument('--select', help='Only names containing this text; useful for resuming an interrupted profile')
    parser.add_argument('--stacks-only', action='store_true')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out/'metadata.json').write_text(json.dumps(dict(binary=str(BINARY),sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),affinity='3',events=EVENTS),indent=2)+'\n')
    if not args.micro_only:
        for layout in ('bulk', 'normal'):
            for cache in (32,512):
                if args.stacks_only and cache!=512:
                    continue
                for stage in (1,2):
                    for engine in ('kiban','lmdb'):
                        c = config(engine,1000000,layout,cache)
                        c.update(action='profile',stage=stage,operations=1000000)
                        name=f'{layout}-{cache}-w{stage+1}-{engine}'
                        if args.select and args.select not in name:
                            continue
                        print(name, flush=True)
                        if not args.stacks_only:
                            collect(c,args.out,name)
                        if cache == 512:
                            collect(c,args.out,name+'-stacks',record=True)
    if args.stacks_only:
        return
    for micro in ('loop','memtable-miss','memtable-hit','bloom','index','hash','lock','arc','cache',
                  'block-seek','block-linear','verify-block','copy','decode'):
        c=config('kiban',1000000,'bulk',512)
        if args.select and args.select not in 'micro-'+micro:
            continue
        c.update(action='micro',micro=micro,operations=1000000)
        print(micro,flush=True)
        collect(c,args.out,'micro-'+micro)
    for layout in ('bulk','normal'):
        for micro in ('iter-construct','iter-first','iter-ten'):
            name='micro-'+layout+'-'+micro
            if args.select and args.select not in name:
                continue
            c=config('kiban',1000000,layout,512)
            c.update(action='micro',micro=micro,operations=1000000)
            print(name,flush=True)
            collect(c,args.out,name)


if __name__ == '__main__':
    main()
