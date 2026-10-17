#!/usr/bin/env python3
"""Compare private copies to read-only sources after all timed work finishes."""
import hashlib, json, os, subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
SOURCE=Path('/home/aetos/.t3/worktrees/kiban/t3-53ad0f47/target/read-path-data')
assert not (set(os.sched_getaffinity(0)) & {8,9}), 'run only on driver CPUs'

def digest(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        while chunk:=f.read(8*1024*1024):h.update(chunk)
    return h.hexdigest()

rows=[]
for layout in ['bulk','normal']:
    for engine in ['kiban','lmdb']:
        folder=f'{engine}-1000000-{layout}'
        source,private=SOURCE/folder,ROOT/'target/phash-data'/folder
        names=sorted(p.name for p in source.iterdir() if p.is_file() and p.name not in ['LOCK','lock.mdb'])
        for name in names:
            a,b=source/name,private/name
            ah,bh=digest(a),digest(b)
            sa,sb=a.stat(),b.stat()
            assert (sa.st_dev,sa.st_ino)!=(sb.st_dev,sb.st_ino), 'hard link'
            assert ah==bh,(a,b)
            rows.append(dict(dataset=folder,file=name,source_sha256=ah,private_sha256=bh,bytes=sa.st_size,source_inode=sa.st_ino,private_inode=sb.st_ino,source_device=sa.st_dev,private_device=sb.st_dev))
binary=ROOT/'tools/agentbench/phash/target/release/kiban-phash'
sha=digest(binary)
assert sha==json.loads((ROOT/'validation/phash/results/metadata.json').read_text())['binary_sha256']
assert not subprocess.check_output(['git','diff','HEAD','--','src/'],cwd=ROOT)
files=['tools/agentbench/phash/src/main.rs','tools/agentbench/phash/Cargo.toml','tools/agentbench/phash/Cargo.lock','tools/agentbench/src/model.rs','examples/support/random.rs']
out=dict(binary_sha256=sha,source_files={f:digest(ROOT/f) for f in files},all_data_equal=True,all_inodes_distinct=True,engine_src_diff_empty=True,files=rows)
(ROOT/'validation/phash/provenance.json').write_text(json.dumps(out,indent=2)+'\n')
print('verified',len(rows),'source/private files; binary',sha)
