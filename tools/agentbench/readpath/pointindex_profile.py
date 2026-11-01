#!/usr/bin/env python3
"""Profile only warmed verified point hits; never use these runs as timed trials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

import run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--db', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--perf', type=Path, required=True)
    parser.add_argument('--layout', default='bulk', choices=['bulk', 'normal'])
    parser.add_argument('--operations', type=int, default=3000000)
    parser.add_argument('--event', default='cpu_core/cycles/u')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert not (args.out/'metadata.json').exists(), 'use a fresh output directory'
    metadata = dict(binary=str(args.binary.resolve()), sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                    cpu=8, db=str(args.db.resolve()), operations=args.operations, commands=[])
    temp_root = run.ROOT/'target/pointindex-latency'
    temp_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=temp_root) as temp:
        for mode in ['stat', 'record']:
            ctl, ack = Path(temp)/'ctl', Path(temp)/'ack'
            for path in [ctl,ack]:
                path.unlink(missing_ok=True)
                os.mkfifo(path)
            c = run.config('kiban', 1000000, args.layout, None)
            c.update(db=str(args.db.resolve()), action='profile', operations=args.operations,
                     point_index=True, stage=1)
            config = Path(temp)/'config.json'
            config.write_text(json.dumps(c))
            command = ['taskset','-c','8',str(args.perf),mode,'--control',f'fifo:{ctl},{ack}','-D','-1']
            if mode == 'stat':
                command += ['-e','cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/branches/u,cpu_core/branch-misses/u,cpu_core/cache-references/u,cpu_core/cache-misses/u',
                            '-o',str(args.out/'stat.txt')]
            else:
                command += ['-m','32','-e',args.event,'-F','499','--call-graph','dwarf,4096',
                            '--user-callchains','-o',str(args.out/'record.data')]
            command += ['--',str(args.binary.resolve()),str(config)]
            with (args.out/(mode+'.log')).open('w') as log:
                result = subprocess.run(command, stdout=log, stderr=log,
                    env={**os.environ,'READBENCH_PERF_CTL':str(ctl),'READBENCH_PERF_ACK':str(ack)})
            metadata['commands'].append(dict(command=command, code=result.returncode))
            (args.out/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
            assert result.returncode == 0
            if mode == 'record':
                commands = [(['report','--stdio','--no-children','--percent-limit','0.5'],'report.txt'),
                            (['annotate','--stdio','--symbol','kiban::db::get_from_version_at::{closure#0}'],'annotate.txt')]
                for extra, name in commands:
                    with (args.out/name).open('w') as log:
                        subprocess.run([str(args.perf),*extra,'-i',str(args.out/'record.data')], stdout=log, stderr=log, check=True)


if __name__ == '__main__':
    main()
