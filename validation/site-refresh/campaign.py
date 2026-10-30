#!/usr/bin/env python3
"""One-commit, four-engine agent-memory campaign for the website's scoreboard.

Reuses the verified runner from tools/agentbench/readpath/rocksdb_bench.py.
Kiban uses the agent_memory settings (point index plus an 18-byte scan prefix).
RocksDB reuses the frozen winner in validation/rocksdb/raw/selection.json, whose
screen is unaffected by engine changes. SQLite uses its fastest read row.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/agentbench/readpath'))
import rocksdb_bench as rb  # noqa: E402

MMAP_ALL_LEVELS = 4294967295
SQLITE_MMAP = 1 << 30
READS = [(layout, cache) for cache in (None, 32) for layout in ('normal', 'bulk')]


def config(engine, n, layout='bulk', cache=None, durable=False):
    selection = json.loads((ROOT / 'validation/rocksdb/raw/selection.json').read_text())
    family = 'write' if n == 10000 else 'read'
    tuning = rb.CANDIDATES[selection[family]] if engine == 'rocksdb' else {}
    c = rb.base(engine, n, layout, cache, tuning)
    c['point_index'] = engine == 'kiban'
    if engine == 'sqlite':
        c.update(sqlite_layout='rowid', sqlite_sync='FULL' if durable else 'NORMAL',
                 sqlite_mmap_bytes=0 if n == 10000 else SQLITE_MMAP)
    return c


def prepare(out):
    for n, layouts in ((10000, ('bulk',)), (1000000, ('bulk', 'normal'))):
        for engine in ('kiban', 'rocksdb', 'sqlite', 'lmdb'):
            for layout in layouts:
                c = config(engine, n, layout)
                c.update(action='prepare', db=str(rb.fixture(c)))
                if not rb.fixture(c).exists():
                    rb.run(c, out / 'prepare', rb.fixture(c).name, True)


def measure(out):
    for layout, cache in READS:
        entries = [(e, config(e, 1000000, layout, cache)) for e in ('kiban', 'rocksdb', 'sqlite', 'lmdb')]
        if cache == 32:
            entries[0][1]['mmap_max_level'] = MMAP_ALL_LEVELS
            entries.append(('kiban-pread', config('kiban', 1000000, layout, cache)))
        scenario = f'read-{layout}-{cache or "default"}'
        for trial in range(5):
            for label, conf in rb.rotate(entries, trial):
                c = dict(conf, action='read', durable=False, db=str(rb.fixture(conf)))
                rb.run(c, out / 'measure' / scenario, f'r{trial + 1}-{label}', True)
    entries = [(e, config(e, 10000, durable=True)) for e in ('kiban', 'rocksdb', 'sqlite', 'lmdb')]
    for trial in range(5):
        for label, conf in rb.rotate(entries, trial):
            c = dict(conf, action='write', durable=True, operations=1000, db=str(rb.DATA / 'write-trial'))
            rb.run(c, out / 'measure' / 'write-durable', f'r{trial + 1}-{label}', True)
    for label, conf in entries:
        c = dict(conf, action='write', durable=True, operations=1000, db=str(rb.DATA / 'probe-trial'))
        rb.run(c, out / 'sync-probe', label, True, ROOT / 'target/sync-probe.so')


def provenance(out):
    binary = rb.BINARY
    meta = dict(commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                engine_src_commit=subprocess.check_output(['git', 'log', '-1', '--format=%H', '--', 'src'], cwd=ROOT, text=True).strip(),
                binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), cpu=10,
                selection=json.loads((ROOT / 'validation/rocksdb/raw/selection.json').read_text()))
    path = out / 'metadata.json'
    if path.exists():
        assert json.loads(path.read_text()) == meta, 'binary or commit changed mid-campaign'
    path.write_text(json.dumps(meta, indent=2) + '\n')


parser = argparse.ArgumentParser()
parser.add_argument('--out', type=Path, default=ROOT / 'validation/site-refresh/raw')
args = parser.parse_args()
args.out = args.out.resolve()
args.out.mkdir(parents=True, exist_ok=True)
provenance(args.out)
prepare(args.out)
measure(args.out)
shutil.rmtree(rb.DATA / 'probe-trial', ignore_errors=True)
print('ALLDONE', flush=True)
