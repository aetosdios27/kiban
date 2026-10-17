#!/usr/bin/env python3
"""Interleave frozen before/after binaries, each followed by its LMDB control."""
import argparse
import hashlib
import json
from pathlib import Path
import run
import subprocess
import shutil

parser=argparse.ArgumentParser()
parser.add_argument('--before',type=Path,required=True)
parser.add_argument('--after',type=Path,required=True)
parser.add_argument('--out',type=Path,required=True)
parser.add_argument('--n',type=int,default=1000000)
parser.add_argument('--layout',choices=['bulk','normal'],default='bulk')
parser.add_argument('--cache',type=lambda value: None if value == 'default' else int(value),default=512)
parser.add_argument('--trials',type=int,default=5)
parser.add_argument('--write',action='store_true')
parser.add_argument('--durable',action='store_true')
parser.add_argument('--resume',action='store_true')
args=parser.parse_args()
binaries={'before':args.before.resolve(),'after':args.after.resolve()}
args.out.mkdir(parents=True,exist_ok=True)
metadata=dict(binaries={k:dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for k,p in binaries.items()},trials=args.trials)
metadata_path=args.out/'metadata.json'
completed=set()
if args.resume and metadata_path.exists():
    assert json.loads(metadata_path.read_text())==metadata, 'resume binaries changed'
    for line in (args.out/'commands.jsonl').read_text().splitlines():
        command=json.loads(line)
        rows=[json.loads(line) for line in (args.out/command['config'].replace('.config.json','.jsonl')).read_text().splitlines()]
        assert rows[-1]['type']=='complete' and all(r.get('errors',0)==0 for r in rows), 'invalid saved trial'
        completed.add(command['config'])
else:
    assert not (args.out/'commands.jsonl').exists(), 'use --resume or a fresh output folder'
metadata_path.write_text(json.dumps(metadata,indent=2)+'\n')
for trial in range(args.trials):
    for label in ('before','after') if trial%2==0 else ('after','before'):
        run.BINARY=binaries[label]
        for engine in ('kiban','lmdb'):
            c=run.config(engine,args.n,args.layout,args.cache)
            name=f'{args.n}-{args.layout}-{args.cache}-r{trial+1}-{label}-{engine}'
            if name+'.config.json' in completed:
                continue
            # An interrupted command is absent from commands.jsonl; discard only
            # its incomplete output before rerunning the identical configuration.
            if args.resume:
                (args.out/(name+'.jsonl')).unlink(missing_ok=True)
            if args.write:
                source=Path(c['db']);dest=source.with_name(source.name+'-paired-write')
                if dest.exists():raise RuntimeError(f'existing trial {dest}')
                subprocess.run(['cp','-a','--reflink=auto',str(source),str(dest)],check=True)
                c.update(db=str(dest),action='write',operations=1000,durable=args.durable)
            print(name,flush=True)
            run.run(c,args.out,name)
            if args.write:shutil.rmtree(dest)
