#!/usr/bin/env python3
"""Second-round paired driver: filter BEFORE the resident point index.

The first round (fuse_engine.py) compared the Bloom baseline against the
integrated binary fuse filter with the same read harness. This round holds
the writer/format constant — both sides read the frozen fuse fixtures
(after-kiban-*, identical SST bytes; the lookup-order change is read-only)
and runs five alternating before/after trials with the same LMDB control:
  before = frozen current-HEAD binary (fuse tables, point index probed
           before the filter), sha256 in provenance.json under "after"
  after  = same source plus the filter-first shortcut in SstTable::get
Only index-on cells are requested for this round; both caches and both
actions run, with per-trial ordering alternating as in the first round.
Receipts, configs, logs and hashes land in one results directory; reuse of
execute() keeps the assertion/affinity/accounting checks of the harness.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import fuse_engine  # noqa: E402

# fuse_engine.py computes its own ROOT as parents[3] of its own file path;
# HERE is the shared directory, so this file's worktree root is HERE.parents[2].
ROOT = HERE.parents[2]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--before', type=Path, required=True)
    p.add_argument('--after', type=Path, required=True)
    p.add_argument('--data', type=Path, default=ROOT / 'target/fuse-engine/data')
    p.add_argument('--out', type=Path, default=ROOT / 'validation/fuse-engine/results2')
    a = p.parse_args()
    a.data = a.data.resolve()
    a.out = a.out.resolve()
    binaries = {k: v.resolve() for k, v in [('before', a.before), ('after', a.after)]}
    a.out.mkdir(parents=True, exist_ok=True)
    runs = 0
    for layout in ['bulk', 'normal']:
        for cache in [32, None]:
            for action in ['read', 'miss']:
                for trial in range(1, 6):
                    for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
                        for engine in ['kiban', 'lmdb']:
                            # Both sides read the fuse fixtures; LMDB is
                            # format-independent, so its fixture name is too.
                            path = fuse_engine.fixture(
                                a.data, 'after' if engine == 'kiban' else 'before',
                                engine, layout, True)
                            c = fuse_engine.config(engine, path, layout, True, True)
                            c.update(cache_mib=cache, action=action)
                            name = f'{layout}-cache{cache}-index1-{action}-r{trial}-{label}-{engine}'
                            fuse_engine.execute(binaries[label], c, a.out / 'reads', name)
                            runs += 1
    print('completed', runs, 'paired engine runs into', a.out)
    import hashlib
    (a.out / 'round-binaries.json').write_text(json.dumps(
        dict(n_before_runs=runs // 2,
             before_sha256=hashlib.sha256(a.before.read_bytes()).hexdigest(),
             after_sha256=hashlib.sha256(a.after.read_bytes()).hexdigest(),
             data=str(a.data)),
        indent=2,
    ) + '\n')


if __name__ == '__main__':
    main()
