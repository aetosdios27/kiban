#!/usr/bin/env python3
"""Alternate frozen current-PH3 and candidate maintenance test executables."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

from pointindex_latency import sibling


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert not (args.out/'metadata.json').exists(), 'use a fresh output directory'
    binaries = dict(before=args.before.resolve(), after=args.after.resolve())
    (args.out/'metadata.json').write_text(json.dumps(dict(cpu=8, n=1000000, trials=5,
        binaries={k:dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for k,p in binaries.items()}),indent=2)+'\n')
    for trial in range(1,6):
        for label in (['before','after'] if trial % 2 else ['after','before']):
            command = ['taskset','-c','8',str(binaries[label]),'point_index_maintenance_measurement',
                       '--ignored','--nocapture','--test-threads=1']
            name = f'r{trial}-{label}'
            start = sibling()
            tick = time.time()
            with (args.out/(name+'.log')).open('w') as log:
                result = subprocess.run(command, stdout=log, stderr=log,
                    env={**os.environ,'KIBAN_POINT_BENCH_N':'1000000','KIBAN_POINT_BENCH_TRIAL':str(trial),
                         'KIBAN_POINT_BENCH_PROFILE':'1'})
            end = sibling()
            text = (args.out/(name+'.log')).read_text()
            records = [json.loads(line) for line in text.splitlines() if line.startswith('{"type":"maintenance"')]
            assert result.returncode == 0 and len(records) == 2, text
            assert {r['point_index'] for r in records} == {False,True}
            assert all(r['errors'] == 0 and r['verified_records'] == 1000000 and r['trial'] == trial for r in records)
            row = dict(name=name, trial=trial, label=label, command=command, code=result.returncode,
                       seconds=time.time()-tick, start=start, end=end,
                       cpu9_busy_percent=100*(1-(end['idle']-start['idle'])/max(1,end['total']-start['total'])),records=records)
            with (args.out/'commands.jsonl').open('a') as log:
                log.write(json.dumps(row)+'\n')
            print(name, row['seconds'], flush=True)


if __name__ == '__main__':
    main()
