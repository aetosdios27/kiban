#!/usr/bin/env python3
"""Resource costs and paired scan tails, with no timing or compilation."""
import collections,json,re,statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'validation/v2';med=statistics.median

def dist(xs,scale=1):return f'{med(xs)/scale:.3f} [{min(xs)/scale:.3f}, {max(xs)/scale:.3f}]'
space=json.loads((OUT/'space.json').read_text())
lines=['# Space, write, compaction and memory costs','',
       '| Records | Layout | SST MiB before → after | Disk ratio | Point-section MiB | Files before → after |','|---:|---|---:|---:|---:|---:|']
for n in [10000,1000000]:
 for layout in ['bulk','normal']:
  a,b=[next(r for r in space if r['label']==label and r['n']==n and r['layout']==layout) for label in ['before','after']]
  lines.append(f'| {n} | {layout} | {a["sst_bytes"]/2**20:.2f} → {b["sst_bytes"]/2**20:.2f} | {b["sst_bytes"]/a["sst_bytes"]:.3f}× | {b["point_bytes"]/2**20:.2f} | {a["sst_files"]} → {b["sst_files"]} |')
lines+=['','The 1M JSON dataset has 1M distinct keys and one version per key. Bulk point metadata uses 12 slot bytes/key plus 10.667 MPHF pilot bits/key (1/3 as many u32 pilots as keys). The remaining section bytes are full-key MVCC records. This retains the original sorted data, so space is additive. Record sequence/kind/general lengths add 17 bytes/key versus the isolated tool\'s specialized decoded record.','',
        '## 10k final sync/flush/verification','',
        'These are end_sync_flush_verify_seconds from the existing write harness, median [range] across five alternating trials. They include verification and are not a pure section-build timer. Per-call W1/W4 and LMDB controls are in WRITES.md.','',
        '| Mode | Before seconds | After seconds |','|---|---:|---:|']
write_resources=[]
for mode in ['fast','durable']:
 values={}
 for label in ['before','after']:
  xs=[]
  for p in (OUT/'writes'/mode).glob(f'*{label}-kiban.jsonl'):
   for row in map(json.loads,p.read_text().splitlines()):
    if row['type']=='complete':xs.append(row['end_sync_flush_verify_seconds'])
  assert len(xs)==5
  values[label]=xs
 lines.append(f'| {mode} | {dist(values["before"])} | {dist(values["after"])} |')
 write_resources.append(dict(mode=mode,finish_seconds=values))
maintenance=[]
for line in (OUT/'maintenance-progress.log').read_text().splitlines():
 at=line.find('{"type":"maintenance"')
 if at>=0:maintenance.append(json.loads(line[at:]))
assert len(maintenance)==10
assert all(r['errors']==0 and r['verified_records']==1000000 for r in maintenance)
assert len({r['trial'] for r in maintenance})==5
lines+=['','## Isolated engine maintenance','',
        'Five alternating option-off/option-on pairs in the final engine, CPU 8, 1M fixed 8-byte binary keys and 296-byte values. Standard Rust test allocator; the JSON read/write harness uses mimalloc. TMPDIR points under this worktree on the same filesystem as the read datasets. Every output key/value is checked after timing. Flush is two 500k-key L0 flushes including durability/publication. Compaction BUILD includes merging, section construction, output sync/rename and table validation; COMMIT is timed separately. This isolates index-write cost but is not the JSON normal-ingest workload.','',
        '| Metric | Point option off median [range] | Point option on median [range] | On/off paired ratio median [range] |','|---|---:|---:|---:|']
maintenance_summary={}
for field,scale,label in [('flush_ns',1e9,'Two flushes (s)'),('compaction_build_ns',1e9,'Compaction BUILD (s)'),('compaction_commit_ns',1e6,'Compaction COMMIT (ms)'),('output_bytes',2**20,'Output MiB'),('output_files',1,'Output files'),('point_resident_bytes',2**20,'Resident point MiB after commit/verification')]:
 a=[next(r[field] for r in maintenance if r['trial']==t and not r['point_index']) for t in range(1,6)]
 b=[next(r[field] for r in maintenance if r['trial']==t and r['point_index']) for t in range(1,6)]
 ratios=[y/x for x,y in zip(a,b)] if all(a) else None
 lines.append(f'| {label} | {dist(a,scale)} | {dist(b,scale)} | {dist(ratios) if ratios else "—"} |')
 maintenance_summary[field]=dict(before=a,after=b,paired_ratios=ratios)
