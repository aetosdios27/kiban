"""Regenerate tables from raw records, never pool trials or hide failures."""
import json
import datetime
import pathlib
import statistics
import sys


def spread(values, scale=1):
    if not values or any(v is None for v in values):
        return '—'
    values = [v / scale for v in values]
    return f'{statistics.median(values):,.2f} [{min(values):,.2f}, {max(values):,.2f}]'


def load_summary(root, runs):
    samples = [json.loads(line) for line in (root / 'load.jsonl').read_text().splitlines()]
    for sample in samples:
        sample['epoch'] = datetime.datetime.fromisoformat(sample['time']).timestamp()
    commands = [json.loads(line) for line in (root / 'commands.jsonl').read_text().splitlines()]
    summaries = []
    for run in runs:
        config = pathlib.Path(run['file']).stem + '.config.json'
        command = next(c for c in commands if pathlib.Path(c['argv'][-1]).name == config)
        start, end = command['time'], (root / run['file']).stat().st_mtime
        observed = [x for x in samples if start <= x['epoch'] <= end and x['measuring']]
        times = [start] + [x['epoch'] for x in observed] + [end]
        gap = max(b - a for a, b in zip(times, times[1:]))
        summaries.append(run | {'start_epoch': start, 'end_epoch': end, 'samples': len(observed),
            'duration_seconds': end - start, 'largest_sample_gap_seconds': gap,
            'coverage': 'partial' if gap > 2.5 else 'sampled throughout',
            'load': [{'median': statistics.median([x['load'][i] for x in observed]),
                      'min': min(x['load'][i] for x in observed),
                      'max': max(x['load'][i] for x in observed)} if observed else None for i in range(3)]})
    (root / 'per-run-load.json').write_text(json.dumps(summaries, indent=2) + '\n')
    return summaries


def generate(root):
    root = pathlib.Path(root)
    status = json.loads((root / 'status.json').read_text())
    lines = ['# Agent-memory benchmark', '',
             'Status: **' + ('PRELIMINARY — loaded host' if status.get('preliminary') else status['status'].upper()) + '**.', '']
    if status['status'] != 'complete':
        lines += ['Campaign incomplete/invalid; no measurements qualify for comparison.', '',
                  'Reason: ' + status.get('error', 'still running'), '']
    else:
        window = "1-minute" if status.get("load_window") == "1m" else "1/5/15-minute"
        lines += [f"Maximum sampled {window} load during measurements: {status['max_measured_load']:.2f}; threshold <2.", '',
                  'Cells are median [minimum, maximum] of per-run metrics; percentiles are not pooled. Latencies are µs.', '',
                  '| Path | Kiban embedded | Kiban server (placeholder) | Helix client | Helix transport floor | Hydra hosted |',
                  '| --- | --- | --- | --- | --- | --- |',
                  '| Transport | in-process | **not implemented** | HTTP/1.1 loopback | same HTTP server, Noop | skipped |', '',
                  '| System | Workload | Runs | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |',
                  '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
        groups = {}
        for run in status['runs']:
            rows = [json.loads(line) for line in (root / run['file']).read_text().splitlines()]
            if rows[-1]['type'] != 'complete':
                raise ValueError('Run missing completion marker')
            for row in rows:
                if row['type'] == 'stage':
                    if row['errors']:
                        raise ValueError('Run has errors')
                    groups.setdefault((run['backend'], row['workload']), []).append(row)
        for (backend, workload), rows in sorted(groups.items()):
            if len(rows) < 5:
                raise ValueError('Fewer than five runs')
            metrics = [spread([r['latency'][metric] for r in rows], 1000)
                       for metric in ['p50_ns', 'p99_ns', 'p999_ns', 'max_ns']]
            metrics += [spread([r[key] for r in rows]) for key in ['verified_ops_s', 'call_ops_s']]
            lines.append(f'| {backend} | {workload} | {len(rows)} | ' + ' | '.join(metrics) + ' |')
        summaries = load_summary(root, status['runs'])
        lines += ['', 'Observed load during each run (per-second samples; median [min, max]). '
                  'The interval includes client setup and final verification. A sampling gap is explicitly marked; missing values are not inferred.', '',
                  '| Trial | Path | 1-minute load | 5-minute load | 15-minute load | Samples / largest gap | Coverage |',
                  '| ---: | --- | ---: | ---: | ---: | --- | --- |']
        for run in summaries:
            cells = [f"{v['median']:.2f} [{v['min']:.2f}, {v['max']:.2f}]" if v else 'unknown' for v in run['load']]
            lines.append(f"| {run['trial']} | {run['backend']} | " + ' | '.join(cells) +
                         f" | {run['samples']} / {run['largest_sample_gap_seconds']:.1f}s | {run['coverage']} |")
        if (root / 'resumptions').exists():
            lines += ['', 'The external session interruption stopped the original supervisor. Completed units were retained; '
                      'the resumed driver verified unchanged benchmark binaries/sources and performed missing reopen checks. '
                      'Original metadata/source snapshots remain intact; resumption commands and driver snapshots are in resumptions/. '
                      'The second Kiban run finished after load sampling stopped, so its observed load range has partial coverage.', '']
        lines += ['', 'Durability: Kiban put+sync per write; Helix v2.3.5 LMDB default synchronous commit. '
                  'No power-loss test was run. No-op has no durable writes.', '',
                  '`remember` and `update` both acknowledge durable writes per call. Kiban performs `put` then `sync` inside the timer; '
                  'the WAL flushes its userspace buffer and calls `sync_data` (`fdatasync` on Linux), covering active and any frozen WAL. '
                  'Helix commits a default synchronous LMDB write transaction before returning its HTTP acknowledgement: dirty data pages are synchronized and '
                  'the metadata commit is written through a synchronous file descriptor. Neither `MDB_NOSYNC` nor `MDB_NOMETASYNC` is enabled. '
                  'Thus both paths pay per-call persistence work, without cross-call batching; this does not imply one identical `fsync` syscall per call. '
                  'Helix remember also creates an agent edge. Separate read-back verification is outside write latency.', '',
                  'Every memory write was immediately read back and compared field-for-field; every foreground read/list was checked. '
                  'Every final record and each agent’s latest list was also verified after process/server restart. '
                  'Any error invalidates the campaign. Warmup is a separate, discarded database per backend/trial.', '',
                  'Latency includes each foreground API call and serialization/deserialization. Separate write read-backs are excluded from latency. '
                  'Verified throughput includes read-backs, generators, oracle comparisons and timer accounting; '
                  'call-only throughput is count / sum of foreground call durations and excludes that harness work. '
                  'Full post-run/reopen checks and sync/flush are outside both foreground metrics; their time is retained in JSONL.', '',
                  'p999 is unavailable below 10,000 samples. Histogram percentile upper bounds have <3.125% relative bucket error above 32 ns. '
                  'Max is exact. Per-kind read/write distributions for mixed traffic are retained in raw JSONL.', '',
                  'These client-path measurements do not establish an engine-level winner, causal bottleneck or adoption ranking. '
                  'These paths have different costs: embedded calls versus JSON/HTTP, and KV ordering versus graph adjacency plus sort. '
                  'The Noop row includes HTTP, routing and an empty read transaction, so it is a transport floor proxy, '
                  'not a pure wire measurement. Do not subtract percentiles or infer engine speedups from these rows.', '']
    (root / 'REPORT.md').write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    generate(sys.argv[1])
