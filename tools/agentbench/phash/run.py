#!/usr/bin/env python3
"""CPU-8-only experiment. Build separately before invoking; never starts a build."""
import argparse, hashlib, json, os, subprocess, tempfile, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT/'tools/agentbench/phash/target/release/kiban-phash'
SOURCE = Path('/home/aetos/.t3/worktrees/kiban/t3-53ad0f47/target/read-path-data')
PERF = SOURCE.parent/'read-path-tools/root/usr/bin/perf'
EVENTS = ','.join('cpu_core/'+x+'/u' for x in ['cycles','instructions','LLC-load-misses','dTLB-load-misses'])

def snapshot():
    return dict(time=time.time(), load=os.getloadavg(), cpu_stat=Path('/proc/stat').read_text(),
                cpu8_frequency=Path('/sys/devices/system/cpu/cpu8/cpufreq/scaling_cur_freq').read_text().strip())

def checksums(path):
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in path.iterdir() if p.is_file() and p.name != 'LOCK'}

def main():
    p=argparse.ArgumentParser(); p.add_argument('--out',type=Path,default=ROOT/'validation/phash/results'); p.add_argument('--trials',type=int,default=5); p.add_argument('--ops',type=int,default=100000); p.add_argument('--smoke',action='store_true'); a=p.parse_args()
    a.out.mkdir(parents=True,exist_ok=True)
    metadata=dict(binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),cpu=8,build_cpu='6,7,10,11',seed=20261016,operations=a.ops,trials=a.trials,start=snapshot(),cpuinfo=Path('/proc/cpuinfo').read_text(),governor=Path('/sys/devices/system/cpu/cpu8/cpufreq/scaling_governor').read_text(),epp=Path('/sys/devices/system/cpu/cpu8/cpufreq/energy_performance_preference').read_text())
    (a.out/'metadata.json').write_text(json.dumps(metadata,indent=2))
    with (a.out/'floor.jsonl').open('w') as f: subprocess.run(['taskset','-c','8',str(BINARY),'floor',str(200000)],check=True,stdout=f)
    for layout in ['bulk','normal']:
        kpath=ROOT/'target/phash-data'/f'kiban-1000000-{layout}'
        before=checksums(kpath)
        for trial in range(1,a.trials+1):
            # Reverse the full alternating sequence so LMDB precedes and follows each candidate.
            variants=['lmdb','kiban','lmdb','mphf','lmdb']
            if trial%2==0: variants.reverse()
            for wi,workload in enumerate(['hit','miss']):
                for vi,variant in enumerate(variants):
                    title=f'{layout}-r{trial}-{workload}-{vi}-{variant}'
                    if (a.out/(title+'.jsonl')).exists(): raise RuntimeError('refusing to overwrite '+title)
                    db=ROOT/'target/phash-data'/f'lmdb-1000000-{layout}' if variant=='lmdb' else kpath
                    args=['taskset','-c','8',str(BINARY),variant,str(db),workload,str(a.ops)]
                    print(title,flush=True)
                    started=snapshot()
                    with tempfile.TemporaryDirectory(prefix='phash-perf-') as tmp:
                        ctl,ack=Path(tmp)/'ctl',Path(tmp)/'ack'; os.mkfifo(ctl); os.mkfifo(ack)
                        env={**os.environ,'RAYON_NUM_THREADS':'1','READBENCH_PERF_CTL':str(ctl),'READBENCH_PERF_ACK':str(ack)}
                        args=['taskset','-c','8',str(PERF),'stat','-x',';','-e',EVENTS,'-o',str(a.out/(title+'.stat')),'--control',f'fifo:{ctl},{ack}','-D','-1','--',*args]
                        with (a.out/(title+'.jsonl')).open('w') as out, (a.out/(title+'.stderr')).open('w') as err:
                            subprocess.run(args,check=True,env=env,stdout=out,stderr=err)
                    with (a.out/'commands.jsonl').open('a') as f: f.write(json.dumps(dict(title=title,args=args,started=started,ended=snapshot()))+'\n')
                    if a.smoke: break
                if a.smoke: break
            if a.smoke: break
        after=checksums(kpath)
        (a.out/(layout+'-kiban-checksums.json')).write_text(json.dumps(dict(before=before,after=after,unchanged=before==after),indent=2))
        assert before==after,'Kiban read changed files'
        if a.smoke: break
    metadata['end']=snapshot(); (a.out/'metadata.json').write_text(json.dumps(metadata,indent=2))
if __name__=='__main__': main()
