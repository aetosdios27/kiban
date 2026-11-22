#!/usr/bin/env python3
"""Repeat only the frozen libmdbx read matrix with a whole-round page-fault gate."""
import argparse
from itertools import groupby
import os
from pathlib import Path
import re

from libmdbx_bench import BINARY, DATA, ROOT, VALIDATION, digest, dump, fixture, host, load, matrix, provenance, rows
from rocksdb_bench import run

MAX_ATTEMPTS = 10


def memory():
    pressure = Path('/proc/pressure/memory').read_text()
    meminfo = Path('/proc/meminfo').read_text()
    available = re.search(r'^MemAvailable:\s+(\d+) kB$', meminfo, re.M)
    return dict(pressure=pressure, mem_available_kib=int(available[1]))


def major_faults(path):
    faults, = re.findall(r'^\s*Major \(requiring I/O\) page faults:\s*(\d+)$', path.read_text(), re.M)
    return int(faults)


def rounds(raw):
    def key(item):
        scenario, name, _ = item
        return scenario, name.split('-', 1)[0]

    for (scenario, trial), group in groupby(matrix(raw, False), key):
        if scenario.startswith('read-'):
            yield scenario, trial, [(name, c) for _, name, c in group]


def benchmark(c, folder, name, resume):
    folder.mkdir(parents=True, exist_ok=True)
    receipts = folder / 'commands.jsonl'
    pending = folder / f'{name}.pending'
    if pending.exists():
        assert receipts.exists() and any(r['name'] == name for r in rows(receipts)), (
            f'{pending}: incomplete command; preserve its receipts and investigate')
    else:
        pending.touch()
    run(c, folder, name, resume, binary=BINARY, data=DATA, fixture_path=fixture, sample=memory)
    pending.unlink()


def prepare(raw, out, resume):
    seen = set()
    for _, _, commands in rounds(raw):
        for _, c in commands:
            db = fixture(c)
            if db in seen:
                continue
            seen.add(db)
            if not db.exists():
                benchmark(dict(c, cache_mib=None, action='prepare', durable=c['backend'] == 'libmdbx'),
                          out / 'prepare', db.name, resume)
            else:
                assert not (out / 'prepare' / f'{db.name}.pending').exists(), db


def measure(raw, out, resume):
    exhausted = []
    for scenario, trial, commands in rounds(raw):
        for attempt in range(1, MAX_ATTEMPTS + 1):
            accepted = out / scenario / trial
            discarded = out / 'discarded' / scenario / f'{trial}-a{attempt}'
            if discarded.exists():
                assert resume, discarded
                continue
            if accepted.exists():
                assert resume and load(accepted / 'gate.json')['attempt'] == attempt, accepted
                break
            staging = out / 'in-progress' / scenario / f'{trial}-a{attempt}'
            for name, c in commands:
                benchmark(c, staging, name, resume)
            faults = {name: major_faults(staging / f'{name}.time.txt') for name, _ in commands}
            dump(staging / 'gate.json', dict(attempt=attempt, major_faults=faults))
            dest = discarded if any(faults.values()) else accepted
            dest.parent.mkdir(parents=True, exist_ok=True)
            staging.rename(dest)
            if dest == accepted:
                break
            print(f'Discarded {scenario}/{trial}-a{attempt} with major faults {faults}', flush=True)
        else:
            exhausted.append(f'{scenario}/{trial}')
            print(f'{exhausted[-1]}: no qualifying round in {MAX_ATTEMPTS} attempts', flush=True)
    return exhausted


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('action', choices=('prepare', 'measure'))
    parser.add_argument('--raw', type=Path, default=VALIDATION / 'raw')
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args()
    assert os.sched_getaffinity(0) <= set(range(12, 22))
    assert os.getpriority(os.PRIO_PROCESS, 0) >= 10
    raw = args.raw.resolve()
    out = raw / 'measure-reads-quiet'
    current = provenance()
    original = load(raw / 'metadata.json')
    assert current['binary_sha256'] == original['binary_sha256'], 'read binary differs from the frozen campaign'
    current['sources'][str(Path(__file__).resolve().relative_to(ROOT))] = digest(Path(__file__))
    current.update(selection_sha256=digest(raw / 'selection.json'), max_attempts=MAX_ATTEMPTS)
    metadata = out / 'metadata.json'
    continuation = out / 'continuation-metadata.json'
    if metadata.exists():
        assert args.resume
        frozen = load(metadata)
        if continuation.exists():
            assert load(continuation) == current
        elif current != frozen:
            driver = str(Path(__file__).resolve().relative_to(ROOT))
            old_sources = frozen['sources']
            for path in current['sources']:
                if path != driver:
                    assert current['sources'][path] == old_sources[path], path
            assert current == dict(frozen, sources=current['sources'])
            dump(continuation, current)
    else:
        assert args.action == 'prepare'
        out.mkdir(parents=True, exist_ok=True)
        dump(metadata, current)
        dump(out / 'host.json', host())
    if args.action == 'prepare':
        prepare(raw, out, args.resume)
    else:
        exhausted = measure(raw, out, args.resume)
        staging = out / 'in-progress'
        if staging.exists():
            for path in staging.iterdir():
                path.rmdir()
            staging.rmdir()
        if exhausted:
            raise SystemExit('Read rounds exhausted: ' + ', '.join(exhausted))


if __name__ == '__main__':
    main()