lines+=['','## Read process peak RSS','',
        'VmHWM from complete harness results, MiB, median [range]. This includes metadata/allocator/verification buffers and is not a bound on host page cache or builder workspace. Resident point sections and raw block residents share block_cache_bytes; existing other metadata and allocator overhead remain outside that raw-byte budget.','',
        '| Layout | Cache MiB | Kiban before | Kiban after | LMDB after |','|---|---|---:|---:|---:|']
resources=[];scan_tails=[]
for layout in ['bulk','normal']:
 for cache in ['512','32','default']:
  folder=OUT/'reads'/f'{layout}-{cache}';memory={};rows={}
  for p in folder.glob('1000000*.jsonl'):
   trial,label,engine=p.stem.split('-')[3:]
   for r in map(json.loads,p.read_text().splitlines()):
    if r['type']=='complete':
     memory[label,engine,trial]=int(re.search(r'^VmHWM:\s+(\d+)',r['proc_status'],re.M)[1])/1024
    if r['type']=='stage':rows[r['workload'],label,engine,trial]=r
  cells=[]
  for label,engine in [('before','kiban'),('after','kiban'),('after','lmdb')]:cells.append(dist([memory[label,engine,f'r{t}'] for t in range(1,6)]))
  lines.append(f'| {layout} | {cache} | '+' | '.join(cells)+' |')
  resources.append(dict(layout=layout,cache=cache,peak_rss_mib={f'{l}-{e}-{t}':v for (l,e,t),v in memory.items()}))
  ratios={}
  for label in ['before','after']:
   for percentile in ['p50_ns','p99_ns']:
    ratios[label,percentile]=[rows['recent-for-agent',label,'kiban',f'r{t}']['latency'][percentile]/rows['recent-for-agent',label,'lmdb',f'r{t}']['latency'][percentile] for t in range(1,6)]
  scan_tails.append(dict(layout=layout,cache=cache,ratios={f'{l}-{p}':v for (l,p),v in ratios.items()}))
lines+=['','## Paired latest-K p50 and p99 ratios','',
        'Kiban/its own following LMDB control, median [range]. These retain tail-control variation so raw before/after p99 shifts are not automatically attributed to the engine.','',
        '| Layout | Cache | p50 ratio before → after | p99 ratio before → after |','|---|---|---:|---:|']
for r in scan_tails:
 cells=[' → '.join(dist(r['ratios'][f'{label}-{p}']) for label in ['before','after']) for p in ['p50_ns','p99_ns']]
 lines.append(f'| {r["layout"]} | {r["cache"]} | '+' | '.join(cells)+' |')
profile=[]
for p in (OUT/'profile').glob('*.stat'):
 counts={}
 for line in p.read_text().splitlines():
  fields=line.split(';')
  if len(fields)>=3 and fields[0].replace('.','',1).replace(',','').strip().isdigit():
   counts[fields[2]]=float(fields[0].replace(',',''))/1000000
 profile.append(dict(name=p.stem,per_operation=counts))
lines+=['','## Gated full-point-operation counters','',
        'One diagnostic run per row, outside reported latency trials; 1M complete public gets including formatting/JSON. Counters are user-mode cpu_core events. These are diagnostic observations, not five-trial speedup claims.','',
        '| Layout/build/backend | cycles/op | instructions/op | LLC misses/op | dTLB misses/op |','|---|---:|---:|---:|---:|']
for r in sorted(profile,key=lambda r:r['name']):
 d=r['per_operation'];cells=[f'{d["cpu_core/"+event+"/u"]:.2f}' for event in ['cycles','instructions','LLC-load-misses','dTLB-load-misses']]
 lines.append('| '+r['name']+' | '+' | '.join(cells)+' |')
(OUT/'COSTS.md').write_text('\n'.join(lines)+'\n')
(OUT/'cost-summary.json').write_text(json.dumps(dict(maintenance=maintenance,maintenance_summary=maintenance_summary,writes=write_resources,resources=resources,scan_tails=scan_tails,profile=profile),indent=2)+'\n')
print('Validated maintenance, space, memory, write finish costs, scan controls and gated counters.')
