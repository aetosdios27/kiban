#!/usr/bin/env python3
"""Run unchanged canonical full benchmarks serially on an explicit filesystem."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
p.add_argument('--tmpdir', type=Path, required=True)
a = p.parse_args()
if 'KIBAN_BENCH_QUICK' in os.environ:
    p.error('unset KIBAN_BENCH_QUICK for the release baseline')
a.output.mkdir(parents=True, exist_ok=False)
a.tmpdir.mkdir(parents=True, exist_ok=True)
repo = Path(__file__).resolve().parent.parent

def command(args):
    return subprocess.check_output(args, cwd=repo, text=True).strip()

metadata = {
    'commit': command(['git', 'rev-parse', 'HEAD']),
    'tracked_changes': command(['git', 'diff', '--stat', 'HEAD']),
    'rustc': command(['rustc', '-Vv']),
    'cargo': command(['cargo', '-V']),
    'profile': 'bench (release defaults)',
    'os': command(['uname', '-a']),
    'os_release': Path('/etc/os-release').read_text(),
    'cpu': Path('/proc/cpuinfo').read_text(),
    'memory': Path('/proc/meminfo').read_text(),
    'filesystem': command(['findmnt', '-J', '-T', str(a.tmpdir.resolve())]),
    'tmpdir': str(a.tmpdir.resolve()),
    'quick': False,
    'rustflags': os.environ.get('RUSTFLAGS', ''),
    'profile_overrides': {k: v for k, v in os.environ.items() if k.startswith('CARGO_PROFILE_')},
    'sources': {},
    'configuration': 'Exact workload counts, thread counts, values, and options are preserved in source snapshots and stdout. No workload flags overridden.',
    'runs': [],
}
source_dir = a.output / 'sources'
source_dir.mkdir()
for path in ['Cargo.toml', 'src/db.rs', 'benches/basic.rs', 'benches/steady_state.rs', 'benches/phase_changing.rs']:
    content = (repo / path).read_bytes()
    metadata['sources'][path] = hashlib.sha256(content).hexdigest()
    (source_dir / path.replace('/', '__')).write_bytes(content)
env = os.environ.copy()
env['TMPDIR'] = str(a.tmpdir.resolve())
for bench in ['basic', 'steady_state', 'phase_changing']:
    args = ['cargo', 'bench', '--bench', bench]
    print(' '.join(args), flush=True)
    start = time.monotonic()
    with (a.output / (bench + '.log')).open('x') as log:
        result = subprocess.run(args, cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
    metadata['runs'].append({'bench': bench, 'command': args, 'exit_code': result.returncode,
                             'elapsed_s': time.monotonic() - start, 'output': bench + '.log'})
    (a.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    if result.returncode:
        raise SystemExit(result.returncode)
