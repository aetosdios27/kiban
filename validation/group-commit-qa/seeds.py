#!/usr/bin/env python3
"""Durable, sharded long crash gate. Run under nohup; retain per-shard exits."""
import argparse
import json
import hashlib
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument('--seeds', type=int, default=200000)
parser.add_argument('--shards', type=int, default=16)
parser.add_argument('--start', type=int, default=0)
parser.add_argument('--label', default='seeds')
args = parser.parse_args()
assert args.seeds >= args.shards > 0
GATE = Path.home() / '.local/share/kiban-gates/gc-qa-20261022'
GATE.mkdir(parents=True, exist_ok=True)
(GATE / 'tmp').mkdir(exist_ok=True)
env = dict(os.environ, TMPDIR=str(GATE / 'tmp'))
provenance = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in ('src/db.rs', 'src/group_commit.rs', 'src/group_commit_tests.rs', 'src/group_commit_qa_tests.rs')}
print(json.dumps(dict(source_sha256=provenance, base=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())), flush=True)
build_command = ['cargo', 'test', '--release', '--lib', '--no-run', '--message-format=json']
with (GATE / 'release-build.log').open('w') as log:
    build = subprocess.run(build_command, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=log, text=True)
    log.write(build.stdout)
assert build.returncode == 0, f'build exit {build.returncode}'
artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
executables = [a['executable'] for a in artifacts if a.get('reason') == 'compiler-artifact' and a.get('executable') and a['target']['name'] == 'kiban']
assert len(executables) == 1, executables
processes = []
start = args.start
began = time.monotonic()
for shard in range(args.shards):
    count = args.seeds // args.shards + (shard < args.seeds % args.shards)
    path = GATE / f'seeds-{start:06}-{count}.log'
    log = path.open('w')
    command = [executables[0], 'group_commit_crash_long', '--ignored', '--nocapture', '--test-threads=1']
    shard_env = dict(env, KIBAN_SIM_SEED_START=str(start), KIBAN_SIM_SEEDS=str(count))
    process = subprocess.Popen(command, cwd=ROOT, env=shard_env, stdout=log, stderr=subprocess.STDOUT)
    processes.append((process, log, path, start, count, command))
    print(json.dumps(dict(shard=shard, pid=process.pid, start=start, count=count, log=str(path))), flush=True)
    start += count
results = []
for process, log, path, start, count, command in processes:
    code = process.wait()
    log.close()
    tail = path.read_text()
    passed = code == 0 and f'checked {count} group-commit crash seeds from {start}' in tail and '1 passed; 0 failed' in tail
    result = dict(start=start, count=count, exit=code, passed=passed, log=str(path), command=command)
    results.append(result)
    print(json.dumps(result), flush=True)
summary = dict(source_sha256=provenance, seeds=sum(r['count'] for r in results if r['passed']), requested=args.seeds, passed=all(r['passed'] for r in results), seconds=round(time.monotonic()-began, 2), results=results)
(GATE / f'{args.label}-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary), flush=True)
raise SystemExit(0 if summary['passed'] else 1)
