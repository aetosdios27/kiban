#!/usr/bin/env python3
"""Confirm rebased reads against current main, preserving the frozen campaign."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

import smallcache as campaign


def sibling_ticks():
    row = next(s for s in Path('/proc/stat').read_text().splitlines() if s.startswith('cpu7 '))
    return list(map(int, row.split()[1:9]))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['prepare-main', 'read'])
    parser.add_argument('--caches', nargs='+', choices=['32', 'default', '512'], default=['32'])
    parser.add_argument('--layouts', nargs='+', choices=['bulk', 'normal'], default=['bulk', 'normal'])
    parser.add_argument('--modes', nargs='+', choices=['mapped', 'pread'], default=['mapped', 'pread'])
    parser.add_argument('--trials', type=int, choices=range(1, 6), default=3)
    parser.add_argument('--prefix', default='rebased')
    args = parser.parse_args()
    root, out = campaign.ROOT, campaign.OUT
    main_data = root/'target/smallcache/main-rebased-data'
    candidate_data = root/'target/smallcache/ph3-data'
    if args.action == 'prepare-main':
        for layout in args.layouts:
            c = campaign.config('kiban', layout, True, 32)
            c.update(action='prepare', db=str(main_data/Path(c['db']).name))
            assert not Path(c['db']).exists(), c['db']
            campaign.execute('main-rebased', c, out/'rebased-main-prepare', layout)
        return

    checks = out/'rebased-checks'
    status = json.loads((checks/'status.json').read_text())
    assert len(status) == 11 and all(r['code'] == 0 for r in status)
    hashes = json.loads((checks/'source-hashes.json').read_text())
    assert all(hashlib.sha256((root/p).read_bytes()).hexdigest() == h for p, h in hashes.items())
    provenance = dict(
        baseline=subprocess.check_output(['git', 'rev-parse', 'origin/main'], text=True).strip(),
        head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        engine_source_hashes=hashes, trials=args.trials, caches=args.caches,
        layouts=args.layouts, modes=args.modes,
        cpu=6, task_sibling_unused=7, build_cpus='12-21', nice=10,
        coordinator_affinity=sorted(os.sched_getaffinity(0)),
        baseline_fixture='Fresh flag-zero PH3 written by main-rebased binary',
        candidate_fixture='Frozen PH3 bit-0 fixtures; same production engine code after rebase',
        binaries={s:hashlib.sha256((campaign.BINS/s).read_bytes()).hexdigest()
                  for s in ['main-rebased', 'rebased']})
    (out/f'{args.prefix}-provenance.json').write_text(json.dumps(provenance, indent=2)+'\n')
    for mode in args.modes:
        folder = out/f'{args.prefix}-{mode}-read'
        for layout in args.layouts:
            for cache_label in args.caches:
                cache = None if cache_label == 'default' else int(cache_label)
                for trial in range(1, args.trials + 1):
                    for side in (['before', 'rebased'] if trial % 2 else ['rebased', 'before']):
                        binary = 'main-rebased' if side == 'before' else 'rebased'
                        for engine in ['kiban', 'lmdb']:
                            c = campaign.config(engine, layout, True, cache)
                            if engine == 'kiban':
                                data = main_data if side == 'before' else candidate_data
                                c['db'] = str(data/Path(c['db']).name)
                            c.update(action='read', readers=1, durable=False,
                                     mmap_max_level=4294967295 if mode == 'mapped' else None)
                            name = f'{layout}-index1-cache{cache}-readers1-durable0-r{trial}-{side}-{engine}'
                            start = sibling_ticks()
                            campaign.execute(side, c, folder, name, binary)
                            delta = [b-a for a,b in zip(start, sibling_ticks())]
                            with (folder/'sibling.jsonl').open('a') as log:
                                log.write(json.dumps(dict(name=name, cpu=7, total_ticks=sum(delta),
                                    busy_ticks=sum(delta)-delta[3]-delta[4],
                                    scope='Whole child process including open and verification'))+'\n')
        subprocess.run([sys.executable, str(root/'tools/agentbench/readpath/smallcache_report.py'),
                        str(folder), '--after', 'rebased', '--trials', str(args.trials)], check=True)
    assert all(hashlib.sha256((root/p).read_bytes()).hexdigest() == h for p, h in hashes.items())


if __name__ == '__main__':
    main()
