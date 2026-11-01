#!/usr/bin/env python3
"""Read-only, alternating large-N cache-budget diagnosis; one frozen executable."""
import argparse
import gzip
import json
import os
import pathlib
import shutil
import statistics
import sys
import time

from engine_run import Campaign, ENGINE, ROOT, capture, cpus, disk_size, installation, load_summary, snapshot, trial_preliminary, write_json
from install import sha
from report import spread

HERE=pathlib.Path(__file__).resolve().parent
BINARY=ENGINE/'target/release/kiban-cachebench'
CONFIGS=(('lmdb',None),('kiban',32),('kiban',256),('kiban',512),('kiban',1024))
WORKLOADS=('recall-by-id','recent-for-agent','mixed-key-reads-only')


def validate(root,record):
    name=record['name'];n=record['n'];operations=record['operations']
    rows=[json.loads(x) for x in (root/(name+'.jsonl')).read_text().splitlines()]
    stages=[x for x in rows if x['type']=='stage']
    assert [x['workload'] for x in stages]==list(WORKLOADS)
    for row in stages:
        assert row['errors']==0 and row['latency']['count']==row['reads']['count']==operations
        assert row['writes']['count']==0
        if operations>=10000:assert row['latency']['p999_ns'] is not None
    assert rows[-1]=={'type':'complete','verified_records':n,'timed_writes':0}
    assert json.loads((root/(name+'-reopen.jsonl')).read_text())=={'type':'reopen_verified','verified_records':n}
    r=json.loads((root/(name+'.resources.json')).read_text())
    for field in ['memory_before_timing','memory_after_timing','memory_after_final_verification']:
        assert cpus(r[field]['Cpus_allowed_list'])=={1,2,3,4}
    if record['backend']=='kiban':
        assert r['settings']['block_cache_mib']==record['cache_mib']
        assert r['settings']['range_cache_bytes']==0 and r['settings']['max_open_table_files']==128 and r['settings']['mmap_max_level'] is None
        assert [x['workload'] for x in r['cache_counters']['stages']]==list(WORKLOADS)
        for row in r['cache_counters']['stages']:
            assert row['hits']+row['misses']>0
            assert row['after']['capacity_bytes']==record['cache_mib']*1024**2
    else:assert r['settings']['lmdb_flags_bits']==327680
    return stages,r


