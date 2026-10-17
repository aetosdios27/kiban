#!/usr/bin/env python3
"""Run the frozen CPU-6 campaign only after all correctness gates pass."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import smallcache as campaign

ROOT, OUT, BINS = campaign.ROOT, campaign.OUT, campaign.BINS


def main():
    checks = OUT/'final-checks'
    status = json.loads((checks/'status.json').read_text())
    assert len(status) == 11 and all(row['code'] == 0 for row in status)
    hashes = json.loads((checks/'source-hashes.json').read_text())
    assert all(hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest for name, digest in hashes.items())
    shutil.copy2(ROOT/'tools/agentbench/readpath/target/release/kiban-readbench', BINS/'after')
    (OUT/'final-provenance.json').write_text(json.dumps(dict(
        engine_source_hashes=hashes,
        binaries={label:hashlib.sha256((BINS/label).read_bytes()).hexdigest() for label in ['before', 'after']},
        baseline='06a457dbfe24aabe1e33a3763e1ac0e1a8d73e05',
        timing_cpu=6, build_cpus='12-21', nice=10,
        toolchain=subprocess.check_output(['rustc', '--version'], text=True).strip(),
    ), indent=2)+'\n')
    inspect = OUT/'inspect'
    for layout in ['bulk', 'normal']:
        for indexed in [False, True]:
            for cache in [32, None]:
                c = campaign.config('kiban', layout, indexed, cache)
                c['action'] = 'inspect'
                campaign.execute('after', c, inspect, f'{layout}-index{int(indexed)}-cache{cache}')
    stages = [
        ('read', []),
        ('cold', []),
        ('write', []),
        ('mapped-read', ['--mapped', '--layouts', 'bulk', 'normal', '--caches', '32', '--indexes', '1']),
        ('mapped-cold', ['--mapped', '--layouts', 'bulk', 'normal', '--caches', '32', '--indexes', '1']),
        ('profile', ['--indexed']),
        ('syscalls', ['--indexed', '--syscalls']),
    ]
    env = {**os.environ, 'READBENCH_PERF_PAGES':'8'}
    for title, extra in stages:
        action = {'mapped-read':'read', 'mapped-cold':'cold', 'syscalls':'profile'}.get(title, title)
        folder = OUT/('final-'+title)
        print('start '+title, flush=True)
        subprocess.run([sys.executable, str(ROOT/'tools/agentbench/readpath/smallcache.py'), action,
                        '--out', str(folder), *extra], cwd=ROOT, env=env, check=True)
        if action != 'profile':
            subprocess.run([sys.executable, str(ROOT/'tools/agentbench/readpath/smallcache_report.py'), str(folder)], cwd=ROOT, check=True)
        print('end '+title, flush=True)
    assert all(hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest for name, digest in hashes.items())


if __name__ == '__main__':
    main()
