#!/usr/bin/env python3
"""Check both SQLite layouts with mmap, with contemporary LMDB each round.
Invoked by sqlite_bench select before the main matrix; no builds/tests here.
"""
import json
import math
from pathlib import Path
import statistics
import sys
import sqlite_bench as bench

out = (Path(sys.argv[1]) if len(sys.argv)>1 else bench.ROOT/'validation/sqlite/raw') / 'mmap-screen'
configs = []
for shape in ('bulk','normal'):
    for cache in (512,None):
        scenario = f'read-1000000-{shape}-{cache}-fast'
        entries = [(layout, dict(bench.base('sqlite',1000000,shape,cache),
                   sqlite_layout=layout,sqlite_mmap_bytes=1024**3)) for layout in ('rowid','clustered')]
        entries += [('lmdb',bench.base('lmdb',1000000,shape,cache))]
        for trial in range(5):
            for label,c in bench.rotate(entries,trial):
                c['db']=str(bench.fixture(c))
                bench.run(c,out/scenario,f'r{trial+1}-{label}',True)

scores = {label:[] for label in ('rowid','clustered')}
for folder in out.iterdir():
    for label in scores:
        stages={}
        for path in folder.glob('r*-'+label+'.jsonl'):
            for r in map(json.loads,path.read_text().splitlines()):
                if r['type']=='stage':
                    stages.setdefault(r['workload'],[]).append(r['call_seconds']/r['latency']['count'])
        assert all(len(v)==5 for v in stages.values()) and stages
        scores[label].extend(statistics.median(v) for v in stages.values())
scores={label:math.exp(statistics.mean(map(math.log,v))) for label,v in scores.items()}
result=dict(read_mmap=min(scores,key=scores.get),scores_seconds=scores)
(out/'selection.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
