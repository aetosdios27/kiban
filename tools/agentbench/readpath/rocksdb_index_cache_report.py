#!/usr/bin/env python3
"""Summarize rocksdb_index_cache.py: medians of five rotated trials, paired
same-round ratios, peak RSS. Every result digest is rechecked first."""
import collections
import hashlib
import json
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RAW = ROOT / 'validation/rocksdb-index-cache/raw/measure'
LABELS = ['kiban-index0', 'kiban-index1', 'rocksdb-index-in-cache', 'rocksdb-index-outside-cache', 'lmdb']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(folder):
    values = collections.defaultdict(dict)
    receipts = [json.loads(s) for s in (folder / 'commands.jsonl').read_text().splitlines()]
    assert len(receipts) == 5 * len(LABELS), folder
    for command in receipts:
        trial, label = command['name'].split('-', 1)
        result = folder / (command['name'] + '.jsonl')
        assert sha(result) == command['result_sha256'], result
        rows = [json.loads(s) for s in result.read_text().splitlines()]
        assert all(r.get('errors', 0) == 0 for r in rows)
        logged = [json.loads(s) for s in (folder / (command['name'] + '.log')).read_text().splitlines() if s.startswith('{')]
        if label.startswith('rocksdb'):
            want = label == 'rocksdb-index-in-cache'
            text = json.dumps([r for r in logged if r.get('type') in ('backend_config', 'requested_config')])
            assert text.count('index_outside_cache') >= 1, f'config not logged: {command["name"]}'
            assert (f'"index_outside_cache": {str(not want).lower()}' in text
                    or f'"index_outside_cache":{str(not want).lower()}' in text), command['name']
        rss = int(next(s.split(':', 1)[1] for s in (folder / (command['name'] + '.time.txt')).read_text().splitlines()
                       if 'Maximum resident set size' in s))
        for row in (r for r in rows if r['type'] == 'stage'):
            assert row['latency']['count'] == 100000
            values[label, row['workload']][trial] = dict(p50=row['latency']['p50_ns'], p99=row['latency']['p99_ns'],
                                                         ops=row['call_ops_s'], rss=rss)
    return values


def med(xs):
    return statistics.median(xs)


lines = ['# RocksDB index/filter placement: fairness check', '',
         'Medians of five rotated trials, 1M records, 100k calls per operation. '
         'Ratios are same-round pairs, median over five rounds; below 1 favors the row engine.', '']
for folder in sorted(RAW.iterdir()):
    v = load(folder)
    lines += [f'## {folder.name}', '',
              '| Operation | Engine | p50 µs | p99 µs | kops/s | peak RSS MiB | p50 / RocksDB in-cache | p50 / RocksDB outside |',
              '|---|---|---:|---:|---:|---:|---:|---:|']
    for work in sorted({w for _, w in v}):
        for label in LABELS:
            d = v[label, work]
            trials = sorted(d)
            ratio = lambda ref: med([d[t]['p50'] / v[ref, work][t]['p50'] for t in trials])
            lines.append(f'| {work} | {label} | {med([d[t]["p50"] for t in trials])/1000:.3f} | '
                         f'{med([d[t]["p99"] for t in trials])/1000:.3f} | {med([d[t]["ops"] for t in trials])/1000:.1f} | '
                         f'{med([d[t]["rss"] for t in trials])/1024:.0f} | {ratio("rocksdb-index-in-cache"):.3f} | '
                         f'{ratio("rocksdb-index-outside-cache"):.3f} |')
    lines.append('')
out = ROOT / 'validation/rocksdb-index-cache/RESULTS.md'
out.write_text('\n'.join(lines))
print(out.read_text() if '-q' not in sys.argv else out)
