#!/usr/bin/env python3
"""Untimed diagnostic counters, never concurrent with rocksdb_bench.py."""
import json
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import rocksdb_bench as bench

ROOT = bench.ROOT
parser=argparse.ArgumentParser(__doc__)
parser.add_argument('--raw',type=Path,default=ROOT/'validation/rocksdb/raw')
parser.add_argument('--out',type=Path,default=ROOT/'validation/rocksdb/profile')
args=parser.parse_args()
OUT = args.out
PERF = ROOT / 'target/rocksdb/perf'
EVENTS = ','.join('cpu_core/' + e + '/u' for e in
                  ('cycles', 'instructions', 'LLC-load-misses', 'branch-misses', 'dTLB-load-misses'))
OUT.mkdir(parents=True, exist_ok=True)
metadata=dict(binary_sha256=bench.digest(bench.BINARY),perf_sha256=bench.digest(PERF),
    perf_version=subprocess.check_output([str(PERF),'--version'],text=True).strip(),
    cpu=10,events=['cycles:u','instructions:u','LLC-load-misses:u','branch-misses:u','dTLB-load-misses:u'],
    event_pmu='cpu_core',operations_per_loop=1000000,layout='bulk',cache_mib=512,
    perf_control='FIFO enable/disable only around profile loop; exclude preload and full verification',
    boundary='Full backend call plus digest and returned Memory destruction; minimal ID/count checks in loop')
(OUT/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
selection_all = json.loads((args.raw/'selection.json').read_text())
selection=selection_all['read']
configs = [('kiban-index0', dict(bench.base('kiban', 1000000, cache=512), point_index=False)),
           ('kiban-index1', dict(bench.base('kiban', 1000000, cache=512), point_index=True)),
           ('rocksdb', bench.base('rocksdb', 1000000, cache=512, tuning=bench.CANDIDATES[selection])),
           ('rocksdb-lz4', bench.base('rocksdb', 1000000, cache=512, tuning=dict(bench.CANDIDATES[selection],lz4=True))),
           ('lmdb', bench.base('lmdb', 1000000, cache=512))]
for stage in (1, 2):
    for label, c in configs:
        c.update(action='profile', stage=stage, operations=1000000, db=str(bench.fixture(c)))
        path = OUT / f'{label}-stage{stage}.config.json'
        path.write_text(json.dumps(c, indent=2)+'\n')
        with tempfile.TemporaryDirectory(prefix='rocksdb-perf-') as tmp:
            ctl, ack = Path(tmp)/'ctl', Path(tmp)/'ack'
            os.mkfifo(ctl); os.mkfifo(ack)
            env = {**os.environ, 'READBENCH_PERF_CTL':str(ctl), 'READBENCH_PERF_ACK':str(ack)}
            with (OUT / f'{label}-stage{stage}.log').open('w') as log:
                result = subprocess.run(['taskset','-c','10',str(PERF), 'stat', '-x',';',
                    '-e',EVENTS,'-o',str(OUT / f'{label}-stage{stage}.stat'),
                    '--control',f'fifo:{ctl},{ack}', '-D','-1','--',str(bench.BINARY),str(path)],
                    env=env, stdout=log, stderr=log, timeout=120)
                if result.returncode:
                    raise RuntimeError(f'perf failed: see {log.name}')
        print(path.name, flush=True)

summary=[]
for path in sorted(OUT.glob('*.stat')):
    counters={}
    for line in path.read_text().splitlines():
        if line and not line.startswith('#'):
            parts=line.split(';')
            if len(parts)>2 and parts[0].strip().isdigit():
                counters[parts[2]]=int(parts[0])/1000000
    summary.append(dict(name=path.stem,per_op=counters))
(OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
