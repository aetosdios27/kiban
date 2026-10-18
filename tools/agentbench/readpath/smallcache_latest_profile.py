#!/usr/bin/env python3
"""Profile latest-ten on the frozen binaries, separately from latency trials."""
import argparse
import hashlib
import json
import os
from pathlib import Path

os.environ['READBENCH_BUILD_CPUS'] = '12,13,14,15,16,17,18,19,20,21'
os.environ['READBENCH_PERF_PAGES'] = '8'
import smallcache as campaign
import profile as profiler


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, default=campaign.OUT/'rebased-latest-profile')
    parser.add_argument('--stacks', action='store_true')
    parser.add_argument('--clocks', action='store_true')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    profiler.CPU = 6
    profiler.EVENTS += ',minor-faults:u,major-faults:u'
    if args.clocks:
        profiler.EVENTS += ',cpu_core/ref-cycles/u,task-clock:u'
    labels = {'before': 'main-rebased', 'rebased': 'rebased'}
    (args.out/'metadata.json').write_text(json.dumps(dict(
        cpu=6, stage=2, counters_operations=1000000, stacks_operations=300000,
        events=profiler.EVENTS,
        binaries={k:hashlib.sha256((campaign.BINS/v).read_bytes()).hexdigest() for k,v in labels.items()}),indent=2)+'\n')
    for cache in [32, None, 512]:
        for side in (['before','rebased'] if cache != None else ['rebased','before']):
            profiler.BINARY = campaign.BINS/labels[side]
            for engine in ['kiban','lmdb']:
                c = campaign.config(engine,'bulk',True,cache)
                if engine == 'kiban':
                    base = 'main-rebased-data' if side == 'before' else 'ph3-data'
                    c['db'] = str(campaign.ROOT/'target/smallcache'/base/Path(c['db']).name)
                c.update(action='profile',stage=2,operations=1000000,mmap_max_level=4294967295)
                name = f'bulk-cache{cache}-{side}-{engine}'
                assert not (args.out/(name+'.config.json')).exists()
                print(name,flush=True)
                profiler.collect(c,args.out,name)
                if args.stacks and engine == 'kiban' and cache in [32,512]:
                    c['operations']=300000
                    profiler.collect(c,args.out,name+'-stacks',record=True,flamegraph=False)


if __name__ == '__main__':
    main()
