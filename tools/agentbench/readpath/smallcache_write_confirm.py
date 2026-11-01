#!/usr/bin/env python3
"""Confirm write controls using a constant active DB path and every trial."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

import smallcache as campaign


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cases', nargs='+', choices=['fast-indexed', 'durable-unindexed', 'durable-indexed'], default=['fast-indexed', 'durable-unindexed'])
    args = parser.parse_args()
    folder = campaign.OUT / 'ph3-write-confirm'
    folder.mkdir(parents=True, exist_ok=True)
    active = campaign.DATA / 'write-confirm-active'
    assert not active.exists()
    metadata = dict(
        cpu=6, build_cpus='12-21', nice=10, trials=5, cases=args.cases,
        reason='Confirm observed write-tail spread; keep the original full matrix.',
        difference='Use the same active DB path for both labels to remove path-name differences.',
        binaries={label:hashlib.sha256((campaign.BINS/label).read_bytes()).hexdigest()
                  for label in ['before', 'ph3']})
    (folder/'metadata.json').write_text(json.dumps(metadata, indent=2)+'\n')
    cases = {'fast-indexed': (True, False), 'durable-unindexed': (False, True), 'durable-indexed': (True, True)}
    for case in args.cases:
        indexed, durable = cases[case]
        for trial in range(1, 6):
            for label in (['before', 'ph3'] if trial % 2 else ['ph3', 'before']):
                for engine in ['kiban', 'lmdb']:
                    c = campaign.config(engine, 'bulk', indexed, 32, 10000)
                    if label == 'ph3' and engine == 'kiban' and indexed:
                        c['db'] = str(campaign.ROOT/'target/smallcache/ph3-data'/Path(c['db']).name)
                    subprocess.run(['cp', '-a', '--reflink=auto', c['db'], str(active)], check=True)
                    c.update(db=str(active), action='write', readers=1, durable=durable, operations=10000)
                    name=f'bulk-index{int(indexed)}-cache32-readers1-durable{int(durable)}-r{trial}-{label}-{engine}'
                    campaign.execute(label, c, folder, name)
                    shutil.rmtree(active)


if __name__ == '__main__':
    main()
