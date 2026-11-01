"""Pinned official CLI + source build. No Docker daemon, login or cloud resources."""
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import urllib.request

VERSION = '2.3.5'
COMMIT = '17e7ecf764aecd553e1f54ca25320d654153a9aa'
CLI_SHA256 = '8a0d39c0955fb091a459882fecbaa352cfd36b968b73cc91ae35cbdd641a14de'
HERE = pathlib.Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def install(root, command):
    root.mkdir(parents=True, exist_ok=True)
    cli = root / 'helix'
    if not cli.exists():
        urllib.request.urlretrieve(f'https://github.com/HelixDB/helix-db/releases/download/v{VERSION}/helix-x86_64-unknown-linux-gnu', cli)
    if sha(cli) != CLI_SHA256:
        raise RuntimeError('Helix CLI checksum mismatch (installer supports Linux x86_64)')
    cli.chmod(0o755)
    src = root / 'helix-src'
    if not src.exists():
        command(['git', 'clone', '--depth', '1', '--branch', f'v{VERSION}',
                 'https://github.com/HelixDB/helix-db.git', str(src)])
    actual = subprocess.check_output(['git', '-C', str(src), 'rev-parse', 'HEAD'], text=True).strip()
    if actual != COMMIT:
        raise RuntimeError(f'Helix source pin mismatch: {actual}')
    project = root / 'query-project'
    (project / 'db').mkdir(parents=True, exist_ok=True)
    (project / 'helix.toml').write_text('[project]\nname = "agentbench"\nqueries = "db"\ncontainer_runtime = "podman"\n\n[local.dev]\nport = 16969\nbuild_mode = "release"\n')
    for path in (HERE / 'helix').glob('*.hx'):
        shutil.copy2(path, project / 'db' / path.name)
    generated = src / 'helix-container' / 'src'
    # CLI compile is the official HelixQL compiler; build --bin in v2.3.5
    # invokes a debug cargo build, so explicitly select upstream's release profile.
    command([str(cli), 'compile', '--path', str(project), '--output', str(generated)])
    changed = subprocess.check_output(['git', '-C', str(src), 'diff', '--name-only'], text=True).splitlines()
    allowed = {'helix-container/src/queries.rs', 'helix-container/src/config.hx.json'}
    if set(changed) - allowed:
        raise RuntimeError(f'Unexpected modified Helix source: {changed}')
    command(['cargo', 'build', '--locked', '--release', '-p', 'helix-container'], cwd=src)
    binary = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', str(src / 'target'))).resolve() / 'release' / 'helix-container'
    inputs = subprocess.check_output(['git', '-C', str(src), 'ls-files', '*.rs', '*.toml', 'Cargo.lock'], text=True).splitlines()
    source_hashes = {name: sha(src / name) for name in inputs}
    evidence = {'source_hashes': source_hashes, 'version': VERSION, 'commit': COMMIT, 'cli_sha256': sha(cli),
                'binary': str(binary), 'binary_sha256': sha(binary),
                'source': str(src), 'cargo_lock_sha256': sha(src / 'Cargo.lock'),
                'queries_sha256': sha(generated / 'queries.rs'),
                'hx_sha256': {p.name: sha(p) for p in (HERE / 'helix').glob('*.hx')},
                'profile': 'upstream release: opt-level=2, codegen-units=1, lto=true, panic=abort'}
    (root / 'installation.json').write_text(json.dumps(evidence, indent=2) + '\n')
    return evidence
