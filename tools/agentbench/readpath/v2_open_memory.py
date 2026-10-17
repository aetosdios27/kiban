#!/usr/bin/env python3
"""Compare frozen pre/fix binaries on private locator fixtures; never build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

import run

ROOT = run.ROOT


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def summarize(folder):
    commands = [json.loads(line) for line in (folder / 'commands.jsonl').read_text().splitlines()]
    assert len(commands) == 30
    samples = {}
    for command in commands:
        assert command['code'] == 0
        name = command['name']
        rows = [json.loads(line) for line in (folder / (name + '.jsonl')).read_text().splitlines()]
        stages = {r['workload']: r for r in rows if r['type'] == 'stage'}
        assert set(stages) == {'recall-by-id', 'recent-for-agent'}
        assert all(r['errors'] == 0 and r['latency']['count'] == 100000 for r in stages.values())
        complete, = [r for r in rows if r['type'] == 'complete']
        assert complete['errors'] == 0 and complete['verified_records'] == 1000000
        assert complete['timed_writes'] == 0
        status = dict(line.split(':', 1) for line in complete['proc_status'].splitlines() if ':' in line)
        assert status['Cpus_allowed_list'].strip() == '8'
        rss = int((folder / (name + '.rss-kib')).read_text().strip())
        samples[command['layout'], command['cache'], command['trial'], command['label'], command['engine']] = dict(
            rss_mib=rss / 1024, reported_hwm_mib=int(status['VmHWM'].split()[0]) / 1024,
            stages={w: r['latency'] for w, r in stages.items()})
    memory = []
    for layout in ['bulk', 'normal']:
        for cache in ['512', 'default']:
            distributions = {label: [samples[layout, cache, t, label, 'kiban']['rss_mib']
                                     for t in range(1, 4)] for label in ['before', 'after']}
            memory.append(dict(layout=layout, cache=cache, rss_mib=distributions))
    latency = []
    for workload in ['recall-by-id', 'recent-for-agent']:
        distributions = {f'{label}-{engine}-{pct}': [samples['bulk', 'default', t, label, engine]['stages'][workload][pct]
                         for t in range(1, 4)] for label in ['before', 'after']
                         for engine in ['kiban', 'lmdb'] for pct in ['p50_ns', 'p99_ns']}
        ratios = {}
        for pct in ['p50_ns', 'p99_ns']:
            for label in ['before', 'after']:
                ratios[f'{label}-kiban/lmdb-{pct}'] = [k / l for k, l in zip(
                    distributions[f'{label}-kiban-{pct}'], distributions[f'{label}-lmdb-{pct}'])]
            ratios[f'raw-after/before-{pct}'] = [a / b for a, b in zip(
                distributions[f'after-kiban-{pct}'], distributions[f'before-kiban-{pct}'])]
            ratios[f'normalized-after/before-{pct}'] = [a / b for a, b in zip(
                ratios[f'after-kiban/lmdb-{pct}'], ratios[f'before-kiban/lmdb-{pct}'])]
        latency.append(dict(workload=workload, distributions_ns=distributions, ratios=ratios))
    summary = dict(commands=len(commands), verified_calls=6000000, errors=0, memory=memory, latency=latency)
    (folder / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['measure', 'summarize'])
    p.add_argument('--before', type=Path, default=ROOT / 'target/v2-open-memory-bin/before')
    p.add_argument('--after', type=Path, default=ROOT / 'target/v2-open-memory-bin/after')
    p.add_argument('--source-data', type=Path, default=ROOT / 'target/v2-final-data/after')
    p.add_argument('--data', type=Path, default=ROOT / 'target/v2-open-memory-data')
    p.add_argument('--out', type=Path, default=ROOT / 'validation/v2/open-memory')
    a = p.parse_args()
    if a.action == 'summarize':
        summarize(a.out)
        return
    assert not a.data.exists(), 'use fresh private fixtures'
    assert not (a.out / 'commands.jsonl').exists(), 'refusing to overwrite trials'
    checks = a.out / 'checks'
    assert len(json.loads((checks / 'status.json').read_text())) == 11
    assert all(r['code'] == 0 for r in json.loads((checks / 'status.json').read_text()))
    assert not json.loads((checks / 'metadata.json').read_text())['quick']
    sources = json.loads((checks / 'source-hashes.json').read_text())
    assert all(sha256(ROOT / name) == digest for name, digest in sources.items())
    a.out.mkdir(parents=True, exist_ok=True)
    a.data.mkdir(parents=True)
    for engine, layout in [('kiban', 'bulk'), ('kiban', 'normal'), ('lmdb', 'bulk')]:
        name = f'{engine}-1000000-{layout}'
        subprocess.run(['cp', '-a', '--reflink=auto', str(a.source_data / name), str(a.data / name)], check=True)
    binaries = {label: path.resolve() for label, path in [('before', a.before), ('after', a.after)]}
    metadata = dict(before_commit='6fc0a0a6', head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    binaries={label: dict(path=str(path), sha256=sha256(path)) for label, path in binaries.items()},
                    engine_sources=sources, harness_sha256=sha256(Path(__file__)), cpu=8,
                    trials=3, operations_per_stage=100000, load=os.getloadavg(), started=time.time(),
                    claim='Remove transient full-backing copy; bulk/default warm point/latest-K latency should remain unchanged.',
                    rss_method='/usr/bin/time maximum resident set size over the complete process, KiB / 1024',
                    source_data=str(a.source_data.resolve()), private_data=str(a.data.resolve()))
    (a.out / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    for layout in ['bulk', 'normal']:
        for cache in ['512', 'default']:
            for trial in range(1, 4):
                for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
                    for engine in ['kiban', 'lmdb'] if (layout, cache) == ('bulk', 'default') else ['kiban']:
                        name = f'{layout}-{cache}-r{trial}-{label}-{engine}'
                        print(name, flush=True)
                        config = run.config(engine, 1000000, layout)
                        config.update(db=str((a.data / f'{engine}-1000000-{layout}').resolve()),
                                      cache_mib=None if cache == 'default' else int(cache), point_index=True,
                                      output=str((a.out / (name + '.jsonl')).resolve()))
                        path = a.out / (name + '.config.json')
                        path.write_text(json.dumps(config, indent=2) + '\n')
                        command = ['/usr/bin/time', '-f', '%M', '-o', str(a.out / (name + '.rss-kib')),
                                   'taskset', '-c', '8', str(binaries[label]), str(path)]
                        started = time.time()
                        with (a.out / (name + '.log')).open('w') as log:
                            result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=log)
                        with (a.out / 'commands.jsonl').open('a') as log:
                            log.write(json.dumps(dict(name=name, layout=layout, cache=cache, trial=trial,
                                                      label=label, engine=engine, command=command,
                                                      started=started, ended=time.time(), code=result.returncode,
                                                      load=os.getloadavg())) + '\n')
                        assert result.returncode == 0, (name, result.returncode)
    assert all(sha256(ROOT / name) == digest for name, digest in sources.items())
    summarize(a.out)


if __name__ == '__main__':
    main()
