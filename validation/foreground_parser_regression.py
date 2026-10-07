#!/usr/bin/env python3
"""Prove malformed-range tests fail with old arithmetic; always restore sources."""
from pathlib import Path
import hashlib
import json
import subprocess

repo = Path(__file__).resolve().parent.parent
out = repo/'target/foreground-checks/parser-regression'
out.mkdir(parents=True,exist_ok=False)
files = [repo/'src/sstable/block.rs',repo/'src/sstable/reader.rs']
original = {p:p.read_text() for p in files}
block = original[files[0]]
start = block.index('        let restart_start = num_restarts')
end = block.index('\n        let entries_end',start)
block = block[:start]+'        let restart_start = raw.len() - 9 - num_restarts * 4;'+block[end:]
reader = original[files[1]]
reader = reader.replace('index_offset.checked_add(index_len).is_none_or(|end| end > data_end)',
                        'index_offset > data_end || index_offset + index_len > data_end')
# cargo fmt may wrap the condition. Match the formatted multi-line form too.
reader = reader.replace('index_offset\n                .checked_add(index_len)\n                .is_none_or(|end| end > data_end)',
                        '(index_offset > data_end || index_offset + index_len > data_end)')
reader = reader.replace('filter_offset.checked_add(filter_len) != Some(index_offset)',
                        'filter_offset + filter_len != index_offset')
if reader == original[files[1]] or '.checked_add(index_len)' in reader:
    raise SystemExit('old arithmetic substitution failed; inspect formatted code')
command = ['cargo','test','--lib','return_corruption','--','--nocapture']
try:
    files[0].write_text(block);files[1].write_text(reader)
    result = subprocess.run(command,cwd=repo,capture_output=True,text=True)
    log = result.stdout+result.stderr
    (out/'old.log').write_text(log)
    if result.returncode==0 or '2 failed' not in log or 'overflow' not in log:
        raise SystemExit('old implementation did not exhibit both expected failures')
finally:
    for path,text in original.items():path.write_text(text)
result = subprocess.run(command,cwd=repo,capture_output=True,text=True)
(out/'fixed.log').write_text(result.stdout+result.stderr)
if result.returncode != 0 or '2 passed' not in result.stdout:
    raise SystemExit('fixed parser regressions failed')
(out/'receipt.json').write_text(json.dumps({'command':command,'old_expected_failure':True,
    'fixed_exit_code':result.returncode,'fixed_sources':{str(p.relative_to(repo)):hashlib.sha256(p.read_bytes()).hexdigest() for p in files}},indent=2)+'\n')
print('Both tests fail with old arithmetic and pass with fixed bounds; sources restored.')
