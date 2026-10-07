#!/usr/bin/env python3
"""Build and freeze binaries, with committed inputs and explicit profile flags."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

repo = Path(__file__).resolve().parent.parent
p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
p.add_argument('--diagnostics', action='store_true')
p.add_argument('--debug', action='store_true')
a = p.parse_args()
revision = subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip()
files = [*repo.glob('src/**/*.rs'),*repo.glob('examples/scale/*.rs'),
         repo/'examples/support/random.rs',repo/'examples/maintenance_verify.rs',
         repo/'Cargo.toml',repo/'Cargo.lock']
inputs = {}
for path in files:
    relative = str(path.relative_to(repo))
    raw = path.read_bytes()
    committed = subprocess.check_output(['git','show',f'{revision}:{relative}'],cwd=repo)
    if raw != committed:
        raise SystemExit(f'commit compiled inputs before freezing: {relative}')
    inputs[relative] = hashlib.sha256(raw).hexdigest()
a.output.mkdir(parents=True,exist_ok=False)
command = ['cargo','build','--release','--locked','--example','scale','--example','maintenance_verify']
if a.diagnostics:
    command += ['--features','diagnostics']
environment = os.environ.copy()
if a.debug:
    environment['CARGO_PROFILE_RELEASE_DEBUG'] = '1'
else:
    environment.pop('CARGO_PROFILE_RELEASE_DEBUG',None)
subprocess.run(command,cwd=repo,env=environment,check=True)
hashes = {}
for name in ['scale','maintenance_verify']:
    shutil.copy2(repo/'target/release/examples'/name,a.output/name)
    hashes[name] = hashlib.sha256((a.output/name).read_bytes()).hexdigest()
receipt = {'revision':revision,'compiled_inputs':inputs,'binary_hashes':hashes,
           'command':command,'features':['diagnostics'] if a.diagnostics else [],
           'profile':'release opt-level3, no LTO','debug':a.debug,
           'rustc':subprocess.check_output(['rustc','-Vv'],text=True)}
(a.output/'build.json').write_text(json.dumps(receipt,indent=2)+'\n')
