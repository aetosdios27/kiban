#!/usr/bin/env python3
"""CPU-6 paired small-cache experiments; never builds or uses other timed CPUs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

import run

ROOT = run.ROOT
DATA = ROOT / 'target/smallcache/data'
OUT = ROOT / 'validation/smallcache'
BINS = ROOT / 'target/smallcache/bin'
run.CPU = 6


def config(engine, layout, indexed, cache, n=1000000):
    c = run.config(engine, n, layout)
    c.update(db=str(DATA / f'{engine}-{n}-{layout}-{int(indexed) if engine == "kiban" else 0}'),
             point_index=indexed, cache_mib=cache)
    return c


def execute(label, c, folder, name, binary_label=None):
    run.BINARY = BINS / (binary_label or label)
    folder.mkdir(parents=True, exist_ok=True)
    if (folder / (name + '.config.json')).exists():
        raise RuntimeError('Refusing overwrite: ' + name)
    print(name, flush=True)
    run.run(c, folder, name)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('action', choices=['prepare', 'profile', 'read', 'cold', 'write'])
    p.add_argument('--after', default='after')
    p.add_argument('--candidate-data', type=Path)
    p.add_argument('--mmap-mode', choices=['none','all'], default='none')
    p.add_argument('--out', type=Path)
    p.add_argument('--pilot', action='store_true')
    p.add_argument('--indexed', action='store_true', help='Profile indexed fixtures')
    p.add_argument('--mapped', action='store_true', help='Compare final pread versus final mapped access')
    p.add_argument('--syscalls', action='store_true', help='Diagnostic preload; never latency trials')
    p.add_argument('--layouts', nargs='+', choices=['bulk', 'normal'])
    p.add_argument('--caches', nargs='+', choices=['32', 'default', '512'])
    p.add_argument('--indexes', type=int, nargs='+', choices=[0, 1])
    a = p.parse_args()
    if a.syscalls and a.action != 'profile':
        p.error('--syscalls is restricted to diagnostic profile loops')
    if a.mapped and a.action not in ['read', 'cold']:
        p.error('--mapped compares access modes for read/cold actions')
    folder = a.out or OUT / a.action
    folder.mkdir(parents=True, exist_ok=True)
    metadata = dict(cpu=6, task_sibling_unused=7, build_cpus='12-21', time=time.time(),
                    head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                    experiment='final pread versus final mapped' if a.mapped else 'main versus candidate',
                    load=os.getloadavg(), binaries={label: hashlib.sha256((BINS/(a.after if a.mapped else label)).read_bytes()).hexdigest()
                                                   for label in ['before', a.after] if (BINS/label).exists()})
    (folder/'metadata.json').write_text(json.dumps(metadata, indent=2)+'\n')
    if a.action == 'prepare':
        for n in [10000, 1000000]:
            for layout in ['bulk', 'normal']:
                for engine, indexed in [('kiban', False), ('kiban', True), ('lmdb', False)]:
                    c = config(engine, layout, indexed, 32, n)
                    c['action'] = 'prepare'
                    execute('before', c, folder, f'{engine}-{n}-{layout}-{int(indexed)}')
        rows = []
        for path in sorted(DATA.glob('kiban-*')):
            ssts = list(path.glob('*.sst'))
            point_bytes = 0
            for sst in ssts:
                with sst.open('rb') as f:
                    f.seek(-68, 2)
                    footer = f.read()
                if int.from_bytes(footer[-28:-24], 'little') == 4:
                    point_bytes += int.from_bytes(footer[8:16], 'little')
            rows.append(dict(dataset=path.name, tables=len(ssts), sst_bytes=sum(f.stat().st_size for f in ssts), point_bytes=point_bytes))
        (folder/'space.json').write_text(json.dumps(rows, indent=2)+'\n')
    elif a.action == 'profile':
        os.environ['READBENCH_BUILD_CPUS'] = '12,13,14,15,16,17,18,19,20,21'
        os.environ['READBENCH_CPU'] = '6'
        import profile as profiler
        profiler.CPU = 6
        for label in ['before', a.after] if a.after != 'before' else ['before']:
            profiler.BINARY = BINS / label
            for layout in ['bulk', 'normal']:
                for engine in ['kiban', 'lmdb']:
                    c = config(engine, layout, a.indexed, 32)
                    c.update(action='profile', stage=1, operations=1000000, mmap_max_level=4294967295 if a.mmap_mode == 'all' else None)
                    if a.candidate_data and label != 'before' and engine == 'kiban' and a.indexed:
                        c['db'] = str(a.candidate_data / Path(c['db']).name)
                    if a.syscalls:
                        os.environ['LD_PRELOAD'] = str(ROOT/'target/smallcache/syscall_probe.so')
                    name = f'{label}-{layout}-{engine}'
                    print(name, flush=True)
                    profiler.collect(c, folder, name)
                    if not a.syscalls:
                        profiler.collect(c, folder, name+'-stacks', record=True, flamegraph=False)
    else:
        layouts = a.layouts or (['bulk'] if a.pilot else ['bulk', 'normal'])
        indexes = [bool(i) for i in a.indexes] if a.indexes else ([True] if a.pilot else [False, True])
        caches = [None if c == 'default' else int(c) for c in a.caches] if a.caches else ([32] if a.pilot else [32, None])
        for layout in layouts:
            for indexed in indexes:
                for cache in caches:
                    if a.action == 'write' and (layout != 'bulk' or cache is None):
                        continue
                    for readers in ([1, 4] if a.action == 'cold' else [1]):
                        for durable in ([False, True] if a.action == 'write' else [False]):
                            scenario = f'{layout}-index{int(indexed)}-cache{cache}-readers{readers}-durable{int(durable)}'
                            for trial in range(1, 6):
                                for label in (['before', a.after] if trial % 2 else [a.after, 'before']):
                                    for engine in ['kiban', 'lmdb']:
                                        n = 10000 if a.action == 'write' else 1000000
                                        c = config(engine, layout, indexed, cache, n)
                                        c.update(action=a.action, readers=readers, durable=durable)
                                        c['mmap_max_level'] = 4294967295 if a.mmap_mode == 'all' else None
                                        if a.candidate_data and label != 'before' and engine == 'kiban' and indexed:
                                            c['db'] = str(a.candidate_data / Path(c['db']).name)
                                        if a.mapped:
                                            c['mmap_max_level'] = 4294967295 if label == a.after else None
                                        if a.action == 'cold':
                                            c['operations'] = 10000
                                        name = f'{scenario}-r{trial}-{label}-{engine}'
                                        if a.action == 'write':
                                            src = Path(c['db'])
                                            dest = DATA / ('write-'+name)
                                            if dest.exists():
                                                raise RuntimeError('Existing write fixture')
                                            subprocess.run(['cp', '-a', '--reflink=auto', str(src), str(dest)], check=True)
                                            c.update(db=str(dest), operations=10000)
                                        execute(label, c, folder, name, a.after if a.mapped else None)
                                        if a.action == 'write':
                                            shutil.rmtree(c['db'])


if __name__ == '__main__':
    main()
