#!/usr/bin/env python3
"""Build frozen engines from archive/final, and one identical standalone range probe."""
import hashlib, json, os, shutil, subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'target/v2-final-bin'
OUT.mkdir(exist_ok=True)
cpus='2,3,4,5,6,7,10,11'
provenance={}
for label,source in [('before',ROOT/'target/v2-main-8bbb4934'),('after',ROOT)]:
    manifest=source/'tools/agentbench/readpath/Cargo.toml'
    cmd=['taskset','-c',cpus,'nice','-n','10','cargo','build','--locked','--release','-j','8','--message-format=json','--manifest-path',str(manifest)]
    with (OUT/(label+'-build.log')).open('w') as log:
        result=subprocess.run(cmd,cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=log,check=True)
    artifacts=[json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
    (OUT/(label+'-artifacts.json')).write_text(json.dumps(artifacts,indent=2))
    exe=next(r['executable'] for r in artifacts if r.get('executable') and r['target']['name']=='kiban-readbench')
    shutil.copy2(exe,OUT/label)
    libs={}
    for name in ['kiban','serde','serde_json','mimalloc']:
        libs[name]=next(f for r in artifacts if r.get('reason')=='compiler-artifact' and r['target']['name']==name for f in r['filenames'] if f.endswith('.rlib'))
    cmd=['taskset','-c',cpus,'nice','-n','10','rustc','--edition=2024','-Copt-level=2','-Clto','-Ccodegen-units=1','-Cpanic=abort','-Adead_code',str(ROOT/'tools/agentbench/readpath/range.rs'),'-Ldependency='+str(Path(libs['kiban']).parent),'-o',str(OUT/(label+'-range'))]
    for name,path in libs.items():cmd+=['--extern',name+'='+path]
    subprocess.run(cmd,cwd=ROOT,check=True)
    provenance[label]=dict(source=str(source),binaries={name:hashlib.sha256((OUT/name).read_bytes()).hexdigest() for name in [label,label+'-range']},build_cpus=cpus,nice=10)
(OUT/'provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
(ROOT/'validation/v2/baseline.json').write_text(json.dumps(dict(commit='8bbb4934f8d102ed0285c813a3770831b21c63e5',source_archive='git archive 8bbb4934 into target/v2-main-8bbb4934',binary=str(OUT/'before'),sha256=provenance['before']['binaries']['before'],build_cpus=cpus,priority='nice 10'),indent=2)+'\n')
