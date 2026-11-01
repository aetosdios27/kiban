#!/usr/bin/env python3
"""Audit superseded evidence separately from the final comparison."""
import hashlib
import json
from pathlib import Path
import sys

folder=Path(sys.argv[1]) if len(sys.argv)>1 else Path(__file__).resolve().parents[3]/'validation/rocksdb/archive-convenience-api'
meta=json.loads((folder/'raw/metadata.json').read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
for p,h in meta['harness_sources'].items():assert sha(folder/'sources'/p)==h,p
commands=[]
for phase in ('screen','measure'):
 for receipts in sorted((folder/'raw'/phase).glob('*/commands.jsonl')):
  for command in map(json.loads,receipts.read_text().splitlines()):
   path=receipts.parent/(command['name']+'.jsonl')
   assert sha(path)==command['result_sha256']
   assert sha(receipts.parent/command['config'])==command['config_sha256']
   assert sha(receipts.parent/(command['name']+'.log'))==command['log_sha256']
   rows=list(map(json.loads,path.read_text().splitlines()))
   assert all(r.get('errors',0)==0 for r in rows)
   assert any(r['type']=='complete' for r in rows)
   status=next(r['proc_status'] for r in rows if 'proc_status' in r)
   assert next(s.split(':',1)[1].strip() for s in status.splitlines() if s.startswith('Cpus_allowed_list:'))=='10'
   commands.append(dict(phase=phase,timed_calls=sum(r['latency']['count'] for r in rows if r['type']=='stage')))
result=dict(accepted_commands=len(commands),screens=sum(c['phase']=='screen' for c in commands),primary=sum(c['phase']=='measure' for c in commands),timed_calls=sum(c['timed_calls'] for c in commands),errors=0,all_affinities='10',excluded_from_final=True,reason='Superseded convenience point API; fresh full campaign uses reusable ReadOptions')
assert result['screens']==270 and result['primary']==61
(folder/'audit.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
