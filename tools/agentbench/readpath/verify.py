#!/usr/bin/env python3
"""Run all commit gates on P-cores and preserve the exact exit statuses."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser()
parser.add_argument('folder', type=Path)
parser.add_argument('--long', action='store_true')
args = parser.parse_args()
folder = args.folder.resolve()
folder.mkdir(parents=True, exist_ok=True)
checks = [
    ('fmt', ['cargo', 'fmt', '--all', '--check']),
    ('clippy', ['cargo', 'clippy', '--all-targets', '--workspace', '--', '-D', 'warnings']),
    ('readbench-fmt', ['cargo', 'fmt', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml', '--check']),
    ('readbench-clippy', ['cargo', 'clippy', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml', '--all-targets', '--', '-D', 'warnings']),
    ('test', ['cargo', 'test', '--workspace', '--', '--test-threads=4']),
    ('release', ['cargo', 'test', '--release', '--workspace', '--', '--test-threads=4']),
    ('readbench-test', ['cargo', 'test', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml']),
    ('fixed', ['cargo', 'test', '--release', 'seeded_crash_fixed_corpus', '--', '--test-threads=1']),
    ('loom', ['cargo', 'test', '--features', 'loom', '--', '--test-threads=1', 'loom_']),
    ('site-facts', ['python3', 'tools/site_facts.py', 'validate']),
    ('long-20000-each', ['cargo', 'test', '--release', '--lib', 'seeded_crash_long', '--', '--ignored', '--test-threads=1', '--nocapture']),
]

env = {**os.environ, 'KIBAN_SIM_SEED_START': '0', 'KIBAN_SIM_SEEDS': '20000', 'CARGO_BUILD_JOBS': '4'}
rows = []
for name, command in checks:
    if name == 'long-20000-each' and not args.long:
        continue
    started = time.time()
    print(f'start {name}', flush=True)
    with (folder / (name + '.log')).open('w') as log:
        result = subprocess.run(['taskset', '-c', '2-5', *command], cwd=ROOT, env=env, stdout=log, stderr=log)
    rows.append(dict(name=name, command=command, code=result.returncode, seconds=time.time()-started))
    (folder / 'status.json').write_text(json.dumps(rows, indent=2)+'\n')
    print(f'end {name}: {result.returncode} ({rows[-1]["seconds"]:.1f}s)', flush=True)
    if result.returncode:
        print((folder / (name + '.log')).read_text()[-5000:], flush=True)
        sys.exit(result.returncode)
sources = sorted(set(subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', 'src', 'Cargo.toml', 'Cargo.lock'], cwd=ROOT, text=True).splitlines()))
(folder / 'source-hashes.json').write_text(json.dumps({p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in sources}, indent=2)+'\n')
