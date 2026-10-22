#!/usr/bin/env python3
"""Assert that the QA branch preserves every tracked non-test source byte."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
BASE = '4170b731'
paths = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', BASE, 'src'], cwd=ROOT, text=True).splitlines()
for path in paths:
    before = subprocess.check_output(['git', 'show', f'{BASE}:{path}'], cwd=ROOT)
    after = (ROOT / path).read_bytes()
    if path == 'src/group_commit_tests.rs':
        continue  # This entire module is included only under cfg(test).
    if path == 'src/group_commit.rs':
        assert after.startswith(before), f'changed existing source: {path}'
        added = after[len(before):]
        assert added.startswith(b'\n#[cfg(all(test, not(feature = "loom")))]\nmod qa_tests {')
        assert b'#[cfg(all(test, feature = "loom"))]\nmod qa_loom_tests {' in added
    else:
        assert after == before, f'changed tracked source: {path}'
print(f'PASS: all tracked src/ bytes match {BASE}, except test-only group_commit additions and group_commit_tests.rs.')
