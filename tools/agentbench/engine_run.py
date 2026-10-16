#!/usr/bin/env python3
"""Build, pin, alternate and verify the three in-process engine backends."""
import argparse
import datetime
import fcntl
import hashlib
import json
import os
import pathlib
import signal
import statistics
import subprocess
import sys
import time

import engine_report
import install

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ENGINE = HERE / 'engine'
BINARY = ENGINE / 'target/release/kiban-enginebench'
ENGINES = ('kiban', 'helix', 'lmdb')
WORKLOADS = ('remember', 'recall-by-id', 'recent-for-agent', 'update', 'mixed-90-10')


def write_json(path, data):
    temporary = path.with_name(path.name+'.tmp')
    temporary.write_text(json.dumps(data, indent=2) + '\n')
    temporary.replace(path)


def cpus(text):
    result = set()
    for part in text.strip().split(','):
        bounds = list(map(int, part.split('-')))
        if len(bounds) == 1:
            result.add(bounds[0])
        elif len(bounds) == 2 and bounds[0] <= bounds[1]:
            result.update(range(bounds[0], bounds[1] + 1))
        else:
            raise ValueError(f'Invalid CPU list: {text}')
    return result


def capture(argv):
    result = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True)
    return {'argv':argv, 'exit_code':result.returncode, 'stdout':result.stdout, 'stderr':result.stderr}


def disk_size(path):
    files = [p for p in path.rglob('*') if p.is_file()]
    if any(p.is_symlink() for p in path.rglob('*')):
        raise RuntimeError('Unexpected database symlink')
    return {'apparent_bytes':sum(p.stat().st_size for p in files),
            'allocated_bytes':sum(p.stat().st_blocks * 512 for p in files), 'files':len(files)}


