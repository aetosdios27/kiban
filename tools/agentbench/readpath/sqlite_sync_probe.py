#!/usr/bin/env python3
"""Count libc sync calls in complete write suites, outside the latency matrix.
Build the diagnostic shared library first on CPUs 12-21, with no timed run active.
"""
import json
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sqlite_bench as bench

parser=argparse.ArgumentParser(__doc__)
parser.add_argument('--raw',type=Path,default=bench.ROOT/'validation/sqlite/raw')
parser.add_argument('--out',type=Path,default=bench.ROOT/'validation/sqlite/sync-probe')
args=parser.parse_args()
out = args.out
out.mkdir(parents=True, exist_ok=True)
selection = json.loads((args.raw/'selection.json').read_text())['write']
configs = []
for durable in (False, True):
    mode = 'durable' if durable else 'fast'
    for engine in ('kiban','lmdb'):
        configs.append((f'{engine}-{mode}', dict(bench.base(engine,10000), durable=durable)))
for sync in ('OFF','NORMAL','FULL'):
    configs.append(('sqlite-'+sync.lower(), dict(bench.base('sqlite',10000),
                    sqlite_layout=selection, sqlite_sync=sync)))
alternate = bench.ROOT/'target/sqlite/fdatasync-target/release/kiban-readbench'
if alternate.exists():
    configs.append(('sqlite-full-fdatasync',dict(bench.base('sqlite',10000),
        sqlite_layout=selection,sqlite_sync='FULL',durable=True)))
records = []
for label, c in configs:
    dest = bench.DATA / 'sync-probe-trial'
    assert not dest.exists(), 'previous diagnostic copy remains'
    subprocess.run(['cp','-a','--reflink=auto',str(bench.fixture(c)),str(dest)],check=True)
    c.update(action='write',operations=1000,db=str(dest),output=str(out/(label+'.jsonl')))
    Path(c['output']).unlink(missing_ok=True)
    path = out/(label+'.config.json')
    path.write_text(json.dumps(c,indent=2)+'\n')
    env = {**os.environ,'LD_PRELOAD':str(bench.ROOT/'target/sqlite/bin/sync-probe.so')}
    with (out/(label+'.log')).open('w') as log:
        subprocess.run(['taskset','-c','10',str(alternate if label=='sqlite-full-fdatasync' else bench.BINARY),str(path)],env=env,
                       stdout=log,stderr=log,check=True)
    text = (out/(label+'.log')).read_text()
    counts = next(s for s in text.splitlines() if s.startswith('sync_probe '))
    record = dict(label=label,**{k:int(v) for k,v in (p.split('=') for p in counts.split()[1:])})
    records.append(record)
    shutil.rmtree(dest)
(out/'summary.json').write_text(json.dumps(records,indent=2)+'\n')
print(json.dumps(records,indent=2))
