#!/usr/bin/env python3
"""Inspect actual block-cache deltas; this is not a latency benchmark."""
import hashlib
import json
from pathlib import Path
import subprocess

import smallcache as campaign


def main():
    out = campaign.OUT/'rebased-scan-cache'
    out.mkdir(parents=True, exist_ok=True)
    binaries = {'before':'main-scan-probe', 'rebased':'rebased-scan-probe'}
    (out/'metadata.json').write_text(json.dumps(dict(
        cpu=6, nice=10, operations=100000,
        interface='SharedKiban gets and SharedSnapshot.range; same Version/SST/block-cache path. Cache/page counts only, not a latency comparison.',
        oracle='Full 1M-record comparison before and after; exact payload comparison on every latest-ten call.',
        binaries={s:hashlib.sha256((campaign.BINS/b).read_bytes()).hexdigest() for s,b in binaries.items()}),indent=2)+'\n')
    for cache in [32, None, 512]:
        for side, binary in binaries.items():
            c=campaign.config('kiban','bulk',True,cache)
            base='main-rebased-data' if side=='before' else 'ph3-data'
            c.update(db=str(campaign.ROOT/'target/smallcache'/base/Path(c['db']).name),
                     mmap_max_level=4294967295,operations=100000)
            name=f'bulk-cache{cache}-{side}'
            path=out/(name+'.config.json')
            assert not path.exists()
            path.write_text(json.dumps(c,indent=2)+'\n')
            print(name,flush=True)
            with (out/(name+'.jsonl')).open('w') as log:
                subprocess.run(['taskset','-c','6',str(campaign.BINS/binary),str(path)],stdout=log,stderr=log,check=True)
            rows=[json.loads(s) for s in (out/(name+'.jsonl')).read_text().splitlines()]
            assert rows[-1]==dict(errors=0,verified_records=1000000,verified_latest_calls=100064)


if __name__=='__main__':
    main()
