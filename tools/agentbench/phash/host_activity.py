#!/usr/bin/env python3
"""Read-only diagnostic for unrelated sibling activity; run on driver CPUs."""
import json, os, time
from pathlib import Path

def snap():
    out={}
    for p in Path('/proc').iterdir():
        if not p.name.isdecimal():continue
        try:
            for t in (p/'task').iterdir():
                raw=(t/'stat').read_text();v=raw[raw.rindex(')')+2:].split()
                out[int(t.name)]=(int(v[11])+int(v[12]),int(v[36]),raw[raw.index('(')+1:raw.rindex(')')],int(p.name))
        except (PermissionError,FileNotFoundError,ProcessLookupError):pass
    return out

if __name__=='__main__':
    assert not (set(os.sched_getaffinity(0)) & {8,9})
    start=time.time();a=snap();time.sleep(1);b=snap();rows=[]
    for tid,(ticks,cpu,name,pid) in b.items():
        delta=ticks-a.get(tid,(ticks,))[0]
        if cpu==9 and delta:
            try:affinity=sorted(os.sched_getaffinity(tid))
            except ProcessLookupError:continue
            rows.append(dict(pid=pid,tid=tid,ticks=delta,cpu=cpu,name=name,affinity=affinity))
    print(json.dumps(dict(time=start,interval_s=time.time()-start,clock_ticks_s=os.sysconf('SC_CLK_TCK'),note='Last-CPU sampling can miss migrated work; these are examples, not complete CPU9 attribution.',threads=sorted(rows,key=lambda r:r['ticks'],reverse=True)),indent=2))
