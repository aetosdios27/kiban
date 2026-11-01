#!/usr/bin/env python3
"""Quick rebased confirmation: main acfec36e vs main plus the fuse filter.

Bulk layout, one million records, point_index on, default cache, `read`
(recall-by-id, recent-for-agent) and `miss` (absent-uniform, absent-agent-prefix)
suites. Three alternating pairs; every Kiban process is followed by an LMDB
control. Timed children run on CPU 8 only (fuse_engine.execute asserts it).
`main` writes the main format (point index, no fuse); `fuse` writes fuse + point.

    prepare  (taskset -c 12-21 nice -n 10, no timed run active)
    measure  (taskset -c 8, no builds or tests running)
    summary
"""
import argparse
import json
from pathlib import Path
import shutil
import statistics
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import fuse_engine  # noqa: E402

ROOT = HERE.parents[2]
N = 1_000_000


def fx(data, label, engine):
    return fuse_engine.fixture(data, label, engine, 'bulk', True, N)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('phase', choices=['prepare', 'measure', 'summary'])
    p.add_argument('--main', type=Path, required=True)
    p.add_argument('--fuse', type=Path, required=True)
    p.add_argument('--data', type=Path, default=ROOT / 'target/fuse-rebased/data')
    p.add_argument('--lmdb-source', type=Path, default=ROOT / 'target/fuse-engine/data/lmdb-bulk-1000000')
    p.add_argument('--out', type=Path, default=ROOT / 'validation/fuse-engine/rebased-confirm')
    p.add_argument('--trials', type=int, default=3)
    a = p.parse_args()
    data, out = a.data.resolve(), a.out.resolve()
    binaries = {'main': a.main.resolve(), 'fuse': a.fuse.resolve()}
    data.mkdir(parents=True, exist_ok=True)
    if a.phase == 'prepare':
        for label, engine in [('main', 'kiban'), ('fuse', 'kiban'), ('main', 'lmdb')]:
            path = fx(data, label, engine)
            if engine == 'lmdb':
                if not path.exists():
                    subprocess.run(['cp', '-a', '--reflink=auto', str(a.lmdb_source), str(path)], check=True)
                continue
            c = fuse_engine.config(engine, path, 'bulk', True, label == 'fuse', N)
            c['action'] = 'prepare'
            fuse_engine.execute(binaries[label], c, out / 'prepare', path.name, prepare=True)
    elif a.phase == 'measure':
        for action in ['read', 'miss']:
            for trial in range(1, a.trials + 1):
                for label in ['main', 'fuse'] if trial % 2 else ['fuse', 'main']:
                    for engine in ['kiban', 'lmdb']:
                        path = fx(data, label, engine)
                        c = fuse_engine.config(engine, path, 'bulk', True, label == 'fuse', N)
                        c.update(cache_mib=None, action=action)
                        name = f'bulk-default-index1-{action}-r{trial}-{label}-{engine}'
                        fuse_engine.execute(binaries[label], c, out / 'reads', name)
    else:
        rows = {}
        for f in sorted((out / 'reads').glob('*.receipt.json')):
            r = json.loads(f.read_text())
            _, _, _, action, trial, label, engine = f.name.removesuffix('.receipt.json').split('-')
            for s in r['stages']:
                rows.setdefault((s['workload'], label, engine), []).append(
                    (s['latency']['p50_ns'], s['latency']['p99_ns'], s['call_ops_s']))
        lines = ['| workload | engine | main p50 ns | fuse p50 ns | main ops/s | fuse ops/s | fuse/main ops | main p99 | fuse p99 |', '|---|---|---|---|---|---|---|---|---|']
        for w in sorted({k[0] for k in rows}):
            for engine in ['kiban', 'lmdb']:
                m, f = rows[(w, 'main', engine)], rows[(w, 'fuse', engine)]
                med = lambda xs, i: statistics.median(x[i] for x in xs)
                lines.append(f'| {w} | {engine} | {med(m,0):.0f} | {med(f,0):.0f} | {med(m,2):,.0f} | {med(f,2):,.0f} | {med(f,2)/med(m,2):.3f} | {med(m,1):.0f} | {med(f,1):.0f} |')
        (out / 'summary.md').write_text('\n'.join(lines) + '\n')
        print('\n'.join(lines))


if __name__ == '__main__':
    main()
