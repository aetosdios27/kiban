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
parser.add_argument('--cache',type=int,default=512)
parser.add_argument('--trials',type=int,default=5)
parser.add_argument('--write',action='store_true')
parser.add_argument('--durable',action='store_true')
args=parser.parse_args()
binaries={'before':args.before.resolve(),'after':args.after.resolve()}
args.out.mkdir(parents=True,exist_ok=True)
(args.out/'metadata.json').write_text(json.dumps(dict(binaries={k:dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for k,p in binaries.items()},trials=args.trials),indent=2)+'\n')
for trial in range(args.trials):
    for label in ('before','after') if trial%2==0 else ('after','before'):
        run.BINARY=binaries[label]
        for engine in ('kiban','lmdb'):
            c=run.config(engine,args.n,args.layout,args.cache)
            if args.write:
                source=Path(c['db']);dest=source.with_name(source.name+'-paired-write')
                if dest.exists():raise RuntimeError(f'existing trial {dest}')
                subprocess.run(['cp','-a','--reflink=auto',str(source),str(dest)],check=True)
                c.update(db=str(dest),action='write',operations=1000,durable=args.durable)
            name=f'{args.n}-{args.layout}-{args.cache}-r{trial+1}-{label}-{engine}'
            print(name,flush=True)
            run.run(c,args.out,name)
            if args.write:shutil.rmtree(dest)
