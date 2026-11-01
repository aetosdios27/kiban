#!/usr/bin/env python3
"""Bounded, serial, randomized-order comparisons; never delete data or clear global caches."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import signal
import subprocess
import time
import tomllib

REQUIRED = {'workload', 'operations', 'keys', 'working-set', 'value-size', 'threads', 'seed',
            'read-percent', 'preload-keys', 'sync-every', 'cache', 'profile', 'duration', 'sample-ms',
            'drain-seconds', 'verify', 'rocks-statistics'}
KNOBS = {'write-buffer', 'block-cache', 'target-file-size', 'base-level-bytes', 'level-multiplier',
         'l0-trigger', 'l0-stall', 'max-open-files'}

def validate(plan):
    if not isinstance(plan.get('repetitions'), int) or plan['repetitions'] < 1:
        raise ValueError('positive repetitions required')
    if not isinstance(plan.get('order_seed'), int):
        raise ValueError('explicit integer order_seed required')
    names = set()
    for case in plan['cases']:
        name, args = case['name'], case['args']
        if not name or not all(c.isalnum() or c in '-_' for c in name) or name in names:
            raise ValueError('case names must be unique and safe path components')
        names.add(name)
        if not REQUIRED <= args.keys():
            raise ValueError(f'{name}: missing fixed parameters: {REQUIRED - args.keys()}')
        if args['profile'] == 'matched' and not KNOBS <= args.keys():
            raise ValueError(f'{name}: all matched engine knobs must be explicit')
        if {'engine', 'db', 'output', 'reuse'} & args.keys():
            raise ValueError('engine/db/output/reuse are controlled by the campaign')
        if case.get('timeout_s', 0) <= 0:
            raise ValueError('each case requires a positive wall timeout')
        if case.get('engines') != ['kiban', 'rocksdb']:
            raise ValueError('paired kiban/rocksdb runs required')
    return plan

def jobs(plan):
    blocks = [(case, repeat) for case in plan['cases'] for repeat in range(plan['repetitions'])]
    rng = random.Random(plan['order_seed']); rng.shuffle(blocks)
    result = []
    for case, repeat in blocks:
        engines = case['engines'].copy(); rng.shuffle(engines)
        for engine in engines:
            result.append((case, repeat, engine))
    return result

def capture(args, repo):
    return subprocess.check_output(args, cwd=repo, text=True).strip()

def terminate(proc):
    os.killpg(proc.pid, signal.SIGTERM)
    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL); proc.wait()

def supervise(command, log, cwd, timeout):
    wall = time.time(); monotonic = time.monotonic()
    proc = subprocess.Popen(command, cwd=cwd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    reason = None
    while proc.poll() is None:
        time.sleep(0.2)
        real = time.time() - wall; active = time.monotonic() - monotonic
        if abs(real - active) > 2:
            reason = 'suspend-or-wall-clock-jump'; terminate(proc); break
        if real > timeout:
            reason = 'wall-timeout'; terminate(proc); break
    elapsed = time.time() - wall
    gap = elapsed - (time.monotonic() - monotonic)
    if abs(gap) > 2:
        reason = reason or 'suspend-or-wall-clock-jump'
    return {'exit_code': proc.wait(), 'wall_elapsed_s': elapsed, 'wall_minus_monotonic_s': gap, 'invalid_reason': reason}

def main():
    p = argparse.ArgumentParser()
    p.add_argument('--plan', type=Path, required=True)
    p.add_argument('--output-root', type=Path, required=True)
    p.add_argument('--data-root', type=Path, required=True)
    p.add_argument('--execute', action='store_true')
    p.add_argument('--prebuilt-from', type=Path, help='reuse a previously validated binary only after checking binary/source hashes')
    a = p.parse_args()
    repo = Path(__file__).resolve().parents[2]
    plan = validate(json.loads(a.plan.read_text()))
    binary = repo / 'tools/compare/target/release/kiban-compare'
    commands = []
    for case, repeat, engine in jobs(plan):
        label = f"{case['name']}-r{repeat}-{engine}"
        args = [str(binary), '--engine', engine, '--db', str(a.data_root.resolve() / label), '--output', str(a.output_root.resolve() / label)]
        for key, value in case['args'].items():
            args += ['--' + key, str(value).lower() if isinstance(value, bool) else str(value)]
        commands.append({'label': label, 'case': case['name'], 'repeat': repeat, 'engine': engine, 'timeout_s': case['timeout_s'], 'command': args})
    if not a.execute:
        print(json.dumps(commands, indent=2)); return
    if a.output_root.resolve().is_relative_to(a.data_root.resolve()) or a.data_root.resolve().is_relative_to(a.output_root.resolve()):
        p.error('output and database roots must be separate')
    a.output_root.mkdir(parents=True, exist_ok=False); a.data_root.mkdir(parents=True, exist_ok=True)
    metadata = {'commit': capture(['git', 'rev-parse', 'HEAD'], repo), 'dirty': capture(['git', 'status', '--porcelain'], repo),
        'kiban_commit': capture(['git', 'rev-parse', 'HEAD'], repo), 'plan': plan, 'commands': commands,
        'rustc': capture(['rustc', '-Vv'], repo), 'os': capture(['uname', '-a'], repo),
        'cpu': Path('/proc/cpuinfo').read_text(), 'initial_memory': Path('/proc/meminfo').read_text(),
        'filesystem': capture(['findmnt', '-J', '-T', str(a.data_root.resolve())], repo),
        'profile': 'release, debug=1, opt-level=3 defaults, no LTO',
        'build_env': {k: os.environ.get(k) for k in ['CARGO_BUILD_JOBS', 'BINDGEN_EXTRA_CLANG_ARGS', 'RUSTFLAGS', 'CXXFLAGS']}, 'source_hashes': {}}
    snapshots = a.output_root / 'sources'; snapshots.mkdir()
    sources = [*[p.relative_to(repo) for p in (repo / 'tools/compare/src').glob('*.rs')], Path('tools/compare/Cargo.toml'), Path('tools/compare/Cargo.lock'),
               Path('examples/scale/config.rs'), Path('examples/scale/workload.rs'), Path('examples/scale/metrics.rs'), Path('examples/scale/cache.rs'), Path('examples/support/random.rs'), Path('tools/compare/run.py')]
    for path in sources:
        content = (repo / path).read_bytes(); metadata['source_hashes'][str(path)] = hashlib.sha256(content).hexdigest()
        (snapshots / str(path).replace('/', '__')).write_bytes(content)
    metadata['resolved_packages'] = [(x['name'], x['version'], x.get('source')) for x in tomllib.loads((repo/'tools/compare/Cargo.lock').read_text())['package']]
    (a.output_root / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    if a.prebuilt_from:
        previous = json.loads((a.prebuilt_from / 'metadata.json').read_text())
        if previous.get('binary_sha256') != hashlib.sha256(binary.read_bytes()).hexdigest():
            raise SystemExit('prebuilt binary hash differs from validated campaign')
        inputs = [p for p in metadata['source_hashes'] if p.endswith(('.rs', 'Cargo.toml', 'Cargo.lock'))]
        if any(metadata['source_hashes'][p] != previous['source_hashes'].get(p) for p in inputs):
            raise SystemExit('compiled sources differ from validated campaign; rebuild required')
        metadata['build_reused_from'] = str(a.prebuilt_from.resolve())
        metadata['validated_build_env'] = previous['build_env']
    else:
        with (a.output_root / 'build.log').open('x') as log:
            result = subprocess.run(['cargo', 'build', '--release', '--locked', '--manifest-path', 'tools/compare/Cargo.toml'], cwd=repo, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            raise SystemExit('comparison build failed; inspect build.log')
    metadata['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
    (a.output_root / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    results = []
    for index, job in enumerate(commands):
        print(f"[{index+1}/{len(commands)}] {job['label']}", flush=True)
        with (a.output_root / (job['label'] + '.log')).open('x') as log:
            result = supervise(job['command'], log, repo, job['timeout_s'])
        status = a.output_root / job['label'] / 'status.json'
        try:
            complete = json.loads(status.read_text()).get('status') == 'complete'
        except (OSError, ValueError):
            complete = False
        result.update({'label': job['label'], 'case': job['case'], 'repeat': job['repeat'], 'engine': job['engine'], 'complete': complete})
        result['valid'] = result['exit_code'] == 0 and complete and result['invalid_reason'] is None
        results.append(result)
        (a.output_root / 'runs.json').write_text(json.dumps(results, indent=2) + '\n')
        if not result['valid']:
            raise SystemExit(f"STOP: invalid/failed run {job['label']}; inspect runs.json and logs; no retry or parameter change")
    print('All paired runs completed and verified.', flush=True)

if __name__ == '__main__':
    main()
