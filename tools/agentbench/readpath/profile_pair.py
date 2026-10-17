#!/usr/bin/env python3
"""Compare frozen full-read profiles, alternating builds and LMDB controls."""
import argparse
import hashlib
import json
from pathlib import Path
import profile as profiler
from run import config

parser = argparse.ArgumentParser()
parser.add_argument('--before', type=Path, required=True)
parser.add_argument('--after', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--layout', choices=['bulk', 'normal'], default='bulk')
parser.add_argument('--stage', type=int, choices=[1, 2], default=1)
parser.add_argument('--trials', type=int, default=3)
parser.add_argument('--stacks', action='store_true')
parser.add_argument('--text-stacks', action='store_true', help='Keep restored stacks and perf reports without requiring inferno')
parser.add_argument('--cache', type=int, choices=[32, 512], default=512)
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=True)
binaries = {'before': args.before.resolve(), 'after': args.after.resolve()}
(args.out / 'metadata.json').write_text(json.dumps(dict(
    binaries={label: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())
              for label, path in binaries.items()},
    layout=args.layout, stage=args.stage, cache=args.cache, trials=args.trials, affinity='3'), indent=2)+'\n')
for trial in range(args.trials):
    for label in ('before', 'after') if trial % 2 == 0 else ('after', 'before'):
        profiler.BINARY = binaries[label]
        for engine in ('kiban', 'lmdb'):
            c = config(engine, 1000000, args.layout, args.cache)
            c.update(action='profile', stage=args.stage, operations=1000000)
            title = f'r{trial+1}-{label}-{engine}'
            print(title, flush=True)
            profiler.collect(c, args.out, title)
            if (args.stacks or args.text_stacks) and trial == 0:
                profiler.collect(c, args.out, title+'-stacks', record=True, flamegraph=not args.text_stacks)
