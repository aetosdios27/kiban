#!/usr/bin/env python3
"""Gated CPU-8 full-operation counters outside the latency matrix."""
import argparse, hashlib, json, os, subprocess, tempfile
from pathlib import Path
import v2
ROOT=v2.ROOT;OUT=ROOT/'validation/v2/profile'
PERF=Path('/home/aetos/.t3/worktrees/kiban/t3-53ad0f47/target/read-path-tools/root/usr/bin/perf')
EVENTS=','.join('cpu_core/'+event+'/u' for event in ['cycles','instructions','LLC-load-misses','dTLB-load-misses','branch-misses'])
def main():
 global OUT
 p=argparse.ArgumentParser();p.add_argument('--before',type=Path,required=True);p.add_argument('--after',type=Path,required=True);p.add_argument('--stage',type=int,choices=[1,2],default=1);p.add_argument('--layout',choices=['bulk','normal']);p.add_argument('--out',type=Path);a=p.parse_args();OUT=a.out or OUT
 OUT.mkdir(parents=True,exist_ok=True)
 (OUT/'metadata.json').write_text(json.dumps(dict(cpu=8,events=EVENTS,operations=1000000,binaries={l:dict(path=str(getattr(a,l).resolve()),sha256=hashlib.sha256(getattr(a,l).read_bytes()).hexdigest()) for l in ['before','after']}),indent=2))
 for layout in ([a.layout] if a.layout else ['bulk','normal']):
  for label in ['before','after']:
   for engine in ['kiban','lmdb']:
    name=f'{layout}-{label}-{engine}';c=v2.config(engine,1000000,layout,'512',label);c.update(action='profile',stage=a.stage,operations=1000000)
    path=OUT/(name+'.config.json');path.write_text(json.dumps(c,indent=2))
    with tempfile.TemporaryDirectory(prefix='v2-perf-') as tmp:
     ctl,ack=Path(tmp)/'ctl',Path(tmp)/'ack';os.mkfifo(ctl);os.mkfifo(ack)
     env={**os.environ,'READBENCH_PERF_CTL':str(ctl),'READBENCH_PERF_ACK':str(ack)}
     with (OUT/(name+'.log')).open('w') as out:
      subprocess.run(['taskset','-c','8',str(PERF),'stat','-x',';','-e',EVENTS,'-o',str(OUT/(name+'.stat')),'--control',f'fifo:{ctl},{ack}','-D','-1','--',str(getattr(a,label).resolve()),str(path)],check=True,env=env,stdout=out,stderr=out)
    print(name,flush=True)
if __name__=='__main__':main()
