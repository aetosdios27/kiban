#!/usr/bin/env python3
"""Excluded Helix large-N calibration with a host-memory bound; no comparison claims."""
import argparse
import json
import os
import pathlib
import signal
import subprocess
import sys
import time

from engine_run import HERE, ROOT, cpus, write_json


def run(args):
    root=args.output.resolve()
    root.mkdir(parents=True,exist_ok=False)
    os.sched_setaffinity(0,cpus(args.driver_cpus))
    available=int(next(line.split()[1] for line in pathlib.Path('/proc/meminfo').read_text().splitlines()
        if line.startswith('MemAvailable:')))*1024
    limit=min(int(args.max_rss_gib*1024**3),available//2)
    if limit<1024**3:raise RuntimeError('Insufficient available RAM for guarded calibration')
    argv=[sys.executable,'-u',str(HERE/'engine_probe.py'),'--output',str(root/'probe'),
        '--n',str(args.n),'--operations',str(args.operations),'--engines','helix','--modes','fast',
        '--cpus',args.cpus,'--driver-cpus',args.driver_cpus]
    maximum=0;reason=None;started=time.monotonic();samples=0
    write_json(root/'guard.json',{'status':'running','comparison_eligible':False,'argv':argv,
        'available_bytes_at_start':available,'rss_limit_bytes':limit})
    with (root/'driver.log').open('w') as log,(root/'rss.jsonl').open('w') as rss:
        proc=subprocess.Popen(argv,cwd=ROOT,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        while proc.poll() is None:
            children=pathlib.Path(f'/proc/{proc.pid}/task/{proc.pid}/children')
            for child in children.read_text().split() if children.exists() else []:
                try:
                    if b'kiban-enginebench' not in pathlib.Path(f'/proc/{child}/cmdline').read_bytes():continue
                    status=pathlib.Path(f'/proc/{child}/status').read_text()
                    metrics={line.split(':',1)[0]:int(line.split()[1])*1024 for line in status.splitlines()
                        if line.startswith(('VmRSS:','VmHWM:','VmSwap:'))}
                    resident=metrics.get('VmRSS',0);maximum=max(maximum,resident)
                    raw=root/'probe/fast-helix.jsonl'
                    stages=[json.loads(line)['workload'] for line in raw.read_text().splitlines()
                        if json.loads(line)['type']=='stage'] if raw.exists() else []
                    rss.write(json.dumps({'epoch':time.time(),'pid':int(child),'last_completed_stage':stages[-1] if stages else 'preload/remember',**metrics})+'\n')
                    rss.flush();samples+=1
                    if resident>limit and reason is None:
                        reason='rss_limit';os.killpg(int(child),signal.SIGTERM)
                except (FileNotFoundError,ProcessLookupError):pass
            if time.monotonic()-started>args.timeout:
                reason='timeout'
                for child in children.read_text().split() if children.exists() else []:
                    try:os.killpg(int(child),signal.SIGTERM)
                    except ProcessLookupError:pass
                os.killpg(proc.pid,signal.SIGTERM)
            time.sleep(0.1)
        code=proc.wait()
    result={'status':'stopped_diagnostic' if reason or code else 'completed_diagnostic',
        'comparison_eligible':False,'reason':reason,'returncode':code,'argv':argv,
        'available_bytes_at_start':available,'rss_limit_bytes':limit,'max_observed_rss_bytes':maximum,
        'samples':samples,'wall_seconds':time.monotonic()-started,
        'note':'Excluded calibration, not a replacement for the three-trial large-N matrix.'}
    write_json(root/'guard.json',result)
    print(json.dumps(result,indent=2))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output',type=pathlib.Path,required=True)
    p.add_argument('--n',type=int,default=1000000)
    p.add_argument('--operations',type=int,default=1000)
    p.add_argument('--max-rss-gib',type=float,default=6)
    p.add_argument('--timeout',type=int,default=600)
    p.add_argument('--cpus',default='1-4')
    p.add_argument('--driver-cpus',default='12-21')
    run(p.parse_args())
