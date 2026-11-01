#!/usr/bin/env python3
"""Serial measurement stages, guarded by completed exact-source correctness gates."""
import argparse,hashlib,json,os,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'validation/v2'
p=argparse.ArgumentParser();p.add_argument('--before',type=Path,required=True);p.add_argument('--after',type=Path,required=True);p.add_argument('--test-binary',type=Path,required=True);a=p.parse_args()
checks=json.loads((OUT/'final-checks/status.json').read_text())
assert len(checks)==11 and all(r['code']==0 for r in checks)
assert not json.loads((OUT/'final-checks/metadata.json').read_text())['quick']
hashes=json.loads((OUT/'final-checks/source-hashes.json').read_text())
assert all(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest for name,digest in hashes.items())
assert hashlib.sha256(a.before.read_bytes()).hexdigest()==json.loads((OUT/'baseline.json').read_text())['sha256']
assert a.after.is_file() and a.test_binary.is_file()
assert Path('/home/aetos/.t3/worktrees/kiban/t3-53ad0f47/target/read-path-tools/root/usr/bin/perf').is_file()
(OUT/'measurement-provenance.json').write_text(json.dumps(dict(engine_sources=hashes,binaries={label:dict(path=str(path.resolve()),sha256=hashlib.sha256(path.read_bytes()).hexdigest()) for label,path in [('before',a.before),('after',a.after),('maintenance',a.test_binary)]},harness_sources={str(path.relative_to(ROOT)):hashlib.sha256(path.read_bytes()).hexdigest() for path in [*(Path(__file__).parent/'src').rglob('*.rs'),*Path(__file__).parent.glob('*.py'),*[ROOT/'tools/agentbench/src'/name for name in ['embedded.rs','runner.rs','model.rs','api.rs']],ROOT/'examples/scale/metrics.rs',ROOT/'examples/support/random.rs']},cpu=8,driver_cpus='6,7,10,11',nice=os.getpriority(os.PRIO_PROCESS,0),start=time.time()),indent=2)+'\n')
rows=[]
def stage(name,args,env_changes=None):
 print('START '+name,flush=True);start=time.time()
 with (OUT/(name+'-progress.log')).open('w') as out:
  result=subprocess.run(args,cwd=ROOT,stdout=out,stderr=out,env={**os.environ,**(env_changes or {})})
 rows.append(dict(stage=name,args=args,start=start,end=time.time(),code=result.returncode,env_changes=env_changes or {}))
 (OUT/'measurement-status.json').write_text(json.dumps(rows,indent=2)+'\n')
 if result.returncode:
  print((OUT/(name+'-progress.log')).read_text()[-5000:],flush=True)
  raise SystemExit(result.returncode)
 print('DONE '+name,flush=True)
common=['--before',str(a.before.resolve()),'--after',str(a.after.resolve())]
for action in ['prepare','read','write']:
 stage(action,['python3',str(ROOT/'tools/agentbench/readpath/v2.py'),action,*common])
stage('profile',['python3',str(ROOT/'tools/agentbench/readpath/v2_profile.py'),*common])
(ROOT/'target/v2-maintenance').mkdir(exist_ok=True)
stage('maintenance',['taskset','-c','8',str(a.test_binary.resolve()),'point_index_maintenance_measurement','--ignored','--nocapture','--test-threads=1'],{'TMPDIR':str(ROOT/'target/v2-maintenance')})
stage('analysis',['python3',str(ROOT/'tools/agentbench/readpath/v2_report.py')])

assert all(hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest for name,digest in hashes.items()), "engine sources changed during measurement"
