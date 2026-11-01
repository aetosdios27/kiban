import json, os, shutil, subprocess, sys
from pathlib import Path
V=Path.home()/'.local/share/kiban-verify'; B=V/'gc-tgt/release/group-commit-bench'; P=V/'gc-probe.so'
D=V/'gc-data'; OUT=V/'gc-results'; OUT.mkdir(exist_ok=True)
CASES=[('kiban','sync'),('kiban','group'),('sqlite','x'),('rocksdb','x')]
THREADS=[1,2,4,8,16]; TRIALS=5; TOTAL=4000
env={**os.environ,'LD_PRELOAD':str(P)}
for eng in {c[0] for c in CASES}:
    d=D/f'base-{eng}'
    if not d.exists():
        subprocess.run(['taskset','-c','0,1,2,5',str(B),'prepare',eng,str(d),'10000'],check=True,stdout=subprocess.DEVNULL)
for t in range(1,TRIALS+1):
    for th in THREADS:
        order=CASES if (t+th)%2 else CASES[::-1]
        for eng,mode in order:
            name=f'{eng}-{mode}-t{th}-r{t}'; out=OUT/(name+'.json')
            if out.exists(): continue
            run=D/'run'; shutil.rmtree(run,ignore_errors=True); shutil.copytree(D/f'base-{eng}',run)
            subprocess.run(['sync']); 
            subprocess.run(['taskset','-c','0,1,2,5',str(B),'run',eng,mode,str(th),str(max(250,TOTAL//th)),str(run),str(out),'10000'],env=env,check=True,stdout=subprocess.DEVNULL)
            print(name,flush=True)
print('ALLDONE')
