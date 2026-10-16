#!/usr/bin/env python3
"""One command: build/install, warm, alternate, verify/reopen, summarize."""
import argparse
import contextlib
import datetime
import fcntl
import hashlib
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import time
import urllib.request

import hydra
import install
import report

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def capture(args):
    p = subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return {'command': args, 'exit_code': p.returncode, 'output': p.stdout}


class Campaign:
    def __init__(self, output, timeout):
        self.output, self.timeout = output, timeout
        self.loads = []
        self.measuring = False

    def load(self):
        row = {'time': datetime.datetime.now(datetime.timezone.utc).isoformat(),
               'load': os.getloadavg(), 'measuring': self.measuring}
        self.loads.append(row)
        with (self.output / 'load.jsonl').open('a') as f:
            f.write(json.dumps(row) + '\n')

    def command(self, args, cwd=ROOT, env=None):
        self.load()
        args = list(map(str, args))
        with (self.output / 'commands.jsonl').open('a') as f:
            f.write(json.dumps({'argv': args, 'cwd': str(cwd), 'time': time.time()}) + '\n')
        index = len((self.output / 'commands.jsonl').read_text().splitlines())
        with (self.output / f'command-{index:03}.log').open('w') as log:
            proc = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            start, wall = time.monotonic(), time.time()
            try:
                while proc.poll() is None:
                    self.load()
                    if time.monotonic() - start > self.timeout:
                        raise RuntimeError(f'Command timed out: {args}')
                    if abs((time.time() - wall) - (time.monotonic() - start)) > 2:
                        raise RuntimeError('Wall/monotonic clock gap; campaign invalid')
                    time.sleep(1)
                self.load()
                if proc.returncode:
                    raise RuntimeError(f'Command failed ({proc.returncode}): {args}; see {log.name}')
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGTERM)
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(proc.pid, signal.SIGKILL)
                        proc.wait()


@contextlib.contextmanager
def server(binary, data, port, log_path, campaign):
    # Refuse to reuse or interfere with any existing listener.
    with socket.socket() as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(('127.0.0.1', port))
    data.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, HELIX_DATA_DIR=str(data), HELIX_PORT=str(port), RUST_LOG='error')
    with log_path.open('w') as log:
        with (campaign.output / 'commands.jsonl').open('a') as commands:
            commands.write(json.dumps({'argv': [str(binary)], 'env': {
                'HELIX_DATA_DIR': str(data), 'HELIX_PORT': str(port), 'RUST_LOG': 'error'},
                'cwd': str(ROOT), 'time': time.time()}) + '\n')
        proc = subprocess.Popen([binary], env=env, cwd=ROOT, stdout=log,
                                stderr=subprocess.STDOUT, start_new_session=True)
        try:
            endpoint = f'http://127.0.0.1:{port}'
            for _ in range(120):
                if proc.poll() is not None:
                    raise RuntimeError(f'Helix exited {proc.returncode}: {log_path}')
                try:
                    request = urllib.request.Request(endpoint + '/Noop', data=b'{}',
                                                     headers={'Content-Type': 'application/json'})
                    with urllib.request.urlopen(request, timeout=1) as response:
                        if 'agentbench-noop' not in json.dumps(json.load(response)):
                            raise RuntimeError('Unexpected Noop body')
                    break
                except (OSError, TimeoutError):
                    time.sleep(0.25)
            else:
                raise RuntimeError('Helix did not become ready')
            yield endpoint
            if proc.poll() is not None:
                raise RuntimeError(f'Helix died during run: {proc.returncode}')
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
                    raise RuntimeError('Helix did not stop within 10s')
                if proc.returncode != 0:
                    raise RuntimeError(f'Helix shutdown failed: exit {proc.returncode}')


