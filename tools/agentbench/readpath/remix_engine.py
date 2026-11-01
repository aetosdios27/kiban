#!/usr/bin/env python3
"""Private-fixture REMIX engine experiment. Never builds; CPU 3 only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

import run

ROOT = run.ROOT
OUT = Path(os.environ.get('REMIX_ENGINE_OUT', ROOT/'validation/remix-engine'))
DATA = ROOT/'target/remix-data'
BIN = ROOT/'target/remix-bin'


def sibling():
    v = list(map(int, next(s for s in Path('/proc/stat').read_text().splitlines() if s.startswith('cpu4 ')).split()[1:9]))
    return dict(total=sum(v), idle=v[3]+v[4])


def execute(config, folder, name, binary):
    assert run.CPU == 3
    run.BINARY = binary.resolve()
    started = sibling()
    run.run(config, folder, name)
    ended = sibling()
    rows = [json.loads(s) for s in (folder/(name+'.jsonl')).read_text().splitlines()] if (folder/(name+'.jsonl')).exists() else []
    if config['action'] != 'prepare':
        assert rows and rows[-1]['type'] == 'complete', 'missing complete record'
    assert all(row.get('errors', 0) == 0 for row in rows)
    for row in rows:
        if row['type'] == 'stage':
            assert row['latency']['count'] == config['operations']
        if row['type'] == 'complete' and 'proc_status' in row:
            assert 'Cpus_allowed_list:\t3\n' in row['proc_status']
    with (folder/'sibling.jsonl').open('a') as f:
        f.write(json.dumps(dict(name=name, start=started, end=ended,
            busy_fraction=1-(ended['idle']-started['idle'])/max(1,ended['total']-started['total'])))+'\n')


def config(label, engine, layout, n=1000000, cache=512):
    c = run.config(engine,n,layout,cache)
    c.update(db=str(DATA/label/f'{engine}-{n}-{layout}'),point_index=True)
    return c


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['prepare','pilot','reads','varied','writes'])
    parser.add_argument('--trials',type=int,default=5)
    parser.add_argument('--select',default='')
    args = parser.parse_args()
    OUT.mkdir(parents=True,exist_ok=True)
    metadata = dict(cpu=3,sibling=4,build_cpus='12-21',nice_build=10,started=time.time(),load=os.getloadavg(),
        binaries={label:dict(path=str(BIN/label),sha256=hashlib.sha256((BIN/label).read_bytes()).hexdigest()) for label in ['before','after']})
    (OUT/(args.action+'-metadata.json')).write_text(json.dumps(metadata,indent=2)+'\n')
    if args.action=='prepare':
        # PH3 main fixtures are copied privately. New sections are written by
        # the real candidate's normal flush/compaction pipeline.
        source = Path('/home/aetos/.t3/worktrees/kiban/t3-f9d8504f/target/pointindex-cost/data/after')
        for layout in ['bulk','normal']:
            for engine in ['kiban','lmdb']:
                dest = DATA/'before'/f'{engine}-1000000-{layout}'
                dest.parent.mkdir(parents=True,exist_ok=True)
                if not dest.exists(): subprocess.run(['cp','-a','--reflink=auto',str(source/f'{engine}-{layout}'),str(dest)],check=True)
            dest = DATA/'after'/f'lmdb-1000000-{layout}'
            dest.parent.mkdir(parents=True,exist_ok=True)
            if not dest.exists(): subprocess.run(['cp','-a','--reflink=auto',str(source/f'lmdb-{layout}'),str(dest)],check=True)
            dest = DATA/'after'/f'kiban-1000000-{layout}'
            if not dest.exists():
                c = config('after','kiban',layout); c['action']='prepare'
                execute(c,OUT/'prepare',f'1000000-{layout}-after-kiban',BIN/'after')
        source = Path('/home/aetos/.t3/worktrees/kiban/t3-fb88ac18/target/latestk-data/on')
        for label in ['before','after']:
            for engine in ['kiban','lmdb']:
                dest=DATA/label/f'{engine}-10000-bulk'
                if dest.exists(): continue
                if label=='after' and engine=='kiban':
                    c=config(label,engine,'bulk',10000);c['action']='prepare'
                    execute(c,OUT/'prepare',f'10000-bulk-{label}-{engine}',BIN/label)
                else: subprocess.run(['cp','-a','--reflink=auto',str(source/f'{engine}-10000-bulk'),str(dest)],check=True)
        return
    for layout in ['bulk','normal'] if args.action!='writes' else ['bulk']:
        for cache in [512,None] if args.action not in ['pilot','writes'] else [512]:
            scenario=f'{layout}-'+('default' if cache is None else str(cache))
            if args.select not in scenario: continue
            if args.action=='writes': scenarios=[('fast',False),('durable',True)]
            else: scenarios=[(scenario,False)]
            for scenario,durable in scenarios:
                folder=OUT/args.action/scenario
                for trial in range(1,(1 if args.action=='pilot' else args.trials)+1):
                    for label in ['before','after'] if trial%2 else ['after','before']:
                        for engine in ['kiban','lmdb']:
                            name=f'1000000-{layout}-{cache}-r{trial}-{label}-{engine}'
                            if (folder/(name+'.jsonl')).exists():
                                receipts = folder/'commands.jsonl'
                                assert receipts.exists() and any(
                                    json.loads(line)['config'] == name+'.config.json'
                                    for line in receipts.read_text().splitlines()), 'partial result; preserve it in a separate attempt folder'
                                continue
                            c=config(label,engine,layout,10000 if args.action=='writes' else 1000000,cache)
                            c['action']='varied' if args.action=='varied' else 'read'
                            if args.action=='writes':
                                source=Path(c['db']); dest=source.with_name(source.name+'-write')
                                assert not dest.exists()
                                subprocess.run(['cp','-a','--reflink=auto',str(source),str(dest)],check=True)
                                c.update(db=str(dest),action='write',operations=1000,durable=durable)
                            print(args.action,scenario,name,flush=True)
                            execute(c,folder,name,BIN/label)
                            if args.action=='writes': shutil.rmtree(dest)


if __name__=='__main__': main()
