#!/usr/bin/env python3
"""Compare complete verified maintenance campaigns with identical fixed plans."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import statistics

p = argparse.ArgumentParser()
p.add_argument('roots', type=Path, nargs='+')
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
rows = []
plan = None
timings = ['immutable_wait_ns','l0_wait_ns','both_wait_ns','flush_queue_ns',
           'flush_queue_max_ns','flush_build_ns','flush_commit_wait_ns','flush_commit_ns',
           'compaction_build_ns','compaction_commit_wait_ns','compaction_commit_ns']
for root in a.roots:
    metadata = json.loads((root/'metadata.json').read_text())
    if plan is not None and metadata['plan'] != plan:
        raise SystemExit('workload/configuration plans differ')
    plan = metadata['plan']
    runs = json.loads((root/'runs.json').read_text())
    if len(runs) != len(plan['cases'])*plan['repetitions'] or not all(r['valid'] for r in runs):
        raise SystemExit(f'incomplete/failed campaign: {root}')
    cases = {c['name']:c for c in plan['cases']}
    for run in runs:
        directory = root/run['label']
        with (directory/'summary.csv').open() as f:
            summary = next(r for r in csv.DictReader(f) if r['stage']=='total')
        with (directory/'samples.csv').open() as f:
            samples = list(csv.DictReader(f))
        if any(None in r or None in r.values() for r in samples):
            raise SystemExit('malformed sample CSV')
        last = samples[-1]
        case,repeat = run['label'].rsplit('-r',1)
        if int(summary['operations']) != cases[case]['args']['operations']:
            raise SystemExit('wrong operation count')
        if sum(int(last[k]) for k in ['immutable_stalls','l0_stalls','both_stalls']) != int(last['stalls']):
            raise SystemExit('stall cause count partition mismatch')
        if sum(int(last[k]) for k in ['immutable_wait_ns','l0_wait_ns','both_wait_ns']) != int(last['stall_ns']):
            raise SystemExit('stall duration partition mismatch')
        row = {'phase':root.name,'case':case,'repeat':int(repeat),
               'revision':metadata['build']['revision'],'operations':int(summary['operations']),
               'ops_s':float(summary['total_ops_s']),
               'read_p99_us':float(summary['read_p99_ns'])/1000 if summary['read_p99_ns'] else None,
               'write_p99_us':float(summary['write_p99_ns'])/1000 if summary['write_p99_ns'] else None,
               'write_p999_us':float(summary['write_p999_ns'])/1000 if summary['write_p999_ns'] else None,
               'write_max_ms':float(summary['write_max_ns'])/1e6 if summary['write_max_ns'] else None,
               'stalls':int(last['stalls']),'stall_s':int(last['stall_ns'])/1e9,
               'sst_wa':float(last['sst_write_amp']) if last['sst_write_amp'] else None,
               'peak_rss_bytes':max(int(r['rss_bytes']) for r in samples if r['rss_bytes']),
               'peak_fds':max(int(r['fds']) for r in samples if r['fds']),
               'max_l0':max(int(r['l0_files']) for r in samples),
               'final_debt_bytes':int(last['debt_bytes']),
               'settled':last['maintenance_running']=='false' and last['pending_flush']=='false' and int(last['debt_bytes'])==0}
        row.update({k:int(last[k]) for k in timings})
        rows.append(row)
groups = {}
for row in rows:
    groups.setdefault((row['phase'],row['case']),[]).append(row)
aggregates = []
for (phase,case),trials in sorted(groups.items()):
    result = {'phase':phase,'case':case,'trials':len(trials)}
    for field in ['ops_s','read_p99_us','write_p99_us','write_p999_us','write_max_ms',
                  'stalls','stall_s','sst_wa','peak_rss_bytes','peak_fds',*timings]:
        values = [r[field] for r in trials if r[field] is not None]
        result[field] = {'median':statistics.median(values),'min':min(values),'max':max(values)} if values else None
    aggregates.append(result)
a.output.mkdir(parents=True,exist_ok=True)
(a.output/'summary.json').write_text(json.dumps({'trials':rows,'aggregates':aggregates,
    'analyzer_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    'policy':'all complete verified trials; median per-trial percentiles, not pooled'},indent=2)+'\n')
with (a.output/'summary.csv').open('w',newline='') as f:
    writer=csv.DictWriter(f,fieldnames=list(rows[0]),lineterminator='\n')
    writer.writeheader();writer.writerows(rows)
lines=['| Case | Phase | Median ops/s | PUT p99 µs | Median trial max PUT ms | Writer wait s | Max flush queue ms |',
       '| --- | --- | ---: | ---: | ---: | ---: | ---: |']
for g in aggregates:
    def value(key,scale=1):
        return '—' if g[key] is None else f"{g[key]['median']*scale:,.2f}"
    lines.append(f"| {g['case']} | {g['phase']} | {value('ops_s')} | {value('write_p99_us')} | {value('write_max_ms')} | {value('stall_s')} | {value('flush_queue_max_ns',1e-6)} |")
(a.output/'summary.md').write_text('\n'.join(lines)+'\n')
print('\n'.join(lines))
print(f'{len(rows)} complete verified trials; {sum(r["operations"] for r in rows):,} foreground operations')
