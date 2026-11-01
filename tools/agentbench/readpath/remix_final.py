#!/usr/bin/env python3
"""Rebuild frozen binaries or run the final, serial five-pair REMIX matrix.

Run verify.py first. Build and measure are separate invocations so compilation
cannot overlap a timed process. Existing result files are never overwritten.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import remix_engine as suite

FINAL = suite.ROOT / 'validation/remix-engine/final'


def hashes():
    files = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard',
         'src', 'Cargo.toml', 'Cargo.lock', 'tools/agentbench/readpath'],
        cwd=suite.ROOT, text=True).splitlines()
    return {f: hashlib.sha256((suite.ROOT/f).read_bytes()).hexdigest()
            for f in sorted(set(files))}


def quiet():
    """Wait for visible builds/tests and a quiet sibling without moving anyone."""
    for attempt in range(7200):
        processes = subprocess.check_output(['ps', '-eo', 'comm=,args='], text=True)
        blockers = [line for line in processes.splitlines()
                    if str(suite.ROOT) in line
                    and (line.split(maxsplit=1)[0] in {'rustc', 'clippy-driver', 'cc1', 'cc1plus', 'ld.lld'}
                         or ('/target/' in line and '/deps/kiban-' in line))]
        start = suite.sibling()
        time.sleep(.25)
        end = suite.sibling()
        busy = 1-(end['idle']-start['idle'])/max(1, end['total']-start['total'])
        if not blockers and end['idle']-start['idle'] == end['total']-start['total']:
            return
        if attempt % 120 == 0:
            print(f'waiting for quiet host: {len(blockers)} build/test processes; CPU4 busy {busy:.1%}', flush=True)
    raise RuntimeError('host did not become quiet in 30 minutes; no timed run started')


def build():
    # Independently verify every archived engine source against the requested base.
    base = json.loads((suite.OUT/'baseline.json').read_text())
    for name in base['engine_hashes']:
        expected = subprocess.check_output(['git', 'show', base['base']+':'+name], cwd=suite.ROOT)
        assert (suite.ROOT/'target/remix-before-src'/name).read_bytes() == expected, name
    subprocess.run(['taskset', '-c', '12-21', 'nice', '-n', '10', 'cargo', 'build',
                    '--release', '--locked', '--features', 'remix', '--manifest-path',
                    'tools/agentbench/readpath/Cargo.toml'], cwd=suite.ROOT, check=True)
    target = suite.BIN/'after'
    target.unlink(missing_ok=True)
    import shutil
    shutil.copy2(suite.ROOT/'tools/agentbench/readpath/target/release/kiban-readbench', target)
    env = {**os.environ, 'REMIX_ENGINE_OUT': str(FINAL)}
    subprocess.run([sys.executable, 'tools/agentbench/readpath/remix_range_build.py'],
                   cwd=suite.ROOT, env=env, check=True)
    # Baseline test harness changes CPU affinity only; engine bytes stay at main.
    baseline = suite.ROOT/'target/remix-before-src'
    test_source = baseline/'src/point_index_tests.rs'
    original = test_source.read_bytes()
    patched = original.replace(b'Cpus_allowed_list:\\t8', b'Cpus_allowed_list:\\t3')
    assert original != patched
    (FINAL/'baseline-maintenance-test.patch').write_text(
        'Test-only change in archived src/point_index_tests.rs: CPU assertion 8 -> 3.\n')
    try:
        test_source.write_bytes(patched)
        for label, root in [('before', baseline), ('after', suite.ROOT)]:
            command = ['taskset', '-c', '12-21', 'nice', '-n', '10', 'cargo', 'test',
                       '--release', '--lib', '--no-run', '--message-format=json',
                       '--manifest-path', str(root/'Cargo.toml')]
            result = subprocess.run(command, stdout=subprocess.PIPE, text=True, check=True)
            artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
            binary = next(r['executable'] for r in artifacts
                          if r.get('reason') == 'compiler-artifact' and r.get('executable')
                          and r['target']['name'] == 'kiban' and r['profile']['test'])
            shutil.copy2(binary, suite.BIN/(label+'-maintenance'))
    finally:
        test_source.write_bytes(original)
    (FINAL/'source-hashes.json').write_text(json.dumps(hashes(), indent=2)+'\n')
    (FINAL/'binaries.json').write_text(json.dumps({p.name: hashlib.sha256(p.read_bytes()).hexdigest()
        for p in [suite.BIN/n for n in ['before', 'after', 'before-range', 'after-range', 'before-maintenance', 'after-maintenance']]}, indent=2)+'\n')


def measure():
    assert json.loads((FINAL/'source-hashes.json').read_text()) == hashes(), 'source changed; rebuild'
    for name, digest in json.loads((FINAL/'binaries.json').read_text()).items():
        assert hashlib.sha256((suite.BIN/name).read_bytes()).hexdigest() == digest, name
    env = {**os.environ, 'REMIX_ENGINE_OUT': str(FINAL), 'READBENCH_CPU': '3'}
    # A quiet-host check runs before every timed process, including diagnostics.
    original = suite.execute
    def execute(*args, **kwargs):
        quiet()
        return original(*args, **kwargs)
    suite.execute = execute
    suite.OUT = FINAL
    for action in ['reads', 'varied', 'writes']:
        sys.argv = [__file__, action, '--trials', '5']
        suite.main()
    import remix_diagnostics as diagnostics
    original_diagnostic = diagnostics.execute
    def diagnostic(*args, **kwargs):
        quiet()
        return original_diagnostic(*args, **kwargs)
    diagnostics.execute = diagnostic
    for action in ['range', 'cost']:
        sys.argv = [__file__, action]
        diagnostics.main()
    rows = []
    for trial in range(1, 6):
        for label in ['before', 'after'] if trial % 2 else ['after', 'before']:
            name = f'r{trial}-{label}'
            folder = FINAL/'maintenance'
            phase_env = {**env, 'KIBAN_POINT_BENCH_N': '1000000',
                         'KIBAN_POINT_BENCH_TRIAL': str(trial)}
            if label == 'after':
                phase_env['KIBAN_SCAN_BENCH'] = '1'
            if not (folder/(name+'.log')).exists():
                diagnostic([suite.BIN/(label+'-maintenance'), '--ignored', '--exact',
                            'db::point_index_tests::point_index_maintenance_measurement',
                            '--nocapture', '--test-threads=1'], folder, name, phase_env)
            records = [json.loads(line[line.index('{'):])
                       for line in (folder/(name+'.log')).read_text().splitlines()
                       if '\"type\":\"maintenance\"' in line]
            assert len(records) == 2
            selected = next(r for r in records if r['point_index'])
            assert selected['errors'] == 0 and selected['verified_records'] == 1000000
            rows.append({**selected, 'label': label, 'scan_index': label == 'after'})
    (FINAL/'maintenance/measurements.json').write_text(json.dumps(rows, indent=2)+'\n')
    subprocess.run([sys.executable, 'tools/agentbench/readpath/remix_engine_report.py'],
                   cwd=suite.ROOT, env=env, check=True)
    assert json.loads((FINAL/'source-hashes.json').read_text()) == hashes(), 'source changed during run'


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['build', 'measure'])
    args = parser.parse_args()
    FINAL.mkdir(parents=True, exist_ok=True)
    if args.action == 'build':
        build()
    else:
        measure()