def report(root):
    status=json.loads((root/'status.json').read_text())
    if status['status']!='complete':
        (root/'REPORT.md').write_text('Incomplete/invalid cache diagnosis; no comparison table.\n')
        return
    lines=['# Large-N read diagnosis: cache budget','',
        '**'+('Preliminary: at least one trial median load failed <3.' if status['preliminary'] else 'Measured on a shared desktop, pinned cores; every trial median load <3.')+'**','',
        'Three alternating trials per configuration; LMDB, K32, K256, K512, K1024. '+
        'N=1,000,000; 100,000 calls per stage; seed 20261016; CPUs 1–4; driver 12–21. '+
        'Fast only; no timed writes or per-read sync. W5 preserves the stage-4 key sequence, with writes converted to reads. '+
        'It is labelled mixed-key-reads-only and is not the original 90/10 workload.','',
        'Cells: median [minimum, maximum] across trials. Latencies µs; percentiles use the original histogram.','',
        '| Config | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |',
        '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    verified=[]
    for run in status['runs']:
        stages,resource=validate(root,run);verified.append((run,stages,resource))
    for workload in WORKLOADS:
        for backend,mib in CONFIGS:
            own=[stages[WORKLOADS.index(workload)] for run,stages,_ in verified if run['backend']==backend and run['cache_mib']==mib]
            assert len(own)==3
            name='LMDB' if backend=='lmdb' else f'Kiban {mib} MiB'
            fields=[spread([r['latency'][m] for r in own],1000) for m in ('p50_ns','p99_ns','p999_ns','max_ns')]
            fields += [spread([r[m] for r in own]) for m in ('verified_ops_s','call_ops_s')]
            lines.append('| '+name+' | '+workload+' | '+' | '.join(fields)+' |')
    lines += ['', '| Config | RSS before reads MiB | RSS after reads MiB | Peak RSS through reads MiB | Disk apparent MiB |',
        '| --- | ---: | ---: | ---: | ---: |']
    for backend,mib in CONFIGS:
        own=[(run,r) for run,_,r in verified if run['backend']==backend and run['cache_mib']==mib]
        name='LMDB' if backend=='lmdb' else f'Kiban {mib} MiB'
        fields=[spread([r[field][key] for _,r in own],1024**2) for field,key in [('memory_before_timing','VmRSS'),('memory_after_timing','VmRSS'),('memory_after_timing','VmHWM')]]
        fields += [spread([run['disk']['apparent_bytes'] for run,_ in own],1024**2)]
        lines.append('| '+name+' | '+' | '.join(fields)+' |')
    lines += ['', 'Cache hit rates below come from an **excluded SharedKiban counter replay**, not the timed direct Kiban API. '+
        'It reopens the same database with a fresh cache, repeats full base verification and exactly the same read sequences. '+
        'Published SST identities and levels must remain unchanged; every replay value is verified. Shared snapshot ranges supply W3. '+
        'Direct Kiban exposes options but no public cache counters; changing its timed API would add another variable.','',
        '| Cache MiB | W2 hit % | W3 hit % | W5-key hit % | Resident cache after replay MiB | SST files per level after preload |',
        '| ---: | ---: | ---: | ---: | ---: | --- |']
    for mib in (32,256,512,1024):
        own=[r for run,_,r in verified if run['cache_mib']==mib]
        hits=[spread([r['cache_counters']['stages'][i]['hit_rate']*100 for r in own]) for i in range(3)]
        resident=spread([r['cache_counters']['stages'][-1]['after']['resident_bytes'] for r in own],1024**2)
        layouts=[{k:len(v) for k,v in r['sst_layout_after_preload'].items()} for r in own]
        assert all(x==layouts[0] for x in layouts)
        lines.append(f'| {mib} | '+' | '.join(hits)+f' | {resident} | '+', '.join(f'L{k}: {v}' for k,v in layouts[0].items())+' |')
    lines += ['', '## Load and RSS during every trial','',
        '| Trial | Config | 1-minute load median [min, max] | Samples / max gap | Gate | RSS after reads MiB | Peak RSS MiB |',
        '| ---: | --- | ---: | --- | --- | ---: | ---: |']
    for run,_,r in verified:
        load=run['load_summary'];v=load['load'][0];name='LMDB' if run['backend']=='lmdb' else f"Kiban {run['cache_mib']} MiB"
        lines.append(f"| {run['trial']} | {name} | {v['median']:.2f} [{v['min']:.2f}, {v['max']:.2f}] | {load['samples']} / {load['largest_sample_gap_seconds']:.2f}s | {'preliminary' if run['preliminary'] else 'pass'} | {r['memory_after_timing']['VmRSS']/1024**2:.2f} | {r['memory_after_timing']['VmHWM']/1024**2:.2f} |")
    lines += ['', 'Read latency includes key creation inside get/recent and stored-value decoding, but excludes input sampling, '+
        'oracle comparison and instrumentation snapshots. Verified throughput includes those checks. '+
        'Bulk preload uses the original fastest correct paths and a final sync, verifies all N values, closes/reopens and verifies again. '+
        'The base is exactly N, all revision zero; this differs from the earlier five-workload campaign after W1 inserts and W4 updates. '+
        'All configs share this read-only population. Full verification warms each engine before measurement; cache clearing and OS page-cache dropping are absent. '+
        'Warmup is a discarded 100-record database. Final full verification and independent reopen are excluded. '+
        'RSS is captured before the separate counter replay; peak includes untimed preload. '+
        'Only block_cache_bytes changes; range cache remains off, file cache 128, range read-ahead default 256 KiB, mmap disabled.','']
    (root/'REPORT.md').write_text('\n'.join(lines))


def run(args):
    output=args.output.resolve();output.mkdir(parents=True,exist_ok=False)
    assert {1,2,3,4}<=os.sched_getaffinity(0) and cpus('12-21')<=os.sched_getaffinity(0)
    os.sched_setaffinity(0,cpus('12-21'))
    assert not os.getenv('LD_PRELOAD') and not os.getenv('ENGINEBENCH_SYNC_PROFILE')
    installation();campaign=Campaign(output,1200)
    if args.build:campaign.command(['cargo','build','--release','--locked','--manifest-path',ENGINE/'Cargo.toml','--bin','kiban-cachebench'],'build')
    original=ENGINE/'target/release/kiban-enginebench'
    metadata={'parameters':{k:str(v) if isinstance(v,pathlib.Path) else v for k,v in vars(args).items()}|{'output':str(output)},'argv':sys.argv,'binary':str(BINARY),'binary_sha256':sha(BINARY),
        'original_h2h_binary_sha256':sha(original) if original.exists() else None,'order':CONFIGS,'cpus':'1-4','driver_cpus':'12-21','mode':'fast',
        'date_utc':__import__('datetime').datetime.now(__import__('datetime').timezone.utc).isoformat(),
        'load_rule':'Each timed trial median 1-minute load <3 with complete coverage; maxima retained',
        'workloads':list(WORKLOADS),'w5_change':'All stage-4 key choices retained; every operation reads; no 90/10 claim',
        'counter_scope':'separate untimed SharedKiban replay after direct timing/RSS snapshots',
        'host':[capture(argv) for argv in [['uname','-a'],['lscpu'],['free','-b'],['findmnt','-T',str(output),'-o','SOURCE,FSTYPE,OPTIONS'],['rustc','-Vv'],['git','rev-parse','HEAD'],['uptime']]],
        'allocator_profile':'Same Helix MiMalloc allocator, opt-level=2, LTO, codegen-units=1 as original engine study'}
    write_json(output/'metadata.json',metadata);snapshot(output)
    status={'status':'incomplete','errors':0,'runs':[]};write_json(output/'status.json',status)
    deadline=time.monotonic()+120
    while os.getloadavg()[0]>=3 and time.monotonic()<deadline:time.sleep(5)
    try:
        for trial in range(1,4):
            for backend,mib in CONFIGS:
                assert sha(BINARY)==metadata['binary_sha256']
                if metadata['original_h2h_binary_sha256']:assert sha(original)==metadata['original_h2h_binary_sha256']
                name=f'r{trial}-'+('lmdb' if mib is None else f'kiban-{mib}')
                print(name,flush=True)
                config={'backend':backend,'cache_mib':mib,'db':str(output/'data'/name),'n':args.n,'operations':args.operations,'k':10,'seed':20261016,'output':str(output/(name+'.jsonl')),'verify_only':False}
                write_json(output/(name+'.config.json'),config)
                campaign.command(['taskset','-c','1-4',BINARY,output/(name+'.config.json')],name)
                resource=json.loads((output/(name+'.resources.json')).read_text())
                disk=disk_size(pathlib.Path(config['db']))
                verify=config|{'verify_only':True,'output':str(output/(name+'-reopen.jsonl'))}
                write_json(output/(name+'-reopen.config.json'),verify)
                campaign.command(['taskset','-c','1-4',BINARY,output/(name+'-reopen.config.json')],name+'-reopen')
                summary=load_summary(output,name,resource)
                record={'name':name,'backend':backend,'cache_mib':mib,'trial':trial,'n':args.n,'operations':args.operations,'disk':disk,'load_summary':summary,'preliminary':trial_preliminary(summary)}
                validate(output,record);status['runs'].append(record);write_json(output/'status.json',status)
                print('  verified; load',summary['load'][0], 'preliminary',record['preliminary'],flush=True)
        status.update(status='complete',preliminary=any(r['preliminary'] for r in status['runs']))
        write_json(output/'status.json',status)
    except BaseException as e:
        status.update(status='invalid',errors=1,error=str(e));write_json(output/'status.json',status);report(output);raise
    report(output)
    if args.archive:
        args.archive.mkdir(parents=True,exist_ok=False)
        files=['metadata.json','source-hashes.json','commands.jsonl','status.json','REPORT.md']
        for record in status['runs']:
            files += [record['name']+suffix for suffix in ('.jsonl','.config.json','.resources.json','-reopen.config.json','-reopen.jsonl')]
        for name in files:shutil.copy2(output/name,args.archive/name)
        (args.archive/'load.jsonl.gz').write_bytes(gzip.compress((output/'load.jsonl').read_bytes(),mtime=0))
        write_json(args.archive/'artifact-hashes.json',{p.name:sha(p) for p in args.archive.iterdir() if p.is_file()})
    print(output/'REPORT.md',flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=pathlib.Path,required=True);p.add_argument('--archive',type=pathlib.Path)
    p.add_argument('--build',action='store_true');p.add_argument('--n',type=int,default=1000000);p.add_argument('--operations',type=int,default=100000)
    run(p.parse_args())
