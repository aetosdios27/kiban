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
parser.add_argument('--cpus', default=os.environ.get('READBENCH_BUILD_CPUS', '12-21'), help='build/test affinity')
parser.add_argument('--resume', action='store_true', help='reuse successful engine gates only if engine hashes and commands are identical; rerun benchmark gates')
parser.add_argument('--quick', action='store_true', help='omit long seeded suites during iteration')
args = parser.parse_args()
folder = args.folder.resolve()
folder.mkdir(parents=True, exist_ok=True)
nice_increment = max(0, 10 - os.getpriority(os.PRIO_PROCESS, 0))
checks = [
    ('test', ['cargo', 'test', '--workspace', '--', '--test-threads=4']),
    ('release', ['cargo', 'test', '--release', '--workspace', '--', '--test-threads=4']),
    ('clippy', ['cargo', 'clippy', '--all-targets', '--workspace', '--', '-D', 'warnings']),
    ('fmt', ['cargo', 'fmt', '--all', '--check']),
    ('fixed', ['cargo', 'test', '--release', 'seeded_crash_fixed_corpus', '--', '--test-threads=1']),
    ('long-20000-each', ['cargo', 'test', '--release', '--lib', 'seeded_crash_long', '--', '--ignored', '--test-threads=1', '--nocapture']),
    ('loom', ['cargo', 'test', '--features', 'loom', '--', '--test-threads=1', 'loom_']),
    ('site-facts', ['python3', 'tools/site_facts.py', 'validate']),
    ('readbench-clippy', ['cargo', 'clippy', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml', '--all-targets', '--', '-D', 'warnings']),
    ('readbench-remix-clippy', ['cargo', 'clippy', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml', '--features', 'remix', '--all-targets', '--', '-D', 'warnings']),
    ('readbench-remix-test', ['cargo', 'test', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml', '--features', 'remix']),
    ('readbench-test', ['cargo', 'test', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml']),
    ('readbench-fmt', ['cargo', 'fmt', '--manifest-path', 'tools/agentbench/readpath/Cargo.toml', '--check']),
]
def source_hashes():
    sources = sorted(set(subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', 'src', 'Cargo.toml', 'Cargo.lock'], cwd=ROOT, text=True).splitlines()))
    return {p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in sources}

initial_hashes = source_hashes()
rows = []
if args.resume:
    assert json.loads((folder/'initial-source-hashes.json').read_text()) == initial_hashes, 'engine changed; use a fresh full gate folder'
    metadata = json.loads((folder/'metadata.json').read_text())
    assert metadata['cpu'] == args.cpus and metadata['quick'] == args.quick
    previous = json.loads((folder/'status.json').read_text())
    with (folder/'attempts.jsonl').open('a') as log:
        log.write(json.dumps(dict(resumed=time.time(), previous=previous))+'\n')
    for row in previous:
        if row['code'] == 0 and not row['name'].startswith('readbench'):
            assert row['command'] == dict(checks)[row['name']]
            rows.append(row)
        elif (folder/(row['name']+'.log')).exists():
            (folder/(row['name']+'.previous.log')).write_bytes((folder/(row['name']+'.log')).read_bytes())
else:
    (folder / 'metadata.json').write_text(json.dumps(dict(
    head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    cpu=args.cpus, nice=10, coordinator_nice=os.getpriority(os.PRIO_PROCESS, 0),
    nice_increment=nice_increment, quick=args.quick,
    seeds_per_suite=20000, seed_start=0, started=time.time()), indent=2)+'\n')
    (folder / 'initial-source-hashes.json').write_text(json.dumps(initial_hashes, indent=2)+'\n')
env = {**os.environ, 'KIBAN_SIM_SEED_START': '0', 'KIBAN_SIM_SEEDS': '20000', 'CARGO_BUILD_JOBS': '8'}
completed = {r['name'] for r in rows}
for name, command in checks:
    if name in completed:
        print(f'reuse {name}: identical engine hashes and command', flush=True)
        continue
    if args.quick and name == 'long-20000-each':
        continue
    started = time.time()
    print(f'start {name}', flush=True)
    with (folder / (name + '.log')).open('w') as log:
        result = subprocess.run(['taskset', '-c', args.cpus, 'nice', '-n', str(nice_increment), *command], cwd=ROOT, env=env, stdout=log, stderr=log)
    rows.append(dict(name=name, command=command, code=result.returncode, seconds=time.time()-started))
    (folder / 'status.json').write_text(json.dumps(rows, indent=2)+'\n')
    print(f'end {name}: {result.returncode} ({rows[-1]["seconds"]:.1f}s)', flush=True)
    if result.returncode:
        print((folder / (name + '.log')).read_text()[-5000:], flush=True)
        sys.exit(result.returncode)
final_hashes = source_hashes()
assert initial_hashes == final_hashes, 'engine sources changed during verification; rerun gates'
(folder / 'source-hashes.json').write_text(json.dumps(final_hashes, indent=2)+'\n')
