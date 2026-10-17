#!/usr/bin/env python3
"""Scoped, untimed CPU sampling. Run only after latency measurements finish."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import rocksdb_bench as bench

parser = argparse.ArgumentParser(__doc__)
parser.add_argument('--raw',type=Path,default=bench.ROOT/'validation/rocksdb/raw')
parser.add_argument('--layout',choices=['bulk','normal'],default='bulk')
parser.add_argument('--out',type=Path,default=bench.ROOT/'validation/rocksdb/sampling')
args = parser.parse_args()
args.out.mkdir(parents=True,exist_ok=True)
perf = bench.ROOT/'target/rocksdb/perf'
traces = bench.ROOT/'target/rocksdb/samples'/args.layout
traces.mkdir(parents=True,exist_ok=True)
selection = json.loads((args.raw/'selection.json').read_text())['read']
metadata = dict(layout=args.layout,binary_sha256=bench.digest(bench.BINARY),perf_sha256=bench.digest(perf),event='cpu_core/cycles/u',frequency=999,call_graph='dwarf,8192',cpu=10,operations=1000000,boundary='FIFO enabled only around full backend profile loop; includes digest and Memory destruction; excludes trace generation/open/full verification',traces={})
for stage in (1,2):
    configs = [('rocksdb',bench.base('rocksdb',1000000,layout=args.layout,cache=512,tuning=bench.CANDIDATES[selection])),('kiban-index1',dict(bench.base('kiban',1000000,layout=args.layout,cache=512),point_index=True))]
    for label,c in configs:
        name = f'{label}-stage{stage}'
        c.update(action='profile',stage=stage,operations=1000000,db=str(bench.fixture(c)))
        path = args.out/(name+'.config.json')
        path.write_text(json.dumps(c,indent=2)+'\n')
        trace = traces/(name+'.data')
        with tempfile.TemporaryDirectory(prefix='rocksdb-sample-') as tmp:
            ctl,ack = Path(tmp)/'ctl',Path(tmp)/'ack'
            os.mkfifo(ctl);os.mkfifo(ack)
            env = dict(os.environ,READBENCH_PERF_CTL=str(ctl),READBENCH_PERF_ACK=str(ack))
            with (args.out/(name+'.log')).open('w') as log:
                subprocess.run(['taskset','-c','10',str(perf),'record','-e','cpu_core/cycles/u','-F','999','--call-graph','dwarf,8192','--control',f'fifo:{ctl},{ack}','-D','-1','-o',str(trace),'--',str(bench.BINARY),str(path)],check=True,env=env,stdout=log,stderr=log,timeout=120)
        with (args.out/(name+'.report.txt')).open('w') as report:
            subprocess.run(['taskset','-c','12-21',str(perf),'report','--stdio','--no-children','--show-nr-samples','--sort','symbol','--percent-limit','0.5','-i',str(trace)],check=True,stdout=report,stderr=report,timeout=120)
        metadata['traces'][name] = dict(path=str(trace),sha256=bench.digest(trace),bytes=trace.stat().st_size)
        print(name,flush=True)
(args.out/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
