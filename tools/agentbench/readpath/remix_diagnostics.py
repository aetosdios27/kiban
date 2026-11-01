#!/usr/bin/env python3
"""Sequential CPU-3 build, general-range, and memtable cost measurements."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

import remix_engine as suite


def execute(cmd,folder,name,env=None):
    folder.mkdir(parents=True,exist_ok=True)
    start=suite.sibling()
    with (folder/(name+'.log')).open('w') as f:
        subprocess.run(['taskset','-c','3',*map(str,cmd)],env=env,stdout=f,stderr=f,check=True)
    end=suite.sibling()
    with (folder/'commands.jsonl').open('a') as f:
        f.write(json.dumps(dict(name=name,cmd=list(map(str,cmd)),start=start,end=end,busy_fraction=1-(end['idle']-start['idle'])/max(1,end['total']-start['total'])))+'\n')


def main():
    parser=argparse.ArgumentParser();parser.add_argument('action',choices=['range','cost','maintenance']);args=parser.parse_args()
    folder=suite.OUT/args.action
    if args.action=='range':
        for layout in ['bulk','normal']:
            for cache in ['512','default']:
                for trial in range(1,6):
                    for label in ['before','after'] if trial%2 else ['after','before']:
                        name=f'{layout}-{cache}-r{trial}-{label}'
                        if (folder/(name+'.log')).exists():continue
                        db=suite.DATA/label/f'kiban-1000000-{layout}'
                        execute([suite.BIN/(label+'-range'),db,cache],folder,name)
    elif args.action=='cost':
        for layout in ['bulk','normal']:
            source=suite.DATA/'after'/f'kiban-1000000-{layout}'
            dest=source.with_name(source.name+'-cost');assert not dest.exists()
            subprocess.run(['cp','-a','--reflink=auto',str(source),str(dest)],check=True)
            c=suite.config('after','kiban',layout);c.update(db=str(dest),action='scan-cost')
            folder.mkdir(parents=True,exist_ok=True)
            path=folder/(layout+'.config.json');path.write_text(json.dumps(c,indent=2)+'\n')
            execute([suite.BIN/'after',path],folder,layout)
            shutil.rmtree(dest)
    else:
        candidates=[p for p in (suite.ROOT/'target/release/deps').glob('kiban-*') if os.access(p,os.X_OK) and p.is_file()]
        binary=None
        for p in candidates:
            result=subprocess.run(['taskset','-c','12-21',str(p),'--list'],text=True,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
            if 'db::point_index_tests::point_index_maintenance_measurement: test' in result.stdout:binary=p;break
        assert binary is not None
        env={**os.environ,'KIBAN_POINT_BENCH_N':'1000000','KIBAN_SCAN_BENCH':'1'}
        execute([binary,'--ignored','--exact','db::point_index_tests::point_index_maintenance_measurement','--nocapture','--test-threads=1'],folder,'million',env)
        rows=[json.loads(s[s.index('{'):]) for s in (folder/'million.log').read_text().splitlines() if '"type":"maintenance"' in s]
        normalized=[{**r,'scan_index':r['point_index'],'point_index':True} for r in rows if r['type']=='maintenance']
        assert len(normalized)==10 and all(r['errors']==0 and r['verified_records']==1000000 for r in normalized)
        (folder/'measurements.json').write_text(json.dumps(normalized,indent=2)+'\n')
        (folder/'metadata.json').write_text(json.dumps(dict(cpu=3,binary=str(binary),sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),env={k:v for k,v in env.items() if k.startswith('KIBAN_')},raw_point_index_field='legacy loop selector; scan_index when KIBAN_SCAN_BENCH=1'),indent=2)+'\n')


if __name__=='__main__':main()
