#!/usr/bin/env python3
"""Archive native OPTIONS bytes for version control without normalizing evidence."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('roots', type=Path, nargs='+')
a = p.parse_args()
for root in a.roots:
    manifest = root / 'options-archives.json'
    if manifest.exists():
        raise SystemExit(f'archive manifest already exists: {manifest}')
    records = []
    for source in sorted(root.glob('*/OPTIONS-*')):
        if source.suffix == '.gz':
            continue
        raw = source.read_bytes()
        archive = source.with_name(source.name + '.gz')
        with archive.open('xb') as output:
            output.write(gzip.compress(raw, mtime=0))
        if gzip.decompress(archive.read_bytes()) != raw:
            raise SystemExit(f'archive verification failed: {archive}')
        records.append({'original': str(source.relative_to(root)),
                        'archive': str(archive.relative_to(root)),
                        'original_sha256': hashlib.sha256(raw).hexdigest(),
                        'original_bytes': len(raw)})
        source.unlink()
    manifest.write_text(json.dumps(records, indent=2) + '\n')
