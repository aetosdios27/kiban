#!/usr/bin/env python3
"""Run the frozen CPU-6 campaign only after all correctness gates pass."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys

import smallcache as campaign

ROOT, OUT, BINS = campaign.ROOT, campaign.OUT, campaign.BINS


def main():
    checks = OUT/'ph3-checks'
    status = json.loads((checks/'status.json').read_text())
    assert len(status) == 11 and all(row['code'] == 0 for row in status)
    hashes = json.loads((checks/'source-hashes.json').read_text())
    assert all(hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest for name, digest in hashes.items())
    shutil.copy2(ROOT/'tools/agentbench/readpath/target/release/kiban-readbench', BINS/'ph3')
    (OUT/'ph3-provenance.json').write_text(json.dumps(dict(
        engine_source_hashes=hashes,
        engine_commit=subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip(),
        binaries={label:hashlib.sha256((BINS/label).read_bytes()).hexdigest() for label in ['before', 'ph3']},
        baseline='06a457dbfe24aabe1e33a3763e1ac0e1a8d73e05',
        timing_cpu=6, build_cpus='12-21', nice=10,
        toolchain=subprocess.check_output(['rustc', '--version'], text=True).strip(),
    ), indent=2)+'\n')
    inspect = OUT/'ph3-inspect'
    for layout in ['bulk', 'normal']:
        for indexed in [False, True]:
            for cache in [32, None, 512]:
                c = campaign.config('kiban', layout, indexed, cache)
                c['action'] = 'inspect'
                if indexed:
                    c['db'] = str(ROOT/'target/smallcache/ph3-data'/Path(c['db']).name)
                campaign.execute('ph3', c, inspect, f'{layout}-index{int(indexed)}-cache{cache}')
    stages = [
        ('mapped-read', 'read', ['--mmap-mode', 'all']),
        ('pread-read', 'read', []),
        ('mapped-cold', 'cold', ['--mmap-mode', 'all']),
        ('pread-cold', 'cold', []),
        ('write', 'write', ['--caches', '32']),
        ('pread-profile', 'profile', ['--indexed']),
        ('mapped-profile', 'profile', ['--indexed', '--mmap-mode', 'all']),
        ('pread-syscalls', 'profile', ['--indexed', '--syscalls']),
        ('mapped-syscalls', 'profile', ['--indexed', '--syscalls', '--mmap-mode', 'all']),
    ]
    env = {**os.environ, 'READBENCH_PERF_PAGES':'8'}
    for title, action, extra in stages:
        folder = OUT/('ph3-'+title)
        print('start '+title, flush=True)
        subprocess.run([sys.executable, str(ROOT/'tools/agentbench/readpath/smallcache.py'), action,
                        '--out', str(folder), '--after', 'ph3', '--candidate-data', str(ROOT/'target/smallcache/ph3-data'), '--caches', '32', 'default', '512', *extra], cwd=ROOT, env=env, check=True)
        if action != 'profile':
            subprocess.run([sys.executable, str(ROOT/'tools/agentbench/readpath/smallcache_report.py'), str(folder), '--after', 'ph3'], cwd=ROOT, check=True)
        if title == 'mapped-read':
            target = []
            for row in json.loads((folder/'summary.json').read_text()):
                if row['scenario'][1:3] == ['1', '32'] and row['scenario'][-1] == 'recall-by-id':
                    d = row['distributions']
                    p50 = statistics.median(row['ratios']['ph3'])
                    p99 = statistics.median(a/b for a,b in zip(d['ph3-kiban']['p99_ns'], d['ph3-lmdb']['p99_ns']))
                    target.append(dict(layout=row['scenario'][0], p50_ratio=p50, p99_ratio=p99,
                                       accepted=p50 <= 0.9 and p99 <= 1))
            (OUT/'ph3-target.json').write_text(json.dumps(target, indent=2)+'\n')
            assert len(target) == 2 and all(row['accepted'] for row in target), target
        print('end '+title, flush=True)
    subprocess.run([sys.executable, str(ROOT/'tools/agentbench/readpath/smallcache_components.py')], cwd=ROOT, env=env, check=True)
    assert all(hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest for name, digest in hashes.items())


if __name__ == '__main__':
    main()
