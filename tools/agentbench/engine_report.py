"""Tables from complete engine trials; no pooling, missing-trial substitution or error rows."""
import json
import pathlib
import sys

from report import spread

WORKLOADS = ('remember', 'recall-by-id', 'recent-for-agent', 'update', 'mixed-90-10')
ENGINES = ('kiban', 'helix', 'lmdb')


def generate(root):
    root = pathlib.Path(root)
    status = json.loads((root/'status.json').read_text())
    lines = ['# In-process engine head-to-head', '']
    if status['status'] != 'complete' or status['errors']:
        lines += ['Campaign incomplete/invalid; no comparison table.', '',status.get('error','still running')]
    else:
        metadata = json.loads((root/'metadata.json').read_text())
        label = 'PRELIMINARY — at least one per-trial median load gate or coverage check failed' if status['preliminary'] else 'Measured on a shared desktop, pinned cores'
        lines += [f'**{label}.**', '',
            f"Benchmark CPUs: `{metadata['parameters']['cpus']}`; driver CPUs: `{metadata['parameters']['driver_cpus']}`. "
            'These affinities do not reserve the cores exclusively.', '',
            'Cells are median [minimum, maximum] of per-trial metrics (five small-N trials, three large-N trials). Latencies are µs. '
            'The median 1-minute load must be <3 in every trial; maxima are reported, not gated.', '']
        groups = {}
        for run in status['runs']:
            rows = [json.loads(line) for line in (root/run['file']).read_text().splitlines()]
            assert rows[-1]['type'] == 'complete' and rows[-1]['verified_records'] == run['final_population']
            assert json.loads((root/(run['name']+'-reopen.jsonl')).read_text()) == {'type':'reopen_verified','verified_records':run['final_population']}
            stages = [row for row in rows if row['type'] == 'stage']
            assert [row['workload'] for row in stages] == list(WORKLOADS)
            for row in stages:
                assert row['errors'] == 0
                groups.setdefault((run['n'],run['mode'],run['backend'],row['workload']),[]).append(row)
        for n in metadata['parameters']['sizes']:
            for mode in metadata['parameters']['modes']:
                runs = [r for r in status['runs'] if r['n'] == n and r['mode'] == mode]
                repetitions = metadata['parameters']['large_runs'] if n > 10000 else metadata['parameters']['runs']
                assert len(runs) == 3*repetitions
                lines += [f'## N={n:,}, {mode}', '',
                    f"{'Preliminary: load/coverage gate failed.' if any(r['preliminary'] for r in runs) else 'Measured on a shared desktop, pinned cores.'} "
                    f"Untimed verified/reopened base: {n:,} records. W1 inserts {runs[0]['write_operations']:,} fresh records; "
                    f"W2/W3/W5: {runs[0]['operations']:,} calls each; W4: {runs[0]['write_operations']:,} overwrites. "
                    f"Final population: {runs[0]['final_population']:,}. {repetitions} alternating trials per engine.", '',
                    '| Engine | Workload | p50 µs | p99 µs | p999 µs | max µs | Verified ops/s | Call-only ops/s |',
                    '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |']
                for workload in WORKLOADS:
                    for backend in ENGINES:
                        rows = groups[n,mode,backend,workload]
                        assert len(rows) == repetitions
                        metrics = [spread([r['latency'][m] for r in rows],1000) for m in ('p50_ns','p99_ns','p999_ns','max_ns')]
                        metrics += [spread([r[m] for r in rows]) for m in ('verified_ops_s','call_ops_s')]
                        lines.append(f'| {backend} | {workload} | '+' | '.join(metrics)+' |')
                lines += ['', '| Engine | End RSS MiB | Peak RSS MiB | Disk apparent MiB | Disk allocated MiB |',
                    '| --- | ---: | ---: | ---: | ---: |']
                for backend in ENGINES:
                    own = [r for r in runs if r['backend'] == backend]
                    resources = [json.loads((root/(r['name']+'.resources.json')).read_text()) for r in own]
                    metrics = [spread([r[m] for r in resources],1024**2) for m in ('rss_bytes','peak_rss_bytes')]
                    metrics += [spread([r['disk'][m] for r in own],1024**2) for m in ('apparent_bytes','allocated_bytes')]
                    lines.append(f'| {backend} | '+' | '.join(metrics)+' |')
                lines.append('')
        lines += ['## Load during every trial', '',
            'Sampled every 200 ms plus actual engine-side boundary samples; median [minimum, maximum]. Warmup is excluded using engine-recorded '
            'timestamps; final sync and full in-process verification are included. Engine open is excluded. '
            'The separate reopen process is excluded. All three load windows are retained.', '',
            '| N | Mode | Trial | Engine | 1-minute load | 5-minute load | 15-minute load | Samples / max gap | Gate |',
            '| ---: | --- | ---: | --- | ---: | ---: | ---: | --- | --- |']
        for run in status['runs']:
            load = run['load_summary']
            cells = [f"{v['median']:.2f} [{v['min']:.2f}, {v['max']:.2f}]" if v else 'unknown' for v in load['load']]
            lines.append(f"| {run['n']:,} | {run['mode']} | {run['trial']} | {run['backend']} | "+' | '.join(cells)+
                f" | {load['samples']} / {load['largest_sample_gap_seconds']:.2f}s | {'preliminary' if run['preliminary'] else 'pass'} |")
        lines += ['', '## What these numbers include', '',
            'All three paths execute in the same release binary, with the same MiMalloc allocator and '
            'compiler profile. There is no HTTP, router, socket, request parsing or response JSON transport. '
            'Helix executes the generated storage/traversal statements with typed inputs/results; '
            '`engine/equivalence.json` retains the exact compiler output used for extraction. '
            'Its graph layout, required indexes, UUID generation, arena, serialization and per-query transaction remain included. '
            'Unused automatic BM25 indexing is explicitly disabled for this study; the HTTP study left it enabled. '
            'Kiban and raw LMDB use the same primary key and JSON storage codec; this JSON is persistent data encoding, not transport.', '',
            '`durable`: Kiban put + sync (WAL sync_data/fdatasync) for every write; Helix and raw LMDB commit '
            'each write transaction with neither NO_SYNC nor NO_META_SYNC. '
            '`fast`: Kiban put without per-write sync; Helix and raw LMDB enable NO_SYNC | NO_META_SYNC '
            'on their exclusively owned environment through heed3 before foreground calls. '
            'Kiban maintenance may still sync. Actual LMDB flag bits are recorded for every process. '
            'All backends explicitly synchronize once after timed work, before full verification and orderly close. '
            'Reopen checks do not establish power-loss durability for fast mode.', '',
            'Every timed value written is immediately read back and checked field-for-field, outside write latency. '
            'All read/list outputs are checked. The complete population and all latest lists are checked '
            'again before close and in a new process after reopen. Any error invalidates the campaign. '
            'Warmup uses a separate discarded database in the same process, outside timing. '
            'The base dataset is loaded untimed: Kiban WriteBatch chunks of 10,000, Helix/LMDB one large transaction, '
            'with relaxed per-call durability and explicit final sync. Every base record is verified before close and again '
            'after reopening in the selected mode. Timed inserts use fresh IDs after that base; all later reads use '
            'the resulting population. Durable W1/W4 are capped at 10,000 calls; W5 retains its full 90/10 count.', '',
            'Verified throughput includes generation, oracle comparisons and separate write read-backs. '
            'Call-only throughput divides call count by the sum of foreground API durations. '
            'Final sync/flush, full verification and reopen checks are excluded from both foreground metrics. '
            'p999 requires at least 10,000 samples. Histogram percentile bounds have <3.125% relative bucket '
            'error above 32 ns; maximum is exact.', '',
            'RSS includes the entire benchmark process: engine, bulk preload, oracle revision vector, allocator and mapped resident pages. '
            'It is not isolated engine heap usage; kernel page cache outside the process is not counted. '
            'Disk sizes are sampled after orderly process exit and before reopen. Apparent bytes and '
            'st_blocks×512 allocated bytes differ on this compressed Btrfs filesystem. '
            'The larger dataset exceeds Kiban’s default 4 MiB memtable and 32 MiB block cache, '
            'but can still fit in host RAM. This is not an out-of-RAM or cold-cache campaign.', '']
    (root/'REPORT.md').write_text('\n'.join(lines).rstrip()+'\n')


if __name__ == '__main__':
    generate(pathlib.Path(sys.argv[1]))
