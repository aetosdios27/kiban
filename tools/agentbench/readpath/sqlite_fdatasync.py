#!/usr/bin/env python3
"""Five rotated durable trials: compare bundled default versus Linux fdatasync.
Build the alternate binary first, with no timed run active:
LIBSQLITE3_FLAGS=-DHAVE_FDATASYNC=1 CARGO_TARGET_DIR=target/sqlite/fdatasync-target \
  taskset -c 12-21 nice -n 10 cargo build --release --locked \
  --manifest-path tools/agentbench/readpath/Cargo.toml
"""
import json
import argparse
from pathlib import Path
import sqlite_bench as bench

original = bench.BINARY
alternate = bench.ROOT/'target/sqlite/fdatasync-target/release/kiban-readbench'
parser=argparse.ArgumentParser(__doc__)
parser.add_argument('--raw',type=Path,default=bench.ROOT/'validation/sqlite/raw')
args=parser.parse_args()
out = args.raw/'fdatasync'
selected_layout=json.loads((args.raw/'selection.json').read_text())['write']
out.mkdir(parents=True, exist_ok=True)
metadata=dict(default_binary=str(original), default_sha256=bench.digest(original),
              tuned_binary=str(alternate),tuned_sha256=bench.digest(alternate),
              flags='LIBSQLITE3_FLAGS=-DHAVE_FDATASYNC=1',cpu=10,trials=5)
path=out/'metadata.json'
if path.exists(): assert json.loads(path.read_text())==metadata
else: path.write_text(json.dumps(metadata,indent=2)+'\n')
configs=[('kiban-index0',dict(bench.base('kiban',10000),point_index=False),original),
         ('kiban-index1',dict(bench.base('kiban',10000),point_index=True),original),
         ('sqlite-full',dict(bench.base('sqlite',10000),sqlite_sync='FULL',sqlite_layout=selected_layout),original),
         ('sqlite-full-fdatasync',dict(bench.base('sqlite',10000),sqlite_sync='FULL',sqlite_layout=selected_layout),alternate),
         ('lmdb',bench.base('lmdb',10000),original)]
for trial in range(5):
    for label,c,binary in bench.rotate(configs,trial):
        c.update(action='write',durable=True,operations=1000,db=str(bench.DATA/'write-trial'))
        bench.BINARY=binary
        bench.run(c,out/'write-10000-bulk-None-durable',f'r{trial+1}-{label}',True)
bench.BINARY=original
