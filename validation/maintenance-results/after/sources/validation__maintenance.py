#!/usr/bin/env python3
"""Run fixed Kiban maintenance trials, then verify the complete reopened state."""
import argparse
import csv
import hashlib
import importlib.util
import json
from pathlib import Path
import random
import subprocess
import shutil

repo = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('compare_campaign', repo/'tools/compare/run.py')
campaign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(campaign)
p = argparse.ArgumentParser()
p.add_argument('--binary-dir', type=Path, required=True, help='immutable copied scale/verifier binaries')
p.add_argument('--prepare-binaries', action='store_true')
p.add_argument('--output-root', type=Path)
p.add_argument('--data-root', type=Path)
p.add_argument('--plan', type=Path, default=repo/'validation/maintenance-plan.json')
a = p.parse_args()
binary_dir = a.binary_dir.resolve()
source_files = [*repo.glob('src/**/*.rs'), *repo.glob('examples/scale/*.rs'),
                repo/'examples/support/random.rs', repo/'examples/maintenance_verify.rs',
                repo/'Cargo.toml', repo/'Cargo.lock', Path(__file__), repo/'tools/compare/run.py']
compiled = {str(f.relative_to(repo)):hashlib.sha256(f.read_bytes()).hexdigest()
            for f in source_files if f.suffix=='.rs' or f.name in ['Cargo.toml','Cargo.lock']}
if a.prepare_binaries:
    subprocess.run(['cargo','build','--release','--example','scale','--example','maintenance_verify'],cwd=repo,check=True)
    binary_dir.mkdir(parents=True,exist_ok=False)
    for name in ['scale','maintenance_verify']:
        shutil.copy2(repo/'target/release/examples'/name,binary_dir/name)
    receipt={'compiled_inputs':compiled,
             'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip(),
             'binary_hashes':{n:hashlib.sha256((binary_dir/n).read_bytes()).hexdigest() for n in ['scale','maintenance_verify']}}
    (binary_dir/'build.json').write_text(json.dumps(receipt,indent=2)+'\n')
    raise SystemExit(0)
if a.output_root is None or a.data_root is None:
    p.error('--output-root and --data-root required for trials')
if a.output_root.resolve().is_relative_to(a.data_root.resolve()) or a.data_root.resolve().is_relative_to(a.output_root.resolve()):
    p.error('output and database roots must be separate')
plan = json.loads(a.plan.read_text())
receipt=json.loads((binary_dir/'build.json').read_text())
binary_hashes = {name: hashlib.sha256((binary_dir/name).read_bytes()).hexdigest()
                 for name in ['scale', 'maintenance_verify']}
if receipt['compiled_inputs']!=compiled or receipt['binary_hashes']!=binary_hashes:
    raise SystemExit('binary/source provenance mismatch; prepare a fresh binary directory')
a.output_root.mkdir(parents=True, exist_ok=False)
a.data_root.mkdir(parents=True, exist_ok=False)
snapshot = a.output_root/'sources'; snapshot.mkdir()
hashes = {}
for source in source_files:
    raw = source.read_bytes(); relative = str(source.relative_to(repo))
    hashes[relative] = hashlib.sha256(raw).hexdigest()
    (snapshot/relative.replace('/', '__')).write_bytes(raw)
def capture(args):
    return subprocess.check_output(args, cwd=repo, text=True).strip()
metadata = {'revision':capture(['git','rev-parse','HEAD']),
            'dirty':capture(['git','status','--porcelain']), 'binary_hashes':binary_hashes,
            'plan':plan, 'source_hashes':hashes, 'rustc':capture(['rustc','-Vv']),
            'os':capture(['uname','-a']), 'cpu':Path('/proc/cpuinfo').read_text(),
            'memory':Path('/proc/meminfo').read_text(),
            'filesystem':capture(['findmnt','-J','-T',str(a.data_root.resolve())]),
            'profile':'root release profile, opt-level 3, no LTO','build':receipt}
(a.output_root/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
jobs = [(c,r) for c in plan['cases'] for r in range(plan['repetitions'])]
random.Random(plan['order_seed']).shuffle(jobs)
results = []
for index,(case,repeat) in enumerate(jobs):
    label = f"{case['name']}-r{repeat}"
    common = ['--db',str(a.data_root.resolve()/label),'--output',str(a.output_root.resolve()/label)]
    for key,value in case['args'].items():
        common += ['--'+key,str(value).lower() if isinstance(value,bool) else str(value)]
    command = [str(binary_dir/'scale'),*common]
    print(f'[{index+1}/{len(jobs)}] {label}',flush=True)
    with (a.output_root/(label+'.log')).open('x') as log:
        result = campaign.supervise(command,log,repo,case['timeout_s'])
    status = a.output_root/label/'status.csv'
    complete = False
    if status.exists():
        with status.open() as status_file:
            complete = any(row['status']=='complete' for row in csv.DictReader(status_file))
    result.update({'label':label,'command':command,'complete':complete})
    if result['exit_code']==0 and complete and result['invalid_reason'] is None:
        verify = [str(binary_dir/'maintenance_verify'),*common]
        with (a.output_root/(label+'-verify.log')).open('x') as log:
            result['verification'] = campaign.supervise(verify,log,repo,120)
        result['verify_command'] = verify
        result['valid'] = result['verification']['exit_code']==0 and result['verification']['invalid_reason'] is None
    else:
        result['valid'] = False
    results.append(result)
    (a.output_root/'runs.json').write_text(json.dumps(results,indent=2)+'\n')
    if not result['valid']:
        raise SystemExit(f'STOP: failed/incomplete trial {label}; no retry or parameter change')
print('All trials completed and reopened state verified.',flush=True)
