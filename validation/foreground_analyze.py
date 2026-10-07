#!/usr/bin/env python3
"""Summarize all complete paired trials; never pool trial percentiles."""
import argparse
import csv
import json
from pathlib import Path
import statistics

p = argparse.ArgumentParser()
p.add_argument('root', type=Path)
a = p.parse_args()
metadata = json.loads((a.root/'metadata.json').read_text())
runs = json.loads((a.root/'runs.json').read_text())
expected = len(metadata['plan']['cases'])*metadata['plan']['repetitions']*len(metadata['builds'])
if len(runs) != expected or not all(r['valid'] for r in runs):
    raise SystemExit('incomplete or invalid campaign; cannot summarize as success')
rows = []
for run in runs:
    root = a.root/run['directory']
    summaries = list(csv.DictReader((root/'summary.csv').open()))
    total = next(r for r in summaries if r['stage']=='total')
    samples = list(csv.DictReader((root/'samples.csv').open()))
    if any(None in row or None in row.values() for row in samples):
        raise SystemExit('malformed sample CSV')
    last = samples[-1]
    row = {'label':run['label'],'case':run['case'],'repeat':run['repeat'],
           'revision':metadata['builds'][run['label']]['receipt']['revision'],
           'operations':int(total['operations']),'ops_s':float(total['total_ops_s']),
           'read_ops_s':float(total['read_ops_s']),'write_ops_s':float(total['write_ops_s']),
           'settled':last['maintenance_running']=='false' and last['pending_flush']=='false' and int(last['debt_bytes'])==0,
           'peak_rss_bytes':max(int(x['rss_bytes']) for x in samples if x['rss_bytes']),
           'peak_fds':max(int(x['fds']) for x in samples if x['fds']),
           'max_l0':max(int(x['l0_files']) for x in samples),
           'db_bytes':int(last['db_bytes'])}
    for kind in ['read','write']:
        for percentile in ['p50','p95','p99','p999','max']:
            name = f'{kind}_{percentile}_ns'
            row[name] = float(total[name]) if total[name] else None
    for name in ['sst_write_amp','stall_ns','immutable_wait_ns','l0_wait_ns','both_wait_ns',
                 'flush_queue_max_ns','compaction_build_ns','compaction_commit_wait_ns','compaction_commit_ns',
                 'flush_build_ns','flush_commit_wait_ns','flush_commit_ns','process_read_bytes','process_write_bytes']:
        row[name] = float(last[name]) if last[name] else None
    rows.append(row)
groups = {}
for row in rows:
    groups.setdefault((row['case'],row['label']), []).append(row)
aggregates = []
for (case,label),trials in sorted(groups.items()):
    result = {'case':case,'label':label,'trials':len(trials),'all_settled':all(r['settled'] for r in trials)}
    for name in rows[0]:
        if name in ['label','case','repeat','revision','settled']: continue
        values = [r[name] for r in trials if r[name] is not None]
        result[name] = {'median':statistics.median(values),'min':min(values),'max':max(values)} if values else None
    aggregates.append(result)
report = {'policy':'every complete verified trial; median trial percentiles, not pooled',
          'profiling':metadata['profiling'],'trials':rows,'aggregates':aggregates}
(a.root/'analysis.json').write_text(json.dumps(report,indent=2)+'\n')
for g in aggregates:
    print(g['case'],g['label'],f"{g['ops_s']['median']:,.0f} ops/s", 'settled',g['all_settled'])
print(f'{len(rows)} verified trials; {sum(r["operations"] for r in rows):,} operations')
