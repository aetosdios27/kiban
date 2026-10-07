#!/usr/bin/env python3
"""Alternate verified frozen engine builds under one explicit workload plan."""
import argparse
import csv
import hashlib
import importlib.util
import json
from pathlib import Path
import random
import subprocess

repo = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('supervisor', repo/'tools/compare/run.py')
supervisor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(supervisor)
p = argparse.ArgumentParser()
p.add_argument('--pair', action='append', required=True, help='label=binary_directory')
p.add_argument('--plan', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--data', type=Path, required=True)
p.add_argument('--perf', type=Path, help='separate diagnostic campaign, not primary rates')
a = p.parse_args()
builds = {}
for pair in a.pair:
    label, directory = pair.split('=', 1)
    if not label.replace('-', '').isalnum() or label in builds:
        p.error('unique alphanumeric build labels required')
    directory = Path(directory).resolve()
    receipt = json.loads((directory/'build.json').read_text())
    for name, digest in receipt['binary_hashes'].items():
        if hashlib.sha256((directory/name).read_bytes()).hexdigest() != digest:
            raise SystemExit('frozen binary hash mismatch')
    for path, digest in receipt['compiled_inputs'].items():
        raw = subprocess.check_output(['git','show',f"{receipt['revision']}:{path}"],cwd=repo)
        if hashlib.sha256(raw).hexdigest() != digest:
            raise SystemExit(f'committed source mismatch: {path}')
    builds[label] = {'directory':str(directory),'receipt':receipt}
if a.output.resolve().is_relative_to(a.data.resolve()) or a.data.resolve().is_relative_to(a.output.resolve()):
    p.error('output and data directories must be separate')
a.output.mkdir(parents=True,exist_ok=False)
a.data.mkdir(parents=True,exist_ok=False)
plan = json.loads(a.plan.read_text())
metadata = {'plan':plan,'builds':builds,'profiling':a.perf is not None,
            'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip(),
            'rustc':subprocess.check_output(['rustc','-Vv'],text=True),
            'uname':subprocess.check_output(['uname','-a'],text=True),
            'cpu':Path('/proc/cpuinfo').read_text(),'memory':Path('/proc/meminfo').read_text(),
            'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
(a.output/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
jobs = [(c,r) for c in plan['cases'] for r in range(plan['repetitions'])]
random.Random(plan['order_seed']).shuffle(jobs)
results = []
labels = list(builds)
for index,(case,repeat) in enumerate(jobs):
    order = labels if index % 2 == 0 else labels[::-1]
    for label in order:
        name = f"{label}-{case['name']}-r{repeat}"
        args = ['--db',str(a.data.resolve()/name),'--output',str(a.output.resolve()/name)]
        for key,value in case['args'].items():
            args += ['--'+key,str(value).lower() if isinstance(value,bool) else str(value)]
        command = [str(Path(builds[label]['directory'])/'scale'),*args]
        if a.perf:
            command = [str(a.perf.resolve()),'record','-q','-e','cpu-clock:u','-F','99',
                       '--call-graph','dwarf,16384','-o',str(a.output.resolve()/(name+'.perf')),'--',*command]
        print(name,flush=True)
        with (a.output/(name+'.log')).open('x') as log:
            result = supervisor.supervise(command,log,repo,case['timeout_s'])
        status = a.output/name/'status.csv'
        complete = status.exists() and any(r['status']=='complete' for r in csv.DictReader(status.open()))
        result.update({'label':label,'case':case['name'],'repeat':repeat,'directory':name,
                       'command':command,'complete':complete,'valid':False})
        if result['exit_code']==0 and result['invalid_reason'] is None and complete:
            verify = [str(Path(builds[label]['directory'])/'maintenance_verify'),*args]
            with (a.output/(name+'-verify.log')).open('x') as log:
                result['verification'] = supervisor.supervise(verify,log,repo,case.get('verify_timeout_s',120))
            result['verify_command'] = verify
            result['valid'] = result['verification']['exit_code']==0 and result['verification']['invalid_reason'] is None
        results.append(result)
        (a.output/'runs.json').write_text(json.dumps(results,indent=2)+'\n')
        if not result['valid']:
            raise SystemExit(f'STOP: {name}; no retry or parameter change')
print('All paired runs completed and reopened-state verified.',flush=True)
