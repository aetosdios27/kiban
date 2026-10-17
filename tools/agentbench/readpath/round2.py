#!/usr/bin/env python3
"""Run the retained-fix paired matrix without overlapping local builds/tests."""
import argparse
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument('--before', type=Path, required=True)
parser.add_argument('--after', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--defaults', action='store_true')
parser.add_argument('--resume', action='store_true')
args = parser.parse_args()
for layout in ('bulk', 'normal'):
    for cache in ('32', '512', *(['default'] if args.defaults else [])):
        folder = args.out / f'{layout}-{cache}'
        command = [sys.executable, str(HERE / 'paired.py'), '--before', str(args.before), '--after', str(args.after), '--out', str(folder), '--layout', layout, '--cache', cache]
        if args.resume:
            command.append('--resume')
        subprocess.run(command, check=True)
        with (folder / 'report.md').open('w') as output:
            subprocess.run([sys.executable, str(HERE / 'paired_report.py'), str(folder)], stdout=output, check=True)
for durable in (False, True):
    folder = args.out / ('writes-durable' if durable else 'writes-fast')
    command = [sys.executable, str(HERE / 'paired.py'), '--before', str(args.before), '--after', str(args.after), '--out', str(folder), '--n', '10000', '--write']
    if args.resume:
        command.append('--resume')
    if durable:
        command.append('--durable')
    subprocess.run(command, check=True)
    with (folder / 'report.md').open('w') as output:
        subprocess.run([sys.executable, str(HERE / 'paired_report.py'), str(folder)], stdout=output, check=True)
