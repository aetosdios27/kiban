#!/usr/bin/env python3
"""Alternate isolated components or frozen builds on one P-core."""
import argparse
import hashlib
import json
from pathlib import Path
import profile as profiler
from run import BINARY, config

parser = argparse.ArgumentParser()
parser.add_argument('--before', required=True)
parser.add_argument('--after', required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--binary', type=Path, default=BINARY)
parser.add_argument('--before-binary', type=Path)
parser.add_argument('--after-binary', type=Path)
parser.add_argument('--layout', choices=['bulk', 'normal'], default='bulk')
parser.add_argument('--trials', type=int, default=5)
args = parser.parse_args()
cases = {'before': (args.before, args.before_binary or args.binary),
         'after': (args.after, args.after_binary or args.binary)}
args.out.mkdir(parents=True, exist_ok=True)
(args.out / 'metadata.json').write_text(json.dumps(dict(
    cases={label: dict(component=name, binary=str(binary.resolve()),
                       sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
           for label, (name, binary) in cases.items()},
    layout=args.layout, trials=args.trials, affinity='3'), indent=2)+'\n')
for trial in range(args.trials):
    labels = ('before', 'after') if trial % 2 == 0 else ('after', 'before')
    for label in labels:
        name, binary = cases[label]
        profiler.BINARY = binary.resolve()
        c = config('kiban', 1000000, args.layout, 512)
        c.update(action='micro', micro=name, operations=1000000)
        title = f'r{trial+1}-{label}-{name}'
        print(title, flush=True)
        profiler.collect(c, args.out, title)
