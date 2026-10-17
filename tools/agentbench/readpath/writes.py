#!/usr/bin/env python3
"""Run W1/W4 from fresh copies; alternate Kiban and LMDB on the same core."""
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

import run

parser = argparse.ArgumentParser()
parser.add_argument('--out', required=True, type=Path)
parser.add_argument('--binary', type=Path, default=run.BINARY)
parser.add_argument('--trials', type=int, default=5)
parser.add_argument('--populations', type=int, nargs='+', default=[10000])
parser.add_argument('--layouts', nargs='+', choices=['bulk','normal'], default=['bulk'])
args = parser.parse_args()
run.BINARY = args.binary.resolve()
args.out.mkdir(parents=True, exist_ok=True)
(args.out/'metadata.json').write_text(json.dumps(dict(binary=str(run.BINARY),sha256=hashlib.sha256(run.BINARY.read_bytes()).hexdigest(),trials=args.trials),indent=2)+'\n')
for n in args.populations:
    for layout in args.layouts:
        for durable in (False,True):
            for trial in range(args.trials):
                for engine in ('kiban','lmdb') if trial % 2 == 0 else ('lmdb','kiban'):
                    c = run.config(engine,n,layout)
                    source=Path(c['db'])
                    dest=source.with_name(source.name+'-write-trial')
                    if dest.exists(): raise RuntimeError(f'existing write trial: {dest}')
                    # Reflink when the filesystem supports it, ordinary copy otherwise.
                    subprocess.run(['cp','-a','--reflink=auto',str(source),str(dest)],check=True)
                    c.update(db=str(dest),action='write',durable=durable,operations=1000)
                    name=f'{n}-{layout}-{"durable" if durable else "fast"}-r{trial+1}-{engine}'
                    print(name,flush=True)
                    run.run(c,args.out,name)
                    shutil.rmtree(dest)
