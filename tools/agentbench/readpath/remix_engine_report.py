#!/usr/bin/env python3
"""Summarize verified paired runs without hiding failed gates or trial spread."""
from collections import defaultdict
import json
import os
from pathlib import Path
import statistics as st

ROOT=Path(__file__).resolve().parents[3]
OUT=Path(os.environ.get('REMIX_ENGINE_OUT', ROOT/'validation/remix-engine'))
summary=[]
for action in ['pilot','reads','varied','writes']:
    for folder in sorted((OUT/action).glob('*')):
        data=defaultdict(dict)
        for path in sorted(folder.glob('*.jsonl')):
            if path.name in ['commands.jsonl','sibling.jsonl']:continue
            name=path.stem.split('-')
            trial,label,engine=name[-3:]
            for line in path.read_text().splitlines():
                row=json.loads(line)
                if row['type']=='stage':
                    assert row['errors']==0
                    data[row['workload']][trial,label,engine]=row['latency']
        for workload,rows in data.items():
            result=dict(action=action,scenario=folder.name,workload=workload)
            trials=sorted(t for t in set(trial for trial,_,_ in rows) if all((t,label,engine) in rows for label in ['before','after'] for engine in ['kiban','lmdb']))
            if not trials:continue
            result['trials']=len(trials)
            for label in ['before','after']:
                result[label]={}
                for field in ['p50_ns','p99_ns']:
                    k=[rows[t,label,'kiban'][field] for t in trials]
                    lm=[rows[t,label,'lmdb'][field] for t in trials]
                    ratios=[a/b for a,b in zip(k,lm)]
                    result[label][field]=dict(kiban=st.median(k),lmdb=st.median(lm),
                        kiban_range=[min(k),max(k)],lmdb_range=[min(lm),max(lm)],
                        paired=st.median(ratios),paired_range=[min(ratios),max(ratios)],
                        ratio_of_medians=st.median(k)/st.median(lm),ratios=ratios)
            result['normalized_after_before']={field:dict(
                median=st.median((rows[t,'after','kiban'][field]/rows[t,'after','lmdb'][field])/(rows[t,'before','kiban'][field]/rows[t,'before','lmdb'][field]) for t in trials),
                ratios=[(rows[t,'after','kiban'][field]/rows[t,'after','lmdb'][field])/(rows[t,'before','kiban'][field]/rows[t,'before','lmdb'][field]) for t in trials]) for field in ['p50_ns','p99_ns']}
            summary.append(result)
(OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
for action in ['pilot','reads','varied','writes']:
    lines=['| Scenario | Workload | Trials | K p50/p99 before (us) | K p50/p99 after (us) | LMDB after (us) | Paired p50 before/after | Paired p99 before/after |',
           '|---|---|---:|---:|---:|---:|---:|---:|']
    for r in summary:
        if r['action']!=action:continue
        b,a=r['before'],r['after']
        f=lambda side,key:f"{side['p50_ns'][key]/1000:.3f}/{side['p99_ns'][key]/1000:.3f}"
        lines.append(f"| {r['scenario']} | {r['workload']} | {r['trials']} | {f(b,'kiban')} | {f(a,'kiban')} | {f(a,'lmdb')} | {b['p50_ns']['paired']:.3f}/{a['p50_ns']['paired']:.3f} | {b['p99_ns']['paired']:.3f}/{a['p99_ns']['paired']:.3f} |")
    (OUT/(action.upper()+'.md')).write_text('\n'.join(lines)+'\n')
    if len(lines)>2:print('\n'.join(lines))
