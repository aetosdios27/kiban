#!/usr/bin/env python3
"""Five interleaved trials; no compiler; private input copies; CPU 8 only."""
import hashlib, json, os, subprocess, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT/'tools/agentbench/phash/target/release/kiban-phash'
OUT = ROOT/'validation/v2/ablation'
OUT.mkdir(parents=True, exist_ok=True)
metadata = dict(binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(), cpu=8,
                load=os.getloadavg(), start=time.time(), operations=100000, trials=5)
(OUT/'metadata.json').write_text(json.dumps(metadata, indent=2))
for layout in ['bulk','normal']:
    for trial in range(1,6):
        variants = [(v,thp) for thp in ['enabled','disabled'] for v in ['lmdb','kiban','sorted','mphf']]
        if trial % 2 == 0:
            variants.reverse()
        for variant, thp in variants:
            title=f'{layout}-r{trial}-{variant}-{thp}'
            output=OUT/(title+'.jsonl')
            if output.exists(): raise RuntimeError('Refusing overwrite '+title)
            engine='lmdb' if variant=='lmdb' else 'kiban'
            db=ROOT/'target/phash-data'/f'{engine}-1000000-{layout}'
            env={**os.environ, 'RAYON_NUM_THREADS':'1'}
            if thp=='disabled': env['PHASH_NO_THP']='1'
            else: env.pop('PHASH_NO_THP',None)
            args=['taskset','-c','8',str(BINARY),variant,str(db),'hit','100000']
            print(title,flush=True)
            start=time.time()
            with output.open('w') as out, (OUT/(title+'.stderr')).open('w') as err:
                subprocess.run(args,env=env,stdout=out,stderr=err,check=True)
            with (OUT/'commands.jsonl').open('a') as f:
                f.write(json.dumps(dict(title=title,args=args,start=start,end=time.time(),load=os.getloadavg()))+'\n')
metadata['end']=time.time()
(OUT/'metadata.json').write_text(json.dumps(metadata, indent=2))
