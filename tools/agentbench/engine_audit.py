#!/usr/bin/env python3
"""Recheck engine campaign evidence and optionally retain a compact Git archive."""
import argparse
import gzip
import json
import os
import pathlib
import shutil

from engine_run import BINARY, ENGINES, ROOT, compiled_input, cpus, load_summary, trial_preliminary, validate_rows, write_json
from install import sha


def audit(root, archive=None):
    root = root.resolve()
    status = json.loads((root/'status.json').read_text())
    metadata = json.loads((root/'metadata.json').read_text())
    os.sched_setaffinity(0,cpus(metadata['parameters']['driver_cpus']))
    assert status['status']=='complete' and status['errors']==0
    assert sha(BINARY)==metadata['binary_sha256']
    for name,digest in json.loads((root/'source-hashes.json').read_text()).items():
        assert sha(root/'sources'/name)==digest,name
        if compiled_input(name): assert sha(ROOT/name)==digest,name
    expected={(n,mode,trial,engine) for n in metadata['parameters']['sizes'] for mode in metadata['parameters']['modes']
        for trial in range(1,(metadata['parameters']['large_runs'] if n>10000 else metadata['parameters']['runs'])+1)
        for engine in metadata['parameters'].get('engines',ENGINES)}
    assert len(status['runs'])==len(expected)
    assert {(r['n'],r['mode'],r['trial'],r['backend']) for r in status['runs']}==expected
    foreground=timed_writes=base_records=reopen_records=0
    names=[]
    for run in status['runs']:
        name=run['name'];names.append(name)
        rows=[json.loads(line) for line in (root/run['file']).read_text().splitlines()]
        validate_rows(rows,run['n'],run['operations'],run['write_operations'])
        resource=json.loads((root/(name+'.resources.json')).read_text())
        assert resource['final_population']==run['final_population']==run['n']+run['write_operations']
        assert resource['preload']['base_records']==run['n']
        assert resource['preload']['reopen_verification_seconds']>0
        assert resource['preload']['lmdb_helix_bulk_transactions']==1
        assert cpus(resource['cpus_allowed_list'])==cpus(metadata['parameters']['cpus'])
        if run['mode']=='durable':
            assert run['write_operations']<=10000
        if run['backend']!='kiban':
            assert resource['engine_settings']['lmdb_flags_bits']==(0 if run['mode']=='durable' else 327680)
            if run['backend']=='helix':assert resource['engine_settings']['bm25'] is False
        else:assert resource['engine_settings']['per_write_sync']==(run['mode']=='durable')
        assert json.loads((root/(name+'-reopen.jsonl')).read_text())=={'type':'reopen_verified','verified_records':run['final_population']}
        observed=load_summary(root,name,resource)
        assert observed==run['load_summary']
        assert trial_preliminary(observed)==run['preliminary']
        for row in rows:
            if row['type']=='stage':
                foreground+=row['latency']['count'];timed_writes+=row['writes']['count']
        base_records+=run['n'];reopen_records+=run['final_population']
    assert status['preliminary']==any(r['preliminary'] for r in status['runs'])
    summary={'complete_trials':len(names),'foreground_operations':foreground,
        'timed_writes_with_immediate_readback':timed_writes,'bulk_base_records':base_records,
        'base_records_verified_before_timing':2*base_records,'final_records_verified_after_reopen':reopen_records,
        'errors':0,'binary_sha256':metadata['binary_sha256'],'source_binary_affinity_flags_counts_values_and_load':'verified',
        'preliminary':status['preliminary']}
    write_json(root/'AUDIT.json',summary)
    if archive:
        archive.mkdir(parents=True,exist_ok=False)
        for name in ['metadata.json','status.json','source-hashes.json','commands.jsonl','REPORT.md','AUDIT.json']:
            shutil.copy2(root/name,archive/name)
        (archive/'load.jsonl.gz').write_bytes(gzip.compress((root/'load.jsonl').read_bytes(),mtime=0))
        for name in names:
            for suffix in ['.jsonl','.config.json','.resources.json','-reopen.jsonl','-reopen.config.json','-reopen.resources.json']:
                shutil.copy2(root/(name+suffix),archive/(name+suffix))
        manifest={path.name:sha(path) for path in archive.iterdir() if path.is_file()}
        write_json(archive/'artifact-hashes.json',manifest)
    return summary


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('results',type=pathlib.Path)
    p.add_argument('--archive',type=pathlib.Path)
    args=p.parse_args()
    print(json.dumps(audit(args.results,args.archive),indent=2))
