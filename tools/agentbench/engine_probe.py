#!/usr/bin/env python3
"""Excluded diagnostics and runtime calibration; never a comparison campaign."""
import argparse
import json
import os
import pathlib
import sys

from engine_run import BINARY, Campaign, ENGINES, ENGINE, cpus, installation, validate_rows, write_json
from install import sha


def run(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    os.sched_setaffinity(0, cpus(args.driver_cpus))
    installation()
    campaign = Campaign(output,21600)
    profiler = output/'sync_profile.so'
    if args.sync_profile:
        campaign.command(['cc','-O2','-Wall','-Wextra','-Werror','-shared','-fPIC',ENGINE/'sync_profile.c',
                          '-o',profiler,'-ldl'],'compile-profiler')
    write_json(output/'metadata.json',{'purpose':'Excluded diagnostic/calibration, not reported latency evidence',
        'parameters':vars(args)|{'output':str(output)}, 'binary':str(BINARY),'binary_sha256':sha(BINARY),'helix_bm25':False})
    for mode in args.modes:
        for backend in args.engines:
            write_operations = min(args.operations,10000) if mode=='durable' else args.operations
            name = f'{mode}-{backend}'
            config = {'backend':backend,'mode':mode,'db':str(output/'data'/name),'n':args.n,
                'operations':args.operations,'k':10,'seed':20261016,'output':str(output/(name+'.jsonl')),
                'verify_only':False,'warmup':100,'write_operations':write_operations}
            path = output/(name+'.config.json')
            write_json(path,config)
            env = dict(os.environ)
            if args.sync_profile:
                assert not env.get('LD_PRELOAD')
                env.update(LD_PRELOAD=str(profiler),ENGINEBENCH_SYNC_PROFILE=str(output/(name+'.sync.json')))
            print(name,flush=True)
            campaign.command(['taskset','-c',args.cpus,BINARY,path],name,env=env)
            rows = [json.loads(line) for line in pathlib.Path(config['output']).read_text().splitlines()]
            validate_rows(rows,args.n,args.operations,write_operations)
            for row in rows:
                if row['type']=='stage':
                    print(f"  {row['workload']}: {row['wall_seconds']:.3f}s, {row['latency']['count']} checked calls",flush=True)
            if args.sync_profile:
                profile = json.loads((output/(name+'.sync.json')).read_text())
                assert profile['sync_errors']==0
                print('  sync profile:',profile,flush=True)
            verify = config|{'verify_only':True,'output':str(output/(name+'-reopen.jsonl'))}
            write_json(output/(name+'-reopen.config.json'),verify)
            campaign.command(['taskset','-c',args.cpus,BINARY,output/(name+'-reopen.config.json')],name+'-reopen')
            assert json.loads(pathlib.Path(verify['output']).read_text())=={'type':'reopen_verified','verified_records':args.n+write_operations}


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output',type=pathlib.Path,required=True)
    p.add_argument('--n',type=int,default=10000)
    p.add_argument('--operations',type=int,default=1000)
    p.add_argument('--engines',nargs='+',choices=ENGINES,default=list(ENGINES))
    p.add_argument('--modes',nargs='+',choices=['durable','fast'],default=['durable','fast'])
    p.add_argument('--cpus',default='1-4')
    p.add_argument('--driver-cpus',default='12-21')
    p.add_argument('--sync-profile',action='store_true')
    run(p.parse_args())
