#!/usr/bin/env python3
"""Compile the same owned-range control against main and candidate libraries."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parents[3]
BIN=ROOT/'target/remix-bin'
source=(ROOT/'tools/agentbench/readpath/range.rs').read_text().replace('Cpus_allowed_list:\\t8','Cpus_allowed_list:\\t3').replace('    let cache_bytes = options.block_cache_bytes;', '    options.point_index = true;\n    let cache_bytes = options.block_cache_bytes;')
path=ROOT/'tools/agentbench/readpath/remix_range.rs'
path.write_text(source)
records={}
for label,root in [('before',ROOT/'target/remix-before-src'),('after',ROOT)]:
    cmd=['taskset','-c','12-21','nice','-n','10','cargo','build','--release','--locked','--message-format=json','--manifest-path',str(root/'tools/agentbench/readpath/Cargo.toml')]
    if label=='after':cmd+=['--features','remix']
    result=subprocess.run(cmd,stdout=subprocess.PIPE,text=True,check=True)
    artifacts=[json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
    libs={name:next(f for r in artifacts if r.get('reason')=='compiler-artifact' and r['target']['name']==name for f in r['filenames'] if f.endswith('.rlib')) for name in ['kiban','serde','serde_json','mimalloc']}
    cmd=['taskset','-c','12-21','nice','-n','10','rustc','--edition=2024','-Copt-level=2','-Clto','-Ccodegen-units=1','-Cpanic=abort','-Adead_code',str(path),'-Ldependency='+str(Path(libs['kiban']).parent),'-o',str(BIN/(label+'-range'))]
    for name,lib in libs.items():cmd+=['--extern',name+'='+lib]
    subprocess.run(cmd,check=True)
    records[label]=dict(command=cmd,sha256=hashlib.sha256((BIN/(label+'-range')).read_bytes()).hexdigest())
(Path(os.environ.get('REMIX_ENGINE_OUT', ROOT/'validation/remix-engine'))/'range-provenance.json').write_text(json.dumps(records,indent=2)+'\n')
