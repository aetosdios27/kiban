#!/usr/bin/env python3
"""Observe only this worktree's ablation children; never changes their state."""
import json, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
BINARY = ROOT/'tools/agentbench/phash/target/release/kiban-phash'
OUT = ROOT/'validation/v2/ablation'
while not json.loads((OUT/'metadata.json').read_text()).get('end'):
    for p in Path('/proc').glob('[0-9]*'):
        try:
            if p.joinpath('exe').resolve() != BINARY: continue
            sample=dict(time=time.time(),pid=int(p.name),title=(OUT/'progress.log').read_text().splitlines()[-1],rollup=p.joinpath('smaps_rollup').read_text())
            with (OUT/'memory.jsonl').open('a') as f: f.write(json.dumps(sample)+'\n')
        except (OSError, ProcessLookupError): pass
    time.sleep(2)
