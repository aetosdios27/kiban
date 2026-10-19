#!/usr/bin/env python3
"""Diagnostic-only /proc sampling around the harness's profile phase.

No profiler is installed on this host. This identifies CPU vs I/O pressure,
not source-level hot functions. Sampling runs are excluded from paired tables.
"""
import argparse
import collections
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time


def read_process(pid):
    proc = Path('/proc')/str(pid)
    stat = (proc/'stat').read_text().split(') ')[1].split()
    io = dict(line.split(': ') for line in (proc/'io').read_text().splitlines())
    return dict(cpu_ticks=int(stat[11])+int(stat[12]), **{k: int(v) for k,v in io.items()})


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--before', type=Path, required=True)
    p.add_argument('--after', type=Path, required=True)
    p.add_argument('--data', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    for indexed in [False, True]:
        for cache in [32, None]:
            for label in ['before', 'after']:
                name = f'{label}-index{int(indexed)}-cache{cache}'
                c = dict(backend='kiban', db=str((a.data/f'{label}-kiban-bulk-index{int(indexed)}-1000000').resolve()), n=1_000_000,
                         operations=1_000_000, k=10, seed=20261016, output='', action='profile', layout='bulk', cache_mib=cache,
                         point_index=indexed, binary_fuse_filter=label=='after', stage=1, rocks={})
                cfg = a.out/(name+'.config.json')
                cfg.write_text(json.dumps(c,indent=2)+'\n')
                with tempfile.TemporaryDirectory(prefix='kiban-fuse-profile-') as tmp:
                    ctl, ack = Path(tmp)/'ctl', Path(tmp)/'ack'
                    os.mkfifo(ctl)
                    os.mkfifo(ack)
                    cfd = os.open(ctl, os.O_RDWR)
                    afd = os.open(ack, os.O_RDWR)
                    with (a.out/(name+'.log')).open('w') as log:
                        process = subprocess.Popen(['taskset','-c','8',str(getattr(a,label).resolve()),str(cfg.resolve())], stdout=log, stderr=log,
                                                   env={**os.environ,'READBENCH_PERF_CTL':str(ctl),'READBENCH_PERF_ACK':str(ack)})
                        with os.fdopen(cfd) as control:
                            assert control.readline().strip() == 'enable'
                            before = read_process(process.pid)
                            states = collections.Counter()
                            stop = threading.Event()
                            def sample():
                                while not stop.is_set():
                                    try:
                                        states[(Path('/proc')/str(process.pid)/'wchan').read_text().strip()] += 1
                                    except FileNotFoundError:
                                        break
                                    stop.wait(.005)
                            thread = threading.Thread(target=sample)
                            thread.start()
                            tick = time.monotonic()
                            os.write(afd,b'ack\n')
                            assert control.readline().strip() == 'disable'
                            seconds = time.monotonic()-tick
                            after = read_process(process.pid)
                            stop.set()
                            thread.join()
                            os.write(afd,b'ack\n')
                        os.close(afd)
                        assert process.wait(timeout=120) == 0
                delta = {k: after[k]-before[k] for k in before}
                row = dict(name=name,seconds=seconds,cpu_seconds=delta['cpu_ticks']/os.sysconf('SC_CLK_TCK'),delta=delta,wchan_samples=states,
                           source_limitation='No source-level sampler installed; only CPU/I/O phase accounting is established.')
                (a.out/(name+'.json')).write_text(json.dumps(row,indent=2)+'\n')
                print(name, json.dumps(row), flush=True)


if __name__ == '__main__':
    main()