def source_snapshot(output):
    files = list((ROOT / 'src').rglob('*.rs')) + list((HERE / 'src').glob('*.rs'))
    files += [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', HERE / 'Cargo.toml', HERE / 'Cargo.lock',
              ROOT / 'examples/scale/metrics.rs', ROOT / 'examples/support/random.rs']
    files += list(HERE.glob('*.py')) + list((HERE / 'helix').glob('*.hx'))
    hashes = {}
    for path in files:
        relative = path.relative_to(ROOT)
        target = output / 'sources' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        content = path.read_bytes()
        target.write_bytes(content)
        hashes[str(relative)] = hashlib.sha256(content).hexdigest()
    write_json(output / 'source-hashes.json', hashes)


def run(args):
    output = args.output.resolve()
    if not args.resume:
        output.mkdir(parents=True, exist_ok=False)
    lock = (output / 'supervisor.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    campaign = Campaign(output, args.timeout)
    if args.resume:
        status = json.loads((output / 'status.json').read_text())
        if status['status'] != 'incomplete' or status['errors']:
            raise RuntimeError('Only externally interrupted, incomplete campaigns may resume')
        metadata = json.loads((output / 'metadata.json').read_text())
        for key in ['n', 'operations', 'runs', 'warmup', 'k', 'seed', 'port', 'timeout', 'build_jobs']:
            setattr(args, key, metadata['parameters'][key])
        campaign.timeout = args.timeout
        campaign.loads = [json.loads(line) for line in (output / 'load.jsonl').read_text().splitlines()]
    else:
        status = {'status': 'incomplete', 'runs': [], 'errors': 0}
        write_json(output / 'status.json', status)
    try:
        if not args.resume:
            env = dict(os.environ, CARGO_BUILD_JOBS=str(args.build_jobs))
            campaign.command(['cargo', 'build', '--release', '--locked', '--manifest-path', HERE / 'Cargo.toml'], env=env)
            if args.install_helix:
                # Installation commands share the bounded process supervisor and logs.
                installation = install.install(args.install_root.resolve(),
                    lambda command, cwd=ROOT: campaign.command(command, cwd=cwd, env=env))
            else:
                installation = json.loads((args.install_root / 'installation.json').read_text())
            binary = pathlib.Path(installation['binary'])
            if install.sha(binary) != installation['binary_sha256']:
                raise RuntimeError('Helix binary differs from installation manifest')
            for name, expected in installation['source_hashes'].items():
                if install.sha(pathlib.Path(installation['source']) / name) != expected:
                    raise RuntimeError(f'Helix source changed since build: {name}')
            generated = pathlib.Path(installation['source']) / 'helix-container/src/queries.rs'
            if install.sha(generated) != installation['queries_sha256']:
                raise RuntimeError('Generated Helix queries changed since build')
            for path in (HERE / 'helix').glob('*.hx'):
                if install.sha(path) != installation['hx_sha256'][path.name]:
                    raise RuntimeError('HelixQL changed; rerun --install-helix')
            executable = HERE / 'target/release/kiban-agentbench'
            write_json(output / 'metadata.json', {
                'date_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'argv': sys.argv, 'parameters': vars(args) | {'output': str(output), 'install_root': str(args.install_root)},
                'helix': installation, 'harness_binary_sha256': install.sha(executable),
                'host': [capture(c) for c in [['uname', '-a'], ['lscpu'], ['free', '-b'],
                    ['findmnt', '-T', str(output), '-o', 'SOURCE,FSTYPE,OPTIONS'], ['lsblk', '-o', 'NAME,MODEL,ROTA,SIZE,FSTYPE'],
                    ['rustc', '-Vv'], ['cargo', '-V'], ['git', 'rev-parse', 'HEAD'], ['git', 'status', '--short'],
                    ['uptime'], ['ps', '-eo', 'pid,comm,pcpu', '--sort=-pcpu']]],
                'durability': {'kiban': 'put + sync on every write; default options',
                    'helix': 'LMDB default synchronous transaction commit; no NO_SYNC/NO_META_SYNC flags'},
                'topology': 'same host/filesystem; embedded Kiban; Helix HTTP/1.1 loopback, sequential persistent client',
                'concurrency': 1, 'hydra': hydra.probe()})
            source_snapshot(output)
            # Retain the exact generated competitor code and lockfile.
            for name in ['queries.rs', 'config.hx.json']:
                src = generated.parent / name
                if src.exists():
                    (output / ('helix-' + name)).write_bytes(src.read_bytes())
            (output / 'helix-Cargo.lock').write_bytes((pathlib.Path(installation['source']) / 'Cargo.lock').read_bytes())
        else:
            installation = metadata['helix']
            binary = pathlib.Path(installation['binary'])
            executable = HERE / 'target/release/kiban-agentbench'
            if install.sha(binary) != installation['binary_sha256'] or install.sha(executable) != metadata['harness_binary_sha256']:
                raise RuntimeError('A benchmark binary changed; cannot resume')
            inputs = json.loads((output / 'source-hashes.json').read_text())
            for name, expected in inputs.items():
                if not name.endswith('.py') and install.sha(ROOT / name) != expected:
                    raise RuntimeError(f'Measured source changed: {name}')
            for name, expected in installation['source_hashes'].items():
                if install.sha(pathlib.Path(installation['source']) / name) != expected:
                    raise RuntimeError(f'Helix source changed: {name}')
            recovery = output / 'resumptions'
            recovery.mkdir(exist_ok=True)
            number = len(list(recovery.glob('*.json'))) + 1
            snapshot = recovery / str(number)
            snapshot.mkdir()
            for source in HERE.glob('*.py'):
                (snapshot / source.name).write_bytes(source.read_bytes())
            write_json(recovery / f'{number}.json', {
                'time': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'argv': sys.argv, 'git': capture(['git', 'rev-parse', 'HEAD']),
                'note': 'External session interruption; completed trials retained. Missing load samples are not filled.'})
        campaign.load()
        if max(os.getloadavg()) >= 2 and not args.allow_preliminary:
            raise RuntimeError('Load >= 2. Rerun when quiet, or explicitly use --allow-preliminary')
        def invoke(backend, name, n, operations, endpoint='', verify=False, data=None):
            config = {'backend': backend, 'db': str(data or output / 'data' / name),
                      'endpoint': endpoint, 'n': n, 'operations': operations, 'k': args.k,
                      'seed': args.seed, 'output': str(output / (name + '.jsonl')), 'verify_only': verify}
            path = output / (name + '.config.json')
            write_json(path, config)
            campaign.command([executable, path])
            return [json.loads(line) for line in pathlib.Path(config['output']).read_text().splitlines()]
        for trial in range(args.runs):
            # A,B,A,B order is fixed and recorded; no engine runs concurrently.
            for backend in ['kiban', 'helix']:
                name = f'r{trial + 1}-{backend}'
                if any(r['trial'] == trial + 1 and r['backend'] == backend for r in status['runs']):
                    continue
                print(name, flush=True)
                data = output / 'data' / name
                existing = output / (name + '.jsonl')
                if existing.exists():
                    rows = [json.loads(line) for line in existing.read_text().splitlines()]
                    if not rows or rows[-1]['type'] != 'complete':
                        raise RuntimeError(f'Partial workload retained at {existing}; no automatic rerun')
                    verification = output / (name + '-reopen.jsonl')
                    if not verification.exists():
                        if backend == 'kiban':
                            invoke(backend, name + '-reopen', args.n, args.operations, verify=True, data=data)
                        else:
                            with server(binary, data, args.port, output / (name + '-recovered-reopen-server.log'), campaign) as endpoint:
                                invoke(backend, name + '-reopen', args.n, args.operations, endpoint, verify=True)
                    marker = [json.loads(line) for line in verification.read_text().splitlines()]
                    if marker[-1]['type'] != 'reopen_verified':
                        raise RuntimeError('Recovered workload has no successful reopen verification')
                    if backend == 'helix':
                        floor_path = output / (name + '-floor.jsonl')
                        floor = [json.loads(line) for line in floor_path.read_text().splitlines()]
                        if floor[-1]['type'] != 'complete':
                            raise RuntimeError('Recovered floor incomplete')
                        status['runs'].append({'trial': trial + 1, 'backend': 'helix-floor', 'file': floor_path.name})
                    status['runs'].append({'trial': trial + 1, 'backend': backend, 'file': existing.name})
                    write_json(output / 'status.json', status)
                    print(name + ': recovered complete workload and verified persistence', flush=True)
                    continue
                if backend == 'kiban':
                    invoke(backend, name + '-warmup', args.warmup, args.warmup)
                    campaign.measuring = True
                    rows = invoke(backend, name, args.n, args.operations, data=data)
                    campaign.measuring = False
                    invoke(backend, name + '-reopen', args.n, args.operations, verify=True, data=data)
                else:
                    warm_data = output / 'data' / (name + '-warmup')
                    with server(binary, warm_data, args.port, output / (name + '-warmup-server.log'), campaign) as endpoint:
                        invoke(backend, name + '-warmup', args.warmup, args.warmup, endpoint)
                        invoke('helix-floor', name + '-floor-warmup', args.warmup, args.warmup, endpoint)
                    with server(binary, data, args.port, output / (name + '-server.log'), campaign) as endpoint:
                        campaign.measuring = True
                        floor = invoke('helix-floor', name + '-floor', args.n, args.operations, endpoint)
                        rows = invoke(backend, name, args.n, args.operations, endpoint)
                        campaign.measuring = False
                    # Restart server over the same data, then verify every final record.
                    with server(binary, data, args.port, output / (name + '-reopen-server.log'), campaign) as endpoint:
                        invoke(backend, name + '-reopen', args.n, args.operations, endpoint, verify=True)
                    status['runs'].append({'trial': trial + 1, 'backend': 'helix-floor', 'file': name + '-floor.jsonl'})
                    if floor[-1]['type'] != 'complete':
                        raise RuntimeError('Incomplete transport floor')
                if rows[-1]['type'] != 'complete':
                    raise RuntimeError('Missing full verification marker')
                status['runs'].append({'trial': trial + 1, 'backend': backend, 'file': name + '.jsonl'})
                write_json(output / 'status.json', status)
        measured = [max(row['load']) for row in campaign.loads if row['measuring']]
        status.update(status='complete', preliminary=max(measured, default=99) >= 2,
                      max_measured_load=max(measured, default=None), load_threshold=2)
    except BaseException as error:
        status.update(status='invalid', errors=1, error=str(error))
        raise
    finally:
        write_json(output / 'status.json', status)
        report.generate(output)
        lock.close()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=pathlib.Path, required=True)
    p.add_argument('--install-root', type=pathlib.Path, default=ROOT / 'target/agentbench-install')
    p.add_argument('--install-helix', action='store_true')
    p.add_argument('--resume', action='store_true', help='retain completed units of an externally interrupted campaign, using its original parameters')
    p.add_argument('--allow-preliminary', action='store_true')
    p.add_argument('--n', type=int, default=10000, help='memory population and W1 call count')
    p.add_argument('--operations', type=int, default=10000, help='calls per W2-W5 and floor')
    p.add_argument('--runs', type=int, default=5)
    p.add_argument('--warmup', type=int, default=100)
    p.add_argument('--k', type=int, default=10)
    p.add_argument('--seed', type=int, default=20261016)
    p.add_argument('--port', type=int, default=16969)
    p.add_argument('--timeout', type=int, default=3600, help='per command timeout, including initial build')
    p.add_argument('--build-jobs', type=int, default=2)
    args = p.parse_args()
    if args.runs < 5 or args.n < 64 or args.operations < 10 or args.operations % 10 or args.warmup < 64:
        p.error('require runs >= 5, n >= 64, operations positive multiple of 10, warmup >= 64')
    run(args)


if __name__ == '__main__':
    main()
