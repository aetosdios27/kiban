#!/usr/bin/env python3
"""CPU-3 FIFO-gated counters and stacks; startup and validation are excluded."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

import remix_engine as suite

PERF=Path('/home/aetos/.t3/worktrees/kiban/t3-53ad0f47/target/read-path-tools/root/usr/bin/perf')
EVENTS=','.join('cpu_core/'+event+'/u' for event in ['cycles','instructions','branch-misses','LLC-load-misses'])


def collect(c,name,record=False):
    folder=suite.OUT/'profile';folder.mkdir(exist_ok=True)
    path=folder/(name+'.config.json');path.write_text(json.dumps(c,indent=2)+'\n')
    binary=suite.BIN/('after' if '-after-' in name else 'before')
    with tempfile.TemporaryDirectory(prefix='remix-perf-') as tmp:
        ctl,ack=Path(tmp)/'ctl',Path(tmp)/'ack';os.mkfifo(ctl);os.mkfifo(ack)
        env={**os.environ,'READBENCH_PERF_CTL':str(ctl),'READBENCH_PERF_ACK':str(ack)}
        if record:cmd=['record','-e','cpu_core/cycles/u','--no-inherit','--user-callchains','-m','1024','-c','1000000','-g','--call-graph','dwarf,8192','-o',str(folder/(name+'.data'))]
        else:cmd=['stat','-x',';','-e',EVENTS,'-o',str(folder/(name+'.stat'))]
        cmd=['taskset','-c','3',str(PERF),*cmd,'--control',f'fifo:{ctl},{ack}','-D','-1','--',str(binary),str(path)]
        start=suite.sibling()
        with (folder/(name+'.log')).open('w') as f:subprocess.run(cmd,env=env,stdout=f,stderr=f,check=True)
        end=suite.sibling()
        with (folder/'commands.jsonl').open('a') as f:f.write(json.dumps(dict(name=name,command=cmd,start=start,end=end))+'\n')
    if record:
        with (folder/(name+'.self.txt')).open('w') as f:subprocess.run([str(PERF),'report','--stdio','--no-children','-i',str(folder/(name+'.data'))],stdout=f,check=True)
        with gzip.open(folder/(name+'.stacks.txt.gz'),'wb') as f:
            result=subprocess.run([str(PERF),'script','-i',str(folder/(name+'.data'))],stdout=subprocess.PIPE,check=True)
            f.write(result.stdout)
        (folder/(name+'.data')).unlink()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--stacks',action='store_true');args=parser.parse_args()
    folder=suite.OUT/'profile';folder.mkdir(exist_ok=True)
    (folder/'metadata.json').write_text(json.dumps(dict(cpu=3,decode_cpu='12-21',perf=str(PERF),events=EVENTS,binaries={s:hashlib.sha256((suite.BIN/s).read_bytes()).hexdigest() for s in ['before','after']}),indent=2)+'\n')
    for layout in ['bulk','normal']:
        for label in ['before','after']:
            for engine in ['kiban','lmdb']:
                c=suite.config(label,engine,layout);c.update(action='profile',stage=2,operations=1000000)
                name=f'{layout}-{label}-{engine}'
                print(name,flush=True);collect(c,name)
                if args.stacks and engine=='kiban' and layout=='normal':collect(c,name+'-stacks',True)


if __name__=='__main__':main()
