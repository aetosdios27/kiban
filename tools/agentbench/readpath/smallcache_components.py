#!/usr/bin/env python3
"""Single diagnostic component runs on CPU 6, outside latency trials."""
import hashlib
import json
import os
from pathlib import Path

os.environ['READBENCH_CPU'] = '6'
os.environ['READBENCH_BUILD_CPUS'] = '12,13,14,15,16,17,18,19,20,21'
os.environ['READBENCH_PERF_PAGES'] = '8'

import profile as profiler
import smallcache as campaign


def main():
    folder = campaign.OUT/'final-components'
    folder.mkdir(parents=True, exist_ok=True)
    assert not (folder/'metadata.json').exists()
    profiler.BINARY = campaign.BINS/'ph3'
    profiler.CPU = 6
    (folder/'metadata.json').write_text(json.dumps(dict(
        binary=str(profiler.BINARY), sha256=hashlib.sha256(profiler.BINARY.read_bytes()).hexdigest(),
        cpu=6, operations=1000000, cache_mib=32,
        note='Isolated component costs overlap; do not add them or call them full-read speedups.'
    ), indent=2)+'\n')
    rows = []
    for case in ['bloom', 'index', 'verify-block', 'cache', 'hash', 'block-seek', 'copy', 'crc-value']:
        c = campaign.config('kiban', 'bulk', False, 32)
        c.update(action='micro', micro=case, operations=1000000)
        print(case, flush=True)
        profiler.collect(c, folder, case)
        data = [json.loads(s) for s in (folder/(case+'.log')).read_text().splitlines() if s.startswith('{')]
        result = next(row for row in data if row['type'] == 'micro')
        assert result['errors'] == 0 and result['operations'] == 1000000
        rows.append(result)
    (folder/'summary.json').write_text(json.dumps(rows, indent=2)+'\n')


if __name__ == '__main__':
    main()
