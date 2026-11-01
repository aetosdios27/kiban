#!/usr/bin/env python3
"""Project suffix bytes for alternate anchors, keeping existing SST partitions.

This does not write a new format or measure latency. PH3 flag zero stays frozen.
"""
import json
from pathlib import Path
import zlib

import run


def common(a, b):
    for i, (left, right) in enumerate(zip(a, b)):
        if left != right:
            return i
    return min(len(a), len(b))


def main():
    rows = []
    for layout in ['bulk', 'normal']:
        control = sum(p.stat().st_size for p in (run.ROOT/'target/pointindex-cost/data/off'/f'kiban-{layout}').glob('*.sst'))
        sst_bytes = 0
        point_bytes = {group: 0 for group in [1024, 512, 256]}
        for path in (run.ROOT/'target/pointindex-cost/data/after'/f'kiban-{layout}').glob('*.sst'):
            sst_bytes += path.stat().st_size
            with path.open('rb') as stream:
                stream.seek(-68, 2)
                footer = stream.read()
                stream.seek(int.from_bytes(footer[:8], 'little'))
                raw = stream.read(int.from_bytes(footer[8:16], 'little'))
            def uint(at, width):
                return int.from_bytes(raw[at:at+width], 'little')
            assert raw[:8] == b'KIBANPH3' and uint(52,4) == 0
            assert zlib.crc32(raw[:-4]) == uint(len(raw)-4, 4)
            address, key, count, seq, value = [uint(p,4) for p in [32,36,40,44,48]]
            arena = uint(24,8)
            pos = arena
            keys, old_suffix = [], 0
            for _ in range(uint(16,4)):
                anchor = arena + uint(pos,address)
                shared = uint(pos+address,key)
                suffix = uint(pos+address+key,key)
                versions = uint(pos+address+2*key,count)
                pos += address+2*key+count
                keys.append(raw[anchor:anchor+shared]+raw[pos:pos+suffix])
                old_suffix += suffix
                pos += suffix+versions*(seq+value+address)
            assert pos == len(raw)-5
            for group in point_bytes:
                suffix = sum(len(k)-common(keys[i//group*group],k) if i % group else len(k)
                             for i,k in enumerate(keys))
                if group == 1024:
                    assert suffix == old_suffix
                point_bytes[group] += len(raw)-old_suffix+suffix
        current_point = point_bytes[1024]
        for group, size in point_bytes.items():
            total = sst_bytes-current_point+size
            rows.append(dict(layout=layout,group=group,point_bytes=size,point_bytes_per_key=size/1000000,
                             projected_sst_bytes=total,overhead_percent=100*(total/control-1),latency_measured=False))
    print(json.dumps(rows,indent=2))


if __name__ == '__main__':
    main()
