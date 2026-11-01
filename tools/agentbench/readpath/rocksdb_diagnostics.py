#!/usr/bin/env python3
"""Audit and tabulate completed sync probes and scoped counter loops."""
import json
from pathlib import Path
import re
import sys

out = Path(sys.argv[1]) if len(sys.argv)>1 else Path(__file__).resolve().parents[3]/'validation/rocksdb'
records=[]
for path in sorted((out/'raw/sync-probe').glob('*.log')):
    counts = [{k:int(v) for k,v in zip(('fsync','fdatasync','failures'),m)} for m in re.findall(r'sync_probe fsync=(\d+) fdatasync=(\d+) failures=(\d+)',path.read_text())]
    assert counts, path
    # /usr/bin/time also inherits LD_PRELOAD and prints a zero-count footer.
    count=max(counts,key=lambda c:c['fsync']+c['fdatasync'])
    assert count['failures']==0
    rows=list(map(json.loads,path.with_suffix('.jsonl').read_text().splitlines()))
    assert any(r['type']=='reopen_verified' for r in rows)
    records.append(dict(label=path.stem,**count))
assert len(records)==7
by={r['label']:r for r in records}
for backend in ('kiban','lmdb','rocksdb'):
    fast=by[backend+'-fast'];durable=by[backend+'-durable']
    assert durable['fsync']+durable['fdatasync']-fast['fsync']-fast['fdatasync']==2100
(out/'sync-probe-summary.json').write_text(json.dumps(records,indent=2)+'\n')
lines=['| Whole-process diagnostic | fsync | fdatasync | Failed calls |','|---|---:|---:|---:|']
lines += [f'| {r["label"]} | {r["fsync"]} | {r["fdatasync"]} | {r["failures"]} |' for r in records]
(out/'SYNC.md').write_text('\n'.join(lines)+'\n')

profiles=[]
for path in sorted((out/'profile').glob('*.stat')):
    per_op={}
    for line in path.read_text().splitlines():
        if not line or line.startswith('#'):continue
        parts=line.split(';')
        if len(parts)>4:
            assert 'not counted' not in parts[0] and 'not supported' not in parts[0],line
            if parts[0].strip().isdigit():
                assert float(parts[4].strip().rstrip('%'))>=99.9, 'multiplexed counter: '+line
                per_op[parts[2]]=int(parts[0])/1000000
    assert len(per_op)==5,(path,per_op)
    log=path.with_suffix('.log').read_text()
    row=next(json.loads(s) for s in log.splitlines() if s.startswith('{') and '"type":"profile"' in s)
    assert row['operations']==1000000 and row['errors']==0
    profiles.append(dict(name=path.stem,per_op=per_op))
assert len(profiles)==10
(out/'profile/summary.json').write_text(json.dumps(profiles,indent=2)+'\n')
lines=['| API row | User cycles/op | Instructions/op | Branch misses/op | LLC misses/op | dTLB misses/op |','|---|---:|---:|---:|---:|---:|']
for r in profiles:
    values=[next(v for k,v in r['per_op'].items() if event in k) for event in ('cycles','instructions','branch-misses','LLC-load-misses','dTLB-load-misses')]
    lines.append('| '+r['name']+' | '+' | '.join(f'{v:.2f}' for v in values)+' |')
(out/'PROFILE.md').write_text('\n'.join(lines)+'\n')
print(json.dumps(records,indent=2))
