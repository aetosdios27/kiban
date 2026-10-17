#!/usr/bin/env python3
"""Summarize the latency matrix without dropping slow or busy-sibling trials."""
import argparse
import json
from pathlib import Path
import statistics


def spread(values):
    return dict(median=statistics.median(values), minimum=min(values), maximum=max(values), values=values)


def summarize(folder):
    commands = [json.loads(line) for line in (folder/'commands.jsonl').read_text().splitlines()]
    assert len(commands) == 20
    data = {}
    for command in commands:
        config = json.loads((folder/command['config']).read_text())
        parts = command['config'].removesuffix('.config.json').split('-')
        trial, label, engine = parts[3:6]
        rows = [json.loads(line) for line in Path(config['output']).read_text().splitlines()]
        complete, = [r for r in rows if r['type'] == 'complete']
        assert complete['errors'] == 0 and complete['verified_records'] == 1000000
        assert 'Cpus_allowed_list:\t8\n' in complete['proc_status']
        for row in rows:
            if row['type'] == 'stage':
                assert row['errors'] == 0 and row['latency']['count'] == 100000
                data[trial, label, engine, row['workload']] = row
    result = {}
    for work in ['recall-by-id', 'recent-for-agent']:
        row = {}
        for label in ['before', 'after']:
            for engine in ['kiban', 'lmdb']:
                row[f'{label}-{engine}'] = {pct: spread([
                    data[f'r{t}',label,engine,work]['latency'][pct] for t in range(1,6)])
                    for pct in ['p50_ns', 'p99_ns']}
                row[f'{label}-{engine}']['call_ops_s'] = spread([
                    data[f'r{t}',label,engine,work]['call_ops_s'] for t in range(1,6)])
        for pct in ['p50_ns', 'p99_ns']:
            raw, normalized = [], []
            for t in range(1,6):
                def latency(label, engine):
                    return data[f'r{t}',label,engine,work]['latency'][pct]
                ratio = latency('after','kiban')/latency('before','kiban')
                raw.append(ratio)
                normalized.append(ratio*latency('before','lmdb')/latency('after','lmdb'))
            row[f'paired-{pct}'] = spread(raw)
            row[f'normalized-{pct}'] = spread(normalized)
        result[work] = row
    values = [json.loads(line)['busy_fraction']*100 for line in (folder/'sibling.jsonl').read_text().splitlines()]
    assert len(values) == 20
    result['cpu9_busy_percent'] = spread(values)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('folder', type=Path)
    args = parser.parse_args()
    rows = {p.name: summarize(p) for p in sorted(args.folder.iterdir()) if p.is_dir()}
    (args.folder/'summary.json').write_text(json.dumps(rows, indent=2)+'\n')
    print('| Case | Workload | p50 µs baseline → candidate | p99 µs baseline → candidate | Paired p50 [min,max] | Paired p99 [min,max] | LMDB-normalized p50 / p99 |')
    print('|---|---|---:|---:|---:|---:|---:|')
    for case, case_rows in rows.items():
        for work in ['recall-by-id', 'recent-for-agent']:
            r = case_rows[work]
            cells = [case, 'Point' if work == 'recall-by-id' else 'Latest-K']
            for pct in ['p50_ns','p99_ns']:
                cells.append(' → '.join(f'{r[label+"-kiban"][pct]["median"]/1000:.3f}' for label in ['before','after']))
            for pct in ['p50_ns','p99_ns']:
                v = r['paired-'+pct]
                cells.append(f'{v["median"]:.3f} [{v["minimum"]:.3f},{v["maximum"]:.3f}]')
            cells.append(' / '.join(f'{r["normalized-"+pct]["median"]:.3f}' for pct in ['p50_ns','p99_ns']))
            print('| '+' | '.join(cells)+' |')


if __name__ == '__main__':
    main()
