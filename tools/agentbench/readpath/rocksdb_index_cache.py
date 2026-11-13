#!/usr/bin/env python3
"""Cache-accounting fairness check: the frozen RocksDB read winner with its
index/filter blocks charged to the block cache (the published campaign) versus
RocksDB's default of keeping them in table-reader memory, which is how Kiban
holds its block index and filter. Same fixtures, same rotation, same runner."""
import argparse
import json
from pathlib import Path

import rocksdb_bench as rb

OUT = rb.ROOT / 'validation/rocksdb-index-cache/raw'


def winner():
    selection = json.loads((rb.ROOT / 'validation/rocksdb/raw/selection.json').read_text())
    return rb.CANDIDATES[selection['read']]


def prepare(args):
    chosen = winner()
    configs = [rb.base('lmdb', 1000000)]
    configs += [dict(rb.base('kiban', 1000000, shape), point_index=index)
                for shape in ('bulk', 'normal') for index in (False, True)]
    configs += [rb.base('rocksdb', 1000000, shape, tuning=dict(chosen)) for shape in ('bulk', 'normal')]
    for c in configs:
        c.update(action='prepare', db=str(rb.fixture(c)))
        rb.run(c, args.out / 'prepare', rb.fixture(c).name, args.resume)


def measure(args):
    chosen = winner()
    for shape in ('bulk', 'normal'):
        for cache in (512, None):
            scenario = f'read-1000000-{shape}-{cache}-fast'
            configs = [('kiban-index' + str(int(index)),
                        dict(rb.base('kiban', 1000000, shape, cache), point_index=index))
                       for index in (False, True)]
            configs += [('rocksdb-index-in-cache', rb.base('rocksdb', 1000000, shape, cache, dict(chosen))),
                        ('rocksdb-index-outside-cache',
                         rb.base('rocksdb', 1000000, shape, cache, dict(chosen, index_outside_cache=True))),
                        ('lmdb', rb.base('lmdb', 1000000, shape, cache))]
            for trial in range(5):
                for label, conf in rb.rotate(configs, trial):
                    c = dict(conf, action='read', durable=False, operations=100000)
                    c['db'] = str(rb.fixture(c))
                    rb.run(c, args.out / 'measure' / scenario, f'r{trial+1}-{label}', args.resume)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('action', choices=['prepare', 'measure'])
    parser.add_argument('--out', type=Path, default=OUT)
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args()
    {'prepare': prepare, 'measure': measure}[args.action](args)


if __name__ == '__main__':
    main()
