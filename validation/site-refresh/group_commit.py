#!/usr/bin/env python3
"""validation/group-commit/run_matrix.py at this checkout, with local paths."""
import json, os, shutil, subprocess
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
B = ROOT / 'tools/agentbench/readpath/target/release/group-commit-bench'
P = ROOT / 'target/gc-probe.so'
D = ROOT / 'target/gc-data'
OUT = ROOT / 'validation/site-refresh/group-commit'
OUT.mkdir(parents=True, exist_ok=True)
D.mkdir(parents=True, exist_ok=True)
CPUS = '0,1,2,5'
CASES = [('kiban', 'sync'), ('kiban', 'group'), ('sqlite', 'x'), ('rocksdb', 'x')]
THREADS = [1, 2, 4, 8, 16]
TRIALS, TOTAL = 5, 4000
env = {**os.environ, 'LD_PRELOAD': str(P)}
for eng in {c[0] for c in CASES}:
    d = D / f'base-{eng}'
    if not d.exists():
        subprocess.run(['taskset', '-c', CPUS, str(B), 'prepare', eng, str(d), '10000'], check=True, stdout=subprocess.DEVNULL)
for t in range(1, TRIALS + 1):
    for th in THREADS:
        for eng, mode in (CASES if (t + th) % 2 else CASES[::-1]):
            name = f'{eng}-{mode}-t{th}-r{t}'
            out = OUT / (name + '.json')
            if out.exists():
                continue
            run = D / 'run'
            shutil.rmtree(run, ignore_errors=True)
            shutil.copytree(D / f'base-{eng}', run)
            subprocess.run(['sync'])
            subprocess.run(['taskset', '-c', CPUS, str(B), 'run', eng, mode, str(th), str(max(250, TOTAL // th)), str(run), str(out), '10000'], env=env, check=True, stdout=subprocess.DEVNULL)
            assert json.loads(out.read_text())['verified'], name
            print(name, flush=True)
print('ALLDONE', flush=True)
