#!/usr/bin/env python3
"""Alternate the existing read workload on one P-core; retain every trial."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT / 'tools/agentbench/readpath/target/release/kiban-readbench'
DATA = ROOT / 'target/read-path-data'


def config(engine, n, layout, cache=32):
    return dict(backend=engine, db=str(DATA / f'{engine}-{n}-{layout}'),
                n=n, operations=100000, k=10, seed=20261016,
                output='', action='read', layout=layout, cache_mib=cache)


def run(c, folder, name):
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / (name + '.config.json')
    c['output'] = str((folder / (name + '.jsonl')).resolve())
    path.write_text(json.dumps(c, indent=2) + '\n')
    started = time.time()
    with (folder / (name + '.log')).open('w') as log:
        subprocess.run(['taskset', '-c', '3', str(BINARY), str(path)],
                       check=True, stdout=log, stderr=log)
    with (folder / 'commands.jsonl').open('a') as log:
        log.write(json.dumps(dict(config=path.name, started=started, ended=time.time(),
                                  load=os.getloadavg(), affinity='3')) + '\n')


def main():
    global BINARY
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['prepare', 'measure'])
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--trials', type=int, default=5)
    parser.add_argument('--binary', type=Path, default=BINARY)
    args = parser.parse_args()
    BINARY = args.binary.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / 'metadata.json').write_text(json.dumps(dict(
        binary=str(BINARY), sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        git_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        affinity='3', load=os.getloadavg(), trials=args.trials), indent=2) + '\n')
    for n in (10000, 1000000):
        for layout in ('bulk', 'normal'):
            if args.action == 'prepare':
                for engine in ('kiban', 'lmdb'):
                    c = config(engine, n, layout)
                    if Path(c['db']).exists():
                        raise RuntimeError(f"existing database: {c['db']}")
                    c['action'] = 'prepare'
                    run(c, args.out, f'{n}-{layout}-{engine}-prepare')
            else:
                for cache in (32, 512):
                    for trial in range(args.trials):
                        for engine in ('kiban', 'lmdb') if trial % 2 == 0 else ('lmdb', 'kiban'):
                            c = config(engine, n, layout, cache)
                            run(c, args.out, f'{n}-{layout}-{cache}-r{trial + 1}-{engine}')


if __name__ == '__main__':
    main()
