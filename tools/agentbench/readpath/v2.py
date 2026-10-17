#!/usr/bin/env python3
"""CPU-8-only persistent-index validation using the existing paired read harness.
Build and test separately; this script never invokes cargo.
"""
import argparse, hashlib, json, os, shutil, subprocess, time
from pathlib import Path
import run
ROOT=run.ROOT
DATA=ROOT/'target/v2-final-data'
OUT=ROOT/'validation/v2'
run.CPU=8

def execute(binary, config, folder, name):
    run.BINARY=binary
    if (folder/(name+'.jsonl')).exists(): raise RuntimeError('Refusing overwrite '+name)
    run.run(config,folder,name)

def config(engine,n,layout,cache,label):
    c=run.config(engine,n,layout,32 if cache=='default' else int(cache))
    c['db']=str(DATA/label/f'{engine}-{n}-{layout}')
    if cache=='default': c['cache_mib']=None
    if label=='after': c.update(point_index=True)
    return c

def space():
    rows=[]
    for label in ['before','after']:
        for n in [10000,1000000]:
            for layout in ['bulk','normal']:
                path=DATA/label/f'kiban-{n}-{layout}'
                point_bytes=0; sizes=[]; formats={}
                for f in path.glob('*.sst'):
                    with f.open('rb') as fp:
                        fp.seek(-68,2); footer=fp.read()
                    version=int.from_bytes(footer[-28:-24],'little')
                    formats[str(version)]=formats.get(str(version),0)+1
                    if version==4: point_bytes+=int.from_bytes(footer[8:16],'little')
                    sizes.append(f.stat().st_size)
                rows.append(dict(label=label,n=n,layout=layout,sst_bytes=sum(sizes),sst_files=len(sizes),point_bytes=point_bytes,formats=formats,total_file_bytes=sum(f.stat().st_size for f in path.iterdir() if f.is_file())))
    (OUT/'space.json').write_text(json.dumps(rows,indent=2)+'\n')

def main():
    p=argparse.ArgumentParser();p.add_argument('action',choices=['prepare','read','write','range','space']);p.add_argument('--before',type=Path,required=True);p.add_argument('--after',type=Path,required=True);a=p.parse_args()
    bins={k:getattr(a,k).resolve() for k in ['before','after']}
    OUT.mkdir(parents=True,exist_ok=True)
    meta=dict(binaries={k:dict(path=str(v),sha256=hashlib.sha256(v.read_bytes()).hexdigest()) for k,v in bins.items()},cpu=8,build_cpus='2,3,4,5,6,7,10,11',trials=5,operations=100000,head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),load=os.getloadavg(),time=time.time())
    (OUT/(a.action+'-metadata.json')).write_text(json.dumps(meta,indent=2)+'\n')
    if a.action=='prepare':
        for label in bins:
            (DATA/label).mkdir(parents=True,exist_ok=True)
            for n in [10000,1000000]:
                for layout in ['bulk','normal']:
                    for engine in ['kiban','lmdb']:
                        c=config(engine,n,layout,'512',label);dest=Path(c['db'])
                        if dest.exists(): raise RuntimeError('Existing database '+str(dest))
                        c['action']='prepare'
                        execute(bins[label],c,OUT/'prepare',f'{label}-{engine}-{n}-{layout}')
        space()
    elif a.action=='read':
        for layout in ['bulk','normal']:
            for cache in ['512','32','default']:
                folder=OUT/'reads'/f'{layout}-{cache}'
                for trial in range(1,6):
                    for label in ['before','after'] if trial%2 else ['after','before']:
                        for engine in ['kiban','lmdb']:
                            name=f'1000000-{layout}-{cache}-r{trial}-{label}-{engine}'
                            print(name,flush=True)
                            execute(bins[label],config(engine,1000000,layout,cache,label),folder,name)
    elif a.action=='range':
        # The unchanged baseline micro harness fixes this at 512 MiB. This is
        # also the default on this host; the final report states that limit.
        for layout in ['bulk','normal']:
            folder=OUT/'ranges'/layout
            for trial in range(1,6):
                for label in ['before','after'] if trial%2 else ['after','before']:
                    c=config('kiban',1000000,layout,'512',label)
                    c.update(action='micro',micro='iter-ten',operations=100000)
                    name=f'{layout}-r{trial}-{label}'
                    if (folder/(name+'.log')).exists(): raise RuntimeError('Existing range trial '+name)
                    execute(bins[label],c,folder,name)
    elif a.action=='write':
        for durable in [False,True]:
            folder=OUT/'writes'/('durable' if durable else 'fast')
            for trial in range(1,6):
                for label in ['before','after'] if trial%2 else ['after','before']:
                    for engine in ['kiban','lmdb']:
                        c=config(engine,10000,'bulk','32',label)
                        src=Path(c['db']);dest=DATA/f'write-{durable}-r{trial}-{label}-{engine}'
                        if dest.exists():raise RuntimeError('Existing write trial '+str(dest))
                        subprocess.run(['cp','-a','--reflink=auto',str(src),str(dest)],check=True)
                        c.update(db=str(dest),action='write',operations=1000,durable=durable)
                        name=f'10000-bulk-32-r{trial}-{label}-{engine}'
                        print(name,flush=True)
                        execute(bins[label],c,folder,name)
                        shutil.rmtree(dest)
    else:space()
if __name__=='__main__':main()
