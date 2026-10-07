#!/usr/bin/env python3
"""Expose endurance evolution in stage-specific minute windows."""
import argparse
import csv
import json
from pathlib import Path
import statistics

p = argparse.ArgumentParser()
p.add_argument('root',type=Path)
a = p.parse_args()
runs = json.loads((a.root/'runs.json').read_text())
windows = []
trends = []
for run in runs:
    if not run['valid']:
        raise SystemExit('failed/incomplete trial must be investigated separately')
    directory = a.root/run['directory']
    stages = {r['name'] for r in csv.DictReader((directory/'stages.csv').open())}
    rows = [r for r in csv.DictReader((directory/'samples.csv').open()) if r['stage'] in stages]
    groups = {}
    for r in rows:
        groups.setdefault((r['stage'],int(float(r['elapsed_s'])//60)), []).append(r)
    previous_sst = previous_app = 0
    for (stage,minute),samples in sorted(groups.items(),key=lambda g:g[0][1]):
        dt = sum(float(r['interval_s']) for r in samples)
        reads = sum(int(r['reads']) for r in samples)
        writes = sum(int(r['writes']) for r in samples)
        last = samples[-1]
        sst,app = int(last['sst_bytes_written']),int(last['app_bytes_total'])
        row = {'run':run['directory'],'stage':stage,'minute':minute,'interval_s':dt,
               'reads':reads,'writes':writes,'ops_s':(reads+writes)/dt,
               'interval_sst_wa':(sst-previous_sst)/(app-previous_app) if app>previous_app else None}
        previous_sst,previous_app = sst,app
        for kind in ['read','write']:
            values = [float(r[f'{kind}_p99_ns']) for r in samples if r[f'{kind}_p99_ns']]
            row[f'{kind}_median_interval_p99_ns'] = statistics.median(values) if values else None
            row[f'{kind}_worst_interval_p99_ns'] = max(values) if values else None
        for name in ['l0_files','debt_bytes','db_bytes','rss_bytes','fds','waiting_writers']:
            values = [int(r[name]) for r in samples if r[name]]
            row[name+'_median'] = statistics.median(values) if values else None
            row[name+'_max'] = max(values) if values else None
        windows.append(row)
    for stage in stages:
        ws = [r for r in windows if r['run']==run['directory'] and r['stage']==stage and r['interval_s']>=40]
        if len(ws)<3: continue
        n=max(1,len(ws)//3)
        first,last = ws[:n],ws[-n:]
        trend={'run':run['directory'],'stage':stage,'policy':'first versus last third of >=40-second minute windows'}
        for name in ['ops_s','write_median_interval_p99_ns','read_median_interval_p99_ns',
                     'rss_bytes_median','db_bytes_median','debt_bytes_median','fds_median']:
            f=[r[name] for r in first if r[name] is not None];l=[r[name] for r in last if r[name] is not None]
            before=statistics.median(f) if f else None;after=statistics.median(l) if l else None
            trend[name]={'first':before,'last':after,'ratio':after/before if before else None}
        trends.append(trend)
with (a.root/'windows.csv').open('w',newline='') as f:
    w=csv.DictWriter(f,fieldnames=list(windows[0]),lineterminator='\n');w.writeheader();w.writerows(windows)
(a.root/'trends.json').write_text(json.dumps(trends,indent=2)+'\n')
print(f'{len(windows)} stage-specific windows; {len(trends)} trends')
print(json.dumps(trends,indent=2))
