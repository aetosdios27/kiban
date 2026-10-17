#!/usr/bin/env python3
"""Verify all paired engine results and retain per-trial distributions."""
import collections,json,statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'validation/v2'
med=statistics.median
summary=[]

def get_rows(folder):
    commands=[json.loads(line) for line in (folder/'commands.jsonl').read_text().splitlines()]
    assert len(commands)==20,(folder,len(commands))
    rows=collections.defaultdict(dict)
    for command in commands:
        assert command['affinity']=='8'
        config=folder/command['config']
        trial,label,engine=config.name.removesuffix('.config.json').split('-')[3:]
        data=[json.loads(line) for line in config.with_name(config.name.replace('.config.json','.jsonl')).read_text().splitlines()]
        assert any(r['type']=='complete' for r in data)
        for row in data:
            if row['type']=='stage':
                assert row['errors']==0
                assert row['latency']['count']==(100000 if folder.parent.name=='reads' else 1000)
                rows[row['workload']][label,engine,trial]=row
    return rows

def distribution(data,label,engine,field):
    return [data[label,engine,'r'+str(t)]['latency'][field] for t in range(1,6)]
def pretty(values,unit=1000):
    return f'{med(values)/unit:.3f} [{min(values)/unit:.3f}, {max(values)/unit:.3f}]'

lines=['# Paired point-read and latest-K results','',
       'Microseconds; median [min,max] of five alternating frozen-before/frozen-after trials. Each candidate is followed by its own LMDB control. CPU 8 only; 100,000 verified timed calls per stage. Every command verifies the 1M-record database before and after timing. W2 includes key formatting, copying Kiban values and JSON decoding; W3 returns and decodes ten newest records for an agent. LMDB retains the existing harness semantics.','',
       '| Layout | Cache | Workload | Before p50 / p99 | After p50 / p99 | LMDB after p50 / p99 | Paired K/LMDB p50 before → after |','|---|---|---|---:|---:|---:|---:|']
for layout in ['bulk','normal']:
    for cache in ['512','32','default']:
        rows=get_rows(OUT/'reads'/f'{layout}-{cache}')
        assert set(rows)=={'recall-by-id','recent-for-agent'}
        for workload,data in sorted(rows.items()):
            vals={}
            for label in ['before','after']:
                for engine in ['kiban','lmdb']:
                    for pct in ['p50_ns','p99_ns']:
                        vals[label,engine,pct]=distribution(data,label,engine,pct)
            ratios={label:[data[label,'kiban','r'+str(t)]['latency']['p50_ns']/data[label,'lmdb','r'+str(t)]['latency']['p50_ns'] for t in range(1,6)] for label in ['before','after']}
            def pair(label,engine):return f'{med(vals[label,engine,"p50_ns"])/1000:.3f} / {med(vals[label,engine,"p99_ns"])/1000:.3f}'
            lines.append(f'| {layout} | {cache} | {workload} | {pair("before","kiban")} | {pair("after","kiban")} | {pair("after","lmdb")} | {med(ratios["before"]):.3f} → {med(ratios["after"]):.3f} |')
            summary.append(dict(layout=layout,cache=cache,workload=workload,distributions={f'{l}-{e}-{p}':v for (l,e,p),v in vals.items()},ratios=ratios))
lines+=['','Default uses KibanOptions::default().block_cache_bytes in both builds (cache_mib=null), about 492 MiB on this host (the exact byte budget is recorded by the range probe). Default trials are separate repetitions, not reused 512-MiB observations.','',
        '## Full distributions','', '| Layout/cache/workload | Candidate | p50 µs | p99 µs | Paired p50 ratio |','|---|---|---:|---:|---:|']
for row in summary:
    for label in ['before','after']:
        d=row['distributions'];r=row['ratios'][label]
        lines.append(f'| {row["layout"]}/{row["cache"]}/{row["workload"]} | {label} | {pretty(d[label+"-kiban-p50_ns"])} | {pretty(d[label+"-kiban-p99_ns"])} | {med(r):.3f} [{min(r):.3f}, {max(r):.3f}] |')
(OUT/'READS.md').write_text('\n'.join(lines)+'\n')
(OUT/'read-summary.json').write_text(json.dumps(summary,indent=2)+'\n')

lines=['# Paired 10k write results','', 'Five alternating trials per mode, 1,000 operations per workload. Main 8bbb4934 versus optional locator-index writes. Per-call samples exclude final flush; end_sync_flush_verify_seconds is reported separately and includes verification. Durable mode syncs each mutating call.','', '| Mode | Workload | Before K p50 / p99 µs | After K p50 / p99 µs | LMDB after p50 / p99 µs | Paired K/LMDB p50 before → after | K call ops/s before → after |','|---|---|---:|---:|---:|---:|---:|']
for mode in ['fast','durable']:
    rows=get_rows(OUT/'writes'/mode)
    for work,data in sorted(rows.items()):
        pair=lambda l,e:' / '.join(f'{med(distribution(data,l,e,p))/1000:.3f}' for p in ['p50_ns','p99_ns'])
        ratios={l:[data[l,'kiban','r'+str(t)]['latency']['p50_ns']/data[l,'lmdb','r'+str(t)]['latency']['p50_ns'] for t in range(1,6)] for l in ['before','after']}
        ops={l:med([data[l,'kiban','r'+str(t)]['call_ops_s'] for t in range(1,6)]) for l in ['before','after']}
        lines.append(f'| {mode} | {work} | {pair("before","kiban")} | {pair("after","kiban")} | {pair("after","lmdb")} | {med(ratios["before"]):.3f} → {med(ratios["after"]):.3f} | {ops["before"]:.0f} → {ops["after"]:.0f} |')
(OUT/'WRITES.md').write_text('\n'.join(lines)+'\n')
print('Validated 120 read commands and 40 write commands; wrote READS.md / WRITES.md.')