def validate_rows(rows, n, operations, write_operations):
    assert rows[-1]['type'] == 'complete' and rows[-1]['verified_records'] == n+write_operations
    stages = [row for row in rows if row['type'] == 'stage']
    assert [row['workload'] for row in stages] == list(WORKLOADS)
    for i, row in enumerate(stages):
        count = write_operations if i in (0,3) else operations
        writes = count if i in (0, 3) else (count // 10 if i == 4 else 0)
        assert row['errors'] == 0 and row['latency']['count'] == count
        assert row['writes']['count'] == writes and row['reads']['count'] == count - writes
        assert row['latency']['p999_ns'] is not None or count < 10000


class Campaign:
    def __init__(self, output, timeout):
        self.output, self.timeout = output, timeout

    def command(self, argv, label, env=None, cwd=ROOT):
        argv = list(map(str, argv))
        started, wall = time.monotonic(), time.time()
        record = {'argv':argv, 'label':label, 'cwd':str(cwd), 'start_epoch':wall}
        with (self.output / 'commands.jsonl').open('a') as log:
            log.write(json.dumps(record) + '\n')
        with (self.output / (label + '.log')).open('w') as log:
            process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                while True:
                    with (self.output / 'load.jsonl').open('a') as load:
                        load.write(json.dumps({'epoch':time.time(), 'load':os.getloadavg(), 'label':label}) + '\n')
                    if process.poll() is not None:
                        break
                    if time.monotonic() - started > self.timeout:
                        raise RuntimeError(f'Timeout: {label}')
                    if abs(time.time() - wall - (time.monotonic() - started)) > 2:
                        raise RuntimeError('Wall/monotonic clock gap; trial invalid')
                    time.sleep(0.2)
                if process.returncode:
                    raise RuntimeError(f'Command failed ({process.returncode}): {label}; see {log.name}')
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()


def load_summary(root, name, resources):
    start, end = resources['measurement_start_epoch'], resources['measurement_end_epoch']
    samples = [json.loads(line) for line in (root / 'load.jsonl').read_text().splitlines()]
    observed = [s for s in samples if s['label'] == name and start <= s['epoch'] <= end]
    observed += resources.get('boundary_load_samples', [])
    observed.sort(key=lambda sample: sample['epoch'])
    epochs = [start] + [s['epoch'] for s in observed] + [end]
    gap = max(b - a for a, b in zip(epochs, epochs[1:]))
    return {'start_epoch':start, 'end_epoch':end, 'samples':len(observed),
        'largest_sample_gap_seconds':gap, 'coverage':'partial' if not observed or gap > 2.5 else 'sampled throughout',
        'load':[{'median':statistics.median(s['load'][i] for s in observed),
                 'min':min(s['load'][i] for s in observed), 'max':max(s['load'][i] for s in observed)}
                if observed else None for i in range(3)]}


def trial_preliminary(summary):
    return (summary['coverage'] != 'sampled throughout' or
            summary['load'][0] is None or summary['load'][0]['median'] >= 3)


def installation():
    manifest = json.loads((ROOT / 'target/agentbench-setup/installation.json').read_text())
    assert manifest['commit'] == install.COMMIT and manifest['version'] == install.VERSION
    source = pathlib.Path(manifest['source'])
    assert source == ROOT / 'target/agentbench-setup/helix-src', 'Engine path dependency requires the standard installation root'
    assert subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD'],text=True).strip() == install.COMMIT
    for name, digest in manifest['source_hashes'].items():
        assert install.sha(source / name) == digest, f'Helix source changed: {name}'
    for path in (HERE / 'helix').glob('*.hx'):
        assert install.sha(path) == manifest['hx_sha256'][path.name]
    return manifest


def compiled_input(name):
    return pathlib.Path(name).suffix in ('.rs', '.toml', '.lock', '.hx') or name.endswith('/equivalence.json')


def snapshot(output):
    files = list((ROOT / 'src').rglob('*.rs')) + list((HERE / 'src').glob('*.rs'))
    files += list((ENGINE / 'src').glob('*.rs')) + list((ENGINE).glob('*.py'))
    files += list(HERE.glob('*.py')) + list((HERE / 'helix').glob('*.hx'))
    files += [ROOT/'Cargo.toml', ROOT/'Cargo.lock', ENGINE/'Cargo.toml', ENGINE/'Cargo.lock',
              ENGINE/'equivalence.json', ENGINE/'sync_profile.c', ROOT/'examples/scale/metrics.rs', ROOT/'examples/support/random.rs']
    hashes = {}
    for source in files:
        relative = source.relative_to(ROOT)
        destination = output / 'sources' / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(source.read_bytes())
        hashes[str(relative)] = install.sha(source)
    write_json(output / 'source-hashes.json', hashes)


def run(args):
    assert not os.getenv('LD_PRELOAD') and not os.getenv('ENGINEBENCH_SYNC_PROFILE'), 'Diagnostic preloads are forbidden in measurement campaigns'
    bench_cpus, driver_cpus = cpus(args.cpus), cpus(args.driver_cpus)
    allowed = os.sched_getaffinity(0)
    assert bench_cpus and driver_cpus and not bench_cpus & driver_cpus
    assert bench_cpus | driver_cpus <= allowed
    pcores = pathlib.Path('/sys/devices/cpu_core/cpus')
    if pcores.exists():
        assert bench_cpus <= cpus(pcores.read_text()), 'Benchmark CPUs must be P-cores'
    os.sched_setaffinity(0, driver_cpus)
    output = args.output.resolve()
    if not args.resume:
        output.mkdir(parents=True, exist_ok=False)
    lock = (output / 'supervisor.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    campaign = Campaign(output, args.timeout)
    status = json.loads((output/'status.json').read_text()) if args.resume else {'status':'incomplete','errors':0,'runs':[]}
    assert status['status'] == 'incomplete' and status['errors'] == 0
    write_json(output/'status.json', status)
    try:
        if args.resume:
            metadata = json.loads((output/'metadata.json').read_text())
            parameters = metadata['parameters']
            assert args.cpus == parameters['cpus'] and args.driver_cpus == parameters['driver_cpus']
            for name in ('runs','large_runs','sizes','operations','modes','k','seed','timeout','engines'):
                setattr(args,name,parameters.get(name,getattr(args,name)))
            campaign.timeout = args.timeout
            assert install.sha(BINARY) == metadata['binary_sha256']
            for name, digest in json.loads((output/'source-hashes.json').read_text()).items():
                assert install.sha(ROOT/name) == digest, f'Source changed: {name}'
            campaign.command([sys.executable, ENGINE/'generate.py','--check'], 'resume-equivalence')
        else:
            if args.install_helix:
                install_count = 0
                def install_command(argv, cwd=ROOT):
                    nonlocal install_count
                    install_count += 1
                    campaign.command(argv,f'install-{install_count:03}',cwd=cwd)
                install.install(ROOT/'target/agentbench-setup',install_command)
            installed = installation()
            campaign.command([sys.executable, ENGINE/'generate.py','--check'], 'equivalence')
            if args.reuse_binary_from:
                previous = pathlib.Path(args.reuse_binary_from)
                assert install.sha(BINARY) == json.loads((previous/'metadata.json').read_text())['binary_sha256']
                for name,digest in json.loads((previous/'source-hashes.json').read_text()).items():
                    if compiled_input(name):
                        assert install.sha(ROOT/name) == digest, f'Compiled input changed before binary reuse: {name}'
            else:
                campaign.command(['cargo','build','--release','--locked','--manifest-path',ENGINE/'Cargo.toml'], 'build')
            metadata = {'date_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'argv':sys.argv, 'parameters':vars(args)|{'output':str(output)}, 'binary_sha256':install.sha(BINARY),
                'helix':installed, 'affinity':{'benchmark':sorted(bench_cpus), 'driver':sorted(driver_cpus),
                    'p_core_cpus':pcores.read_text().strip() if pcores.exists() else None,
                    'physical_cores':sorted({(pathlib.Path(f'/sys/devices/system/cpu/cpu{c}/topology/core_id')).read_text().strip() for c in bench_cpus}),
                    'exclusive_reservation':False},
                'load_rule':'Every timed trial median 1-minute load <3 with complete sampling; maxima recorded, not gated.',
                'helix_tuning':{'bm25':False,'reason':'W1-W5 do not use full-text search; avoid automatic property indexing'},
                'profile':'common upstream Helix opt-level=2, LTO, codegen-units=1, panic=abort; MiMalloc from Helix library for every backend',
                'host':[capture(argv) for argv in [['uname','-a'],['lscpu'],['lscpu','-e=CPU,CORE,SOCKET,ONLINE,MAXMHZ'],
                    ['free','-b'],['findmnt','-T',str(output),'-o','SOURCE,FSTYPE,OPTIONS'],['lsblk','-o','NAME,MODEL,SIZE,FSTYPE'],
                    ['rustc','-Vv'],['cargo','-V'],['git','rev-parse','HEAD'],['git','status','--short'],['uptime']]],
                'build_environment':{k:v for k,v in os.environ.items() if k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_TARGET') or k.startswith('CARGO_PROFILE_RELEASE_')}}
            write_json(output/'metadata.json', metadata)
            snapshot(output)
        installation()  # Recheck the pinned engine inputs on resume too.
        deadline = time.monotonic() + args.wait_seconds
        while os.getloadavg()[0] >= 3 and time.monotonic() < deadline:
            with (output/'load-wait.jsonl').open('a') as log:
                log.write(json.dumps({'epoch':time.time(),'load':os.getloadavg()})+'\n')
            time.sleep(10)
        for n, operations in zip(args.sizes, args.operations):
            for trial in range(1, (args.large_runs if n > 10000 else args.runs)+1):
                for mode in args.modes:
                    write_operations = min(operations,10000) if mode == 'durable' else operations
                    for backend in args.engines:
                        name = f'n{n}-{mode}-r{trial}-{backend}'
                        if any(run['name'] == name for run in status['runs']):
                            continue
                        print(name, flush=True)
                        config = {'backend':backend,'db':str(output/'data'/name),'n':n,'operations':operations,
                            'k':args.k,'seed':args.seed,'output':str(output/(name+'.jsonl')),
                            'verify_only':False,'mode':mode,'warmup':100,'write_operations':write_operations}
                        config_path = output/(name+'.config.json')
                        raw = output/(name+'.jsonl')
                        resource_path = output/(name+'.resources.json')
                        if raw.exists():
                            validate_rows([json.loads(line) for line in raw.read_text().splitlines()], n, operations,write_operations)
                            assert resource_path.exists(), 'Interrupted trial lacks resource marker; no automatic rerun'
                        else:
                            write_json(config_path, config)
                            campaign.command(['taskset','-c',args.cpus,BINARY,config_path],name)
                        validate_rows([json.loads(line) for line in raw.read_text().splitlines()], n, operations,write_operations)
                        resource = json.loads(resource_path.read_text())
                        assert cpus(resource['cpus_allowed_list']) == bench_cpus
                        before_reopen = disk_size(pathlib.Path(config['db']))
                        reopen = name+'-reopen'
                        if not (output/(reopen+'.jsonl')).exists():
                            verify_config = config | {'verify_only':True,'output':str(output/(reopen+'.jsonl'))}
                            write_json(output/(reopen+'.config.json'),verify_config)
                            campaign.command(['taskset','-c',args.cpus,BINARY,output/(reopen+'.config.json')],reopen)
                        marker = json.loads((output/(reopen+'.jsonl')).read_text())
                        assert marker == {'type':'reopen_verified','verified_records':n+write_operations}
                        observed = load_summary(output,name,resource)
                        preliminary = trial_preliminary(observed)
                        run_record = {'name':name,'n':n,'operations':operations,'trial':trial,'mode':mode,'backend':backend,
                            'write_operations':write_operations,'final_population':n+write_operations,'file':raw.name,'disk':before_reopen,'load_summary':observed,'preliminary':preliminary}
                        status['runs'].append(run_record)
                        write_json(output/'status.json',status)
                        print(f"  verified; load median {observed['load'][0]['median'] if observed['load'][0] else 'unknown'}; preliminary={preliminary}",flush=True)
        status.update(status='complete',preliminary=any(run['preliminary'] for run in status['runs']))
        write_json(output/'status.json',status)
    except BaseException as error:
        status.update(status='invalid',errors=1,error=str(error))
        write_json(output/'status.json',status)
        engine_report.generate(output)
        raise
    engine_report.generate(output)
    print(output/'REPORT.md',flush=True)


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output',type=pathlib.Path,required=True)
    p.add_argument('--sizes',nargs='+',type=int,default=[10000,1000000])
    p.add_argument('--operations',nargs='+',type=int,default=[10000,100000])
    p.add_argument('--modes',nargs='+',choices=['durable','fast'],default=['durable','fast'])
    p.add_argument('--engines',nargs='+',choices=ENGINES,default=list(ENGINES))
    p.add_argument('--runs',type=int,default=5)
    p.add_argument('--large-runs',type=int,default=3)
    p.add_argument('--reuse-binary-from')
    p.add_argument('--wait-seconds',type=int,default=120)
    p.add_argument('--cpus',default='1-4')
    p.add_argument('--driver-cpus',default='12-21')
    p.add_argument('--seed',type=int,default=20261016)
    p.add_argument('--k',type=int,default=10)
    p.add_argument('--timeout',type=int,default=21600)
    p.add_argument('--resume',action='store_true')
    p.add_argument('--install-helix',action='store_true')
    return p


if __name__ == '__main__':
    args = parser().parse_args()
    if len(args.sizes) != len(args.operations) or args.runs < 5 or args.large_runs < 3 or min(args.sizes) < 64 or min(args.operations) < 10000:
        raise SystemExit('Require equal size/operation lists, >=5 trials, N>=64 and >=10000 operations for p999')
    run(args)
