#!/usr/bin/env python3
"""Report verified trials, paired ratios, medians and ranges; never fold invalid trials in."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import statistics

p=argparse.ArgumentParser()
p.add_argument('roots', type=Path, nargs='+')
p.add_argument('--output', type=Path)
a=p.parse_args()
rows=[]; invalid=[]; pairs={}
for root in a.roots:
    metadata=json.loads((root/'metadata.json').read_text())
    for run in json.loads((root/'runs.json').read_text()):
        if not run['valid']:
            invalid.append({'campaign':str(root),**run}); continue
        summary=json.loads((root/run['label']/'summary.json').read_text())
        config=json.loads((root/run['label']/'config.json').read_text())
        metrics=summary['metrics']
        if config['duration_s']==0 and metrics['operations']!=config['operations']:
            raise SystemExit('incorrect completed operation budget: '+run['label'])
        if config['verify'] and summary['verification']['verified_keys'] is None:
            raise SystemExit('missing verification: '+run['label'])
        row={'campaign':str(root),'case':run['case'],'repeat':run['repeat'],'engine':run['engine'],'profile':summary['profile'],
            'total_ops_s':metrics['total_ops_s'],'durable_completion_ops_s':summary['durable_completion_ops_s'],'reads_s':metrics['read_ops_s'],'writes_s':metrics['write_ops_s'],
            'read_p99_us':None if metrics['read_latency']['p99_ns'] is None else metrics['read_latency']['p99_ns']/1000,
            'write_p99_us':None if metrics['write_latency']['p99_ns'] is None else metrics['write_latency']['p99_ns']/1000,
            'read_p999_us':None if metrics['read_latency']['p999_ns'] is None else metrics['read_latency']['p999_ns']/1000,
            'write_p999_us':None if metrics['write_latency']['p999_ns'] is None else metrics['write_latency']['p999_ns']/1000,
            'read_max_us':None if metrics['read_latency']['samples']==0 else metrics['read_latency']['max_ns']/1000,
            'write_max_us':None if metrics['write_latency']['samples']==0 else metrics['write_latency']['max_ns']/1000,
            'max_l0':summary['max_l0'],'rss_bytes':summary['peak_sampled_rss'],'fds':summary['peak_sampled_fds'],'db_bytes':summary['db_bytes'],
            'sst_wa':summary['amplification']['sst_write_amp'],'process_wa':summary['amplification']['process_write_amp'],
            'final_sync_flush_drain_s':summary['final_sync_flush_drain_s'],'close_s':summary['close_s'],'settled':summary['maintenance_settled'],
            'operations':metrics['operations'],'reads':metrics['reads'],'writes':metrics['writes'],'misses':metrics['misses'],'verified_keys':summary['verification']['verified_keys']}
        rows.append(row)
        pairs.setdefault((str(root),run['case'],run['repeat']),{})[run['engine']]=(row,config)
paired=[]
keys=['profile','workload','keys','working_set','value_size','operations','read_percent','threads','seed','duration_s','sample_ms','sync_every_per_thread','preload_keys','cache','matched_knobs','rocks_statistics']
for (campaign,case,repeat),engines in pairs.items():
    if set(engines)!={'kiban','rocksdb'}:continue
    (k,kc),(r,rc)=engines['kiban'],engines['rocksdb']
    if any(kc[x]!=rc[x] for x in keys):raise SystemExit('unmatched parameters: '+case)
    if kc['duration_s']==0 and (k['reads'],k['writes'],k['misses'])!=(r['reads'],r['writes'],r['misses']):raise SystemExit('different logical operation results: '+case)
    paired.append({'campaign':campaign,'case':case,'repeat':repeat,'kiban_over_rocksdb_throughput':k['total_ops_s']/r['total_ops_s']})
groups={}
for row in rows:groups.setdefault((row['campaign'],row['case'],row['engine']),[]).append(row)
aggregates=[]
for (campaign,case,engine),trials in sorted(groups.items()):
    result={'campaign':campaign,'case':case,'engine':engine,'trials':len(trials)}
    for key in ['total_ops_s','durable_completion_ops_s','read_p99_us','write_p99_us','read_p999_us','write_p999_us','read_max_us','write_max_us','rss_bytes','db_bytes','sst_wa','process_wa','final_sync_flush_drain_s','close_s']:
        values=[r[key] for r in trials if r[key] is not None]
        result[key]=None if not values else {'median':statistics.median(values),'min':min(values),'max':max(values)}
    aggregates.append(result)
out=a.output or a.roots[0]
out.mkdir(parents=True,exist_ok=True)
with (out/'comparison.csv').open('w',newline='') as f:
    writer=csv.DictWriter(f,fieldnames=list(rows[0]) if rows else ['campaign']);writer.writeheader();writer.writerows(rows)
(out/'comparison.json').write_text(json.dumps({'analyzer_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'trials':rows,'aggregates':aggregates,'paired_ratios':paired,'invalid':invalid,'policy':'only complete verified trials; median per-trial percentiles, not pooled percentile; unknown metrics remain null'},indent=2)+'\n')
lines=['| Case | Engine | Trials | Median ops/s (min–max) | GET p99 µs | PUT p99 µs |','| --- | --- | ---: | ---: | ---: | ---: |']
for g in aggregates:
    n=g['total_ops_s'];read=g['read_p99_us'];write=g['write_p99_us']
    rd='' if read is None else f"{read['median']:.2f}"
    wr='' if write is None else f"{write['median']:.2f}"
    lines.append(f"| {g['case']} | {g['engine']} | {g['trials']} | {n['median']:,.0f} ({n['min']:,.0f}–{n['max']:,.0f}) | {rd} | {wr} |")
(out/'comparison.md').write_text('\n'.join(lines)+'\n')
print('\n'.join(lines))
print(f'Valid trials: {len(rows)}; complete pairs: {len(paired)}; invalid trials excluded: {len(invalid)}')
