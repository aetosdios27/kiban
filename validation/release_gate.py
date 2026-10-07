#!/usr/bin/env python3
"""Run the release gate serially, retain exact-commit evidence, stop on failure."""
import argparse
import json
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
p.add_argument('--data', type=Path, required=True)
a = p.parse_args()
repo = Path(__file__).resolve().parent.parent
subprocess.run(['git', 'diff', '--exit-code', 'HEAD'], cwd=repo, check=True)
a.output.mkdir(parents=True, exist_ok=False)
a.data.mkdir(parents=True, exist_ok=True)
commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
commands = [
    ('fmt', ['cargo', 'fmt', '--check']),
    ('clippy', ['cargo', 'clippy', '--all-targets', '--', '-D', 'warnings']),
    ('clippy-loom', ['cargo', 'clippy', '--all-targets', '--features', 'loom', '--', '-D', 'warnings']),
    ('tests', ['cargo', 'test']),
    ('examples', ['cargo', 'test', '--examples']),
    ('loom', ['cargo', 'test', '--features', 'loom', '--lib', '--', '--test-threads=1', 'loom_']),
    ('ignored-stress', ['cargo', 'test', '--release', '--lib', '--', '--ignored', '--nocapture']),
    ('basic', ['cargo', 'run', '--example', 'basic', '--', str(a.data / 'basic')]),
    ('package', ['cargo', 'package']),
    ('package-list', ['cargo', 'package', '--list']),
    ('publish-dry-run', ['cargo', 'publish', '--dry-run']),
    # Last: never replace a running torture executable while children spawn.
    ('recovery', ['python3', 'validation/run.py', 'torture', '--execute', '--seed', '20261006',
                  '--threads', '4', '--data-root', str(a.data / 'torture'),
                  '--output-root', str(a.output / 'torture')]),
]
results = {'commit': commit, 'checks': []}
for label, args in commands:
    print(label + ': ' + ' '.join(args), flush=True)
    start = time.monotonic()
    with (a.output / (label + '.log')).open('x') as log:
        result = subprocess.run(args, cwd=repo, stdout=log, stderr=subprocess.STDOUT)
    results['checks'].append({'check': label, 'command': args, 'exit_code': result.returncode,
                              'elapsed_s': time.monotonic() - start})
    (a.output / 'gate.json').write_text(json.dumps(results, indent=2) + '\n')
    if result.returncode:
        raise SystemExit(result.returncode)
