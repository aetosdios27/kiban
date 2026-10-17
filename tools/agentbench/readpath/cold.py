#!/usr/bin/env python3
"""Alternate file-specific cold-start probes without a global cache drop."""
import argparse
import hashlib
import json
from pathlib import Path
import run

parser = argparse.ArgumentParser()
parser.add_argument('--binary', required=True, type=Path)
parser.add_argument('--out', required=True, type=Path)
args = parser.parse_args()
run.BINARY = args.binary.resolve()
args.out.mkdir(parents=True, exist_ok=True)
assert not (args.out/'commands.jsonl').exists(), 'use a fresh folder'
(args.out/'metadata.json').write_text(json.dumps(dict(binary=str(run.BINARY),sha256=hashlib.sha256(run.BINARY.read_bytes()).hexdigest(),affinity='3',trials=5,operations=10000,cache_mib=32,cold='post-open madvise DONTNEED on this process dataset mappings, then file-specific posix_fadvise DONTNEED before timing'),indent=2)+'\n')
for readers in (1,4):
    for trial in range(1,6):
        cases=('pread','mapped','lmdb') if trial%2 else ('lmdb','mapped','pread')
        for case in cases:
            c=run.config('lmdb' if case=='lmdb' else 'kiban',1000000,'normal',32)
            c.update(action='cold',operations=10000,readers=readers,mmap_max_level=4294967295 if case=='mapped' else None)
            name=f'cold-{readers}-r{trial}-{case}'
            print(name,flush=True)
            run.run(c,args.out,name)
