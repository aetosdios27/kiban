#!/usr/bin/env python3
"""Freeze/replay PH2 and PH3 fixtures with CPU-8 timings and CPU-9 receipts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

import run

ROOT = run.ROOT
run.CPU = 8


def sibling():
    ticks = [int(v) for v in next(line for line in Path('/proc/stat').read_text().splitlines()
                                 if line.startswith('cpu9 ')).split()[1:9]]
    return dict(total=sum(ticks), idle=ticks[3] + ticks[4])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--data', type=Path, default=ROOT/'target/pointindex-cost/data')
    parser.add_argument('--layouts', nargs='+', default=['bulk', 'normal'])
    parser.add_argument('--caches', nargs='+', default=['512', 'default'])
    parser.add_argument('--trials', type=int, default=5)
    parser.add_argument('--operations', type=int, default=100000)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    binaries = dict(before=args.before.resolve(), after=args.after.resolve())
    metadata = dict(binaries={k: dict(path=str(p), sha256=hashlib.sha256(p.read_bytes()).hexdigest())
                              for k, p in binaries.items()}, cpu=8, sibling=9,
                    layouts=args.layouts, caches=args.caches, trials=args.trials,
                    operations=args.operations, started=time.time(), load=os.getloadavg())
    assert not (args.out/'metadata.json').exists(), 'use a fresh output directory'
    (args.out/'metadata.json').write_text(json.dumps(metadata, indent=2)+'\n')
    for layout in args.layouts:
        for cache in args.caches:
            folder = args.out/f'{layout}-{cache}'
            for trial in range(1, args.trials+1):
                for label in (['before', 'after'] if trial % 2 else ['after', 'before']):
                    run.BINARY = binaries[label]
                    for engine in ['kiban', 'lmdb']:
                        name = f'1000000-{layout}-{cache}-r{trial}-{label}-{engine}'
                        c = run.config(engine, 1000000, layout,
                                       None if cache == 'default' else int(cache))
                        c.update(db=str(args.data/label/f'{engine}-{layout}'),
                                 point_index=True, operations=args.operations)
                        start = sibling()
                        run.run(c, folder, name)
                        end = sibling()
                        rows = [json.loads(line) for line in (folder/(name+'.jsonl')).read_text().splitlines()]
                        complete, = [r for r in rows if r['type'] == 'complete']
                        assert complete['verified_records'] == 1000000 and complete['errors'] == 0
                        assert 'Cpus_allowed_list:\t8\n' in complete['proc_status']
                        assert all(r['errors'] == 0 and r['latency']['count'] == args.operations
                                   for r in rows if r['type'] == 'stage')
                        with (folder/'sibling.jsonl').open('a') as log:
                            log.write(json.dumps(dict(name=name, start=start, end=end,
                                busy_fraction=1-(end['idle']-start['idle'])/max(1,end['total']-start['total'])))+'\n')
                        print(name, flush=True)
            subprocess.run(['python3', str(ROOT/'tools/agentbench/readpath/paired_report.py'), str(folder)], check=True)


if __name__ == '__main__':
    main()
