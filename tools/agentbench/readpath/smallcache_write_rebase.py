#!/usr/bin/env python3
"""Five paired write trials against PH3 main, using one active database path."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

import smallcache as campaign


def main():
    assert not os.environ.get('LD_PRELOAD'), 'Write latency trials must not use a preload'
    root,out=campaign.ROOT,campaign.OUT
    status=json.loads((out/'rebased-checks/status.json').read_text())
    assert len(status)==11 and all(r['code']==0 for r in status)
    hashes=json.loads((out/'rebased-checks/source-hashes.json').read_text())
    assert all(hashlib.sha256((root/p).read_bytes()).hexdigest()==h for p,h in hashes.items())
    main_data=root/'target/smallcache/main-rebased-data'
    indexed=campaign.config('kiban','bulk',True,None,10000)
    indexed.update(action='prepare',db=str(main_data/Path(indexed['db']).name))
    if not Path(indexed['db']).exists():
        campaign.execute('main-rebased',indexed,out/'rebased-write-prepare','bulk-index1')
    folder=out/'rebased-write-confirm'
    folder.mkdir(parents=True,exist_ok=True)
    active=root/'target/smallcache/write-rebased-active'
    assert not active.exists()
    binaries={'before':'main-rebased','rebased':'rebased'}
    (folder/'metadata.json').write_text(json.dumps(dict(
        baseline='37feee99f925d895aa0f00c32d902e0690089bde',trials=5,
        cpu=6,build_cpus='12-21',nice=10,engine_source_hashes=hashes,
        cases='Fast/durable, index off/on; default cache and mmap None; 10k operations per stage',
        ordering='Alternate sides within every pair; reverse case ordering on even trials',
        active_path=str(active),
        binaries={s:hashlib.sha256((campaign.BINS/b).read_bytes()).hexdigest() for s,b in binaries.items()}),indent=2)+'\n')
    cases=[(False,False),(True,False),(False,True),(True,True)]
    for trial in range(1,6):
        for index,durable in (cases if trial%2 else cases[::-1]):
            for side in (['before','rebased'] if trial%2 else ['rebased','before']):
                for engine in ['kiban','lmdb']:
                    c=campaign.config(engine,'bulk',index,None,10000)
                    if engine=='kiban' and index:
                        data=main_data if side=='before' else root/'target/smallcache/ph3-data'
                        c['db']=str(data/Path(c['db']).name)
                    subprocess.run(['cp','-a','--reflink=auto',c['db'],str(active)],check=True)
                    c.update(db=str(active),action='write',operations=10000,
                             readers=1,durable=durable,mmap_max_level=None)
                    name=f'bulk-index{int(index)}-cacheNone-readers1-durable{int(durable)}-r{trial}-{side}-{engine}'
                    campaign.execute(side,c,folder,name,binaries[side])
                    shutil.rmtree(active)
    subprocess.run(['python3',str(root/'tools/agentbench/readpath/smallcache_report.py'),
                    str(folder),'--after','rebased'],check=True)
    assert all(hashlib.sha256((root/p).read_bytes()).hexdigest()==h for p,h in hashes.items())


if __name__=='__main__':
    main()
