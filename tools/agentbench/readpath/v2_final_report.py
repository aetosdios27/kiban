#!/usr/bin/env python3
"""Recheck final artifacts and generate the review tables without timing code."""
import argparse
import hashlib
import json
import re
import statistics
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'validation/v2'
median = statistics.median
p = argparse.ArgumentParser()
p.add_argument('--verdict', required=True)
p.add_argument('--reason', required=True)
a = p.parse_args()

def read(path):
    return json.loads(path.read_text())

def records(path):
    return [json.loads(line) for line in path.read_text().splitlines()]

def dist(values, scale=1):
    return f'{median(values)/scale:.3f} [{min(values)/scale:.3f}, {max(values)/scale:.3f}]'

def ratio(after, before):
    return [x/y for x, y in zip(after, before)]

def table(header):
    return ['| ' + ' | '.join(header) + ' |', '|' + '|'.join('---' for _ in header) + '|']

checks = read(OUT / 'final-checks/status.json')
assert len(checks) == 11 and all(r['code'] == 0 for r in checks)
assert not read(OUT / 'final-checks/metadata.json')['quick']
for name, digest in read(OUT / 'final-checks/source-hashes.json').items():
    assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest
provenance = read(OUT / 'measurement-provenance.json')
assert provenance['engine_sources'] == read(OUT / 'final-checks/source-hashes.json')
for name, digest in provenance['harness_sources'].items():
    assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest
for row in provenance['binaries'].values():
    assert hashlib.sha256(Path(row['path']).read_bytes()).hexdigest() == row['sha256']
rows = []
summary = {'verdict': a.verdict, 'reason': a.reason, 'reads': [], 'ranges': [], 'maintenance': [], 'writes': [], 'rss': []}
lines = ['# Kiban v2 locator-index validation', '', f'Verdict: {a.verdict}. {a.reason}', '',
    'Main is `8bbb4934f8d102ed0285c813a3770831b21c63e5`, extracted with `git archive` and built without engine edits. The final engine uses full-key MPHF records with locators into sorted data. Values appear once on disk. Both builds keep main\'s RAM-sized default cache. The point-index write option remains opt-in.', '',
    'All latency tables show microseconds. Entries are medians across five alternating before/after trials. Each read/write candidate has its own following LMDB control. Ratios divide Kiban by that control; lower is better. These are warm, single-thread public operations on CPU 8. Builds and correctness tests run separately on CPUs 2,3,4,5,6,7,10,11 at nice 10. CPU 9 is never assigned. The independent crash sweep on CPUs 12–21 remains untouched.', '',
    '## Point reads and latest-K', '',
    'Each stage has 100,000 timed calls. Every call checks returned payloads after timing. Every command verifies the full 1M-record database before and after timing. Point calls include key formatting, value copying and JSON decoding. Latest-K uses the bounded range API and returns ten decoded records for one agent.', '']
lines += table(['Layout/cache', 'Operation', 'Main p50/p99', 'Final p50/p99', 'LMDB final p50/p99', 'Paired p50 K/LMDB main → final', 'Paired p99 K/LMDB main → final'])
scan_lines = table(['Layout/cache', 'Raw final/main p50 [range]', 'Control-normalized final/main p50 [range]', 'Raw final/main p99 [range]', 'Control-normalized final/main p99 [range]'])
rss_lines = table(['Layout/cache', 'Main peak RSS MiB [range]', 'Final peak RSS MiB [range]'])
for layout in ['bulk', 'normal']:
    for cache in ['512', '32', 'default']:
        folder = OUT/'reads'/f'{layout}-{cache}'
        commands = records(folder/'commands.jsonl')
        assert len(commands) == 20 and all(c['affinity'] == '8' for c in commands)
        stages = {}
        peaks = {}
        for trial in range(1, 6):
            for label in ['before', 'after']:
                for engine in ['kiban', 'lmdb']:
                    data = records(folder/f'1000000-{layout}-{cache}-r{trial}-{label}-{engine}.jsonl')
                    complete = next(r for r in data if r['type'] == 'complete')
                    assert complete['errors'] == 0 and complete['verified_records'] == 1_000_000
                    assert 'Cpus_allowed_list:\t8\n' in complete['proc_status']
                    peaks[label, engine, trial] = int(re.search(r'^VmHWM:\s+(\d+)', complete['proc_status'], re.M)[1])/1024
                    for r in data:
                        if r['type'] == 'stage':
                            assert r['errors'] == 0 and r['latency']['count'] == 100000
                            stages[r['workload'], label, engine, trial] = r['latency']
        summary['rss'].append(dict(layout=layout, cache=cache, main_mib=[peaks['before', 'kiban', t] for t in range(1, 6)], final_mib=[peaks['after', 'kiban', t] for t in range(1, 6)]))
        rss_lines.append(f'| {layout}/{cache} | ' + ' | '.join(dist([peaks[l, 'kiban', t] for t in range(1, 6)]) for l in ['before', 'after']) + ' |')
        for work in ['recall-by-id', 'recent-for-agent']:
            d = {(l, e, pct): [stages[work, l, e, t][pct+'_ns'] for t in range(1, 6)] for l in ['before', 'after'] for e in ['kiban', 'lmdb'] for pct in ['p50', 'p99']}
            k_l = {(l, pct): ratio(d[l, 'kiban', pct], d[l, 'lmdb', pct]) for l in ['before', 'after'] for pct in ['p50', 'p99']}
            pair = lambda l, e: '/'.join(f'{median(d[l, e, pct])/1000:.3f}' for pct in ['p50', 'p99'])
            cells = [' → '.join(f'{median(k_l[l, pct]):.3f}' for l in ['before', 'after']) for pct in ['p50', 'p99']]
            lines.append(f'| {layout}/{cache} | {work} | {pair("before", "kiban")} | {pair("after", "kiban")} | {pair("after", "lmdb")} | ' + ' | '.join(cells) + ' |')
            summary['reads'].append(dict(layout=layout, cache=cache, workload=work, latency={f'{l}-{e}-{pct}': v for (l, e, pct), v in d.items()}, ratios={f'{l}-{pct}': v for (l, pct), v in k_l.items()}))
            if work == 'recent-for-agent':
                cells=[]
                for pct in ['p50', 'p99']:
                    cells += [dist(ratio(d['after', 'kiban', pct], d['before', 'kiban', pct])), dist(ratio(k_l['after', pct], k_l['before', pct]))]
                scan_lines.append(f'| {layout}/{cache} | ' + ' | '.join(cells) + ' |')
lines += ['', 'See [READS.md](READS.md) and [read-summary.json](read-summary.json) for every trial\'s point and latest-K latency range.', '', '## Scan gate', '',
    'Ratios below pair trial numbers. The normalized ratio divides final Kiban/LMDB by main Kiban/LMDB. Raw ratios retain the public API latency; controls expose drift. Ratios are computed within each trial before taking medians, so dividing displayed medians need not reproduce paired changes. Range probes use one identical source linked separately against each engine library. They start at varying keys within each agent and check every returned key and complete payload. Range samples exclude verification and include iterator construction and value copying.', '']
lines += scan_lines + ['']
range_lines = table(['Layout/cache/count', 'Main p50 µs [range]', 'Final p50 µs [range]', 'Paired final/main p50 [range]', 'Paired final/main p99 [range]'])
cache_bytes = {}
for layout in ['bulk', 'normal']:
    for cache in ['512', 'default']:
        folder = OUT/'ranges'/f'{layout}-{cache}'
        assert len(records(folder/'commands.jsonl')) == 10
        for count in [10, 256]:
            d = {}
            for label in ['before', 'after']:
                values = [next(r for r in records(folder/f'{layout}-{cache}-r{t}-{label}.jsonl') if r['count'] == count) for t in range(1, 6)]
                assert all(r['errors'] == 0 and r['operations'] == 1000 and r['verified_records'] == 1000*count for r in values)
                assert all(r['cache_bytes'] == values[0]['cache_bytes'] for r in values)
                if cache in cache_bytes: assert cache_bytes[cache] == values[0]['cache_bytes']
                cache_bytes[cache] = values[0]['cache_bytes']
                for pct in ['p50', 'p99']:
                    d[label, pct] = [r[pct+'_ns'] for r in values]
            range_lines.append(f'| {layout}/{cache}/{count} | {dist(d["before", "p50"], 1000)} | {dist(d["after", "p50"], 1000)} | {dist(ratio(d["after", "p50"], d["before", "p50"]))} | {dist(ratio(d["after", "p99"], d["before", "p99"]))} |')
            summary['ranges'].append(dict(layout=layout, cache=cache, count=count, latency={f'{l}-{pct}': v for (l, pct), v in d.items()}))
summary['cache_bytes'] = cache_bytes
# Preserve the frozen preliminary analyzer; use measured cgroup-aware budgets
# in the canonical report and correct its descriptive host-RAM estimate.
reads_path=OUT/'READS.md'
reads_path.write_text(reads_path.read_text().replace('about 492 MiB on this host', f'{cache_bytes["default"]/2**20:.3f} MiB on this host'))
lines += range_lines + ['', f'Explicit 512 MiB is {cache_bytes["512"]:,} bytes. The actual default is {cache_bytes["default"]:,} bytes ({cache_bytes["default"]/2**20:.3f} MiB). An ancestor cgroup memory.max of 28,737,957,888 bytes limits the default below the physical-RAM estimate. Both binaries used this actual default; the earlier 492 MiB descriptive estimate was incorrect. Default runs are independent trials.', '', '## Disk space and SST count', '']
lines += table(['Records/layout', 'Main SST MiB', 'Final SST MiB', 'Final/main', 'Main → final SST files', 'Locator MiB'])
space = read(OUT/'space.json')
for n in [10000, 1000000]:
    for layout in ['bulk', 'normal']:
        b, f = [next(r for r in space if r['label'] == l and r['n'] == n and r['layout'] == layout) for l in ['before', 'after']]
        assert b['formats'] == {'3': b['sst_files']} and f['formats'] == {'4': f['sst_files']}
        lines.append(f'| {n}/{layout} | {b["sst_bytes"]/2**20:.3f} | {f["sst_bytes"]/2**20:.3f} | {f["sst_bytes"]/b["sst_bytes"]:.3f} | {b["sst_files"]} → {f["sst_files"]} | {f["point_bytes"]/2**20:.3f} |')
summary['space'] = space
lines += ['', 'Locator metadata retains each full key, MVCC fields, a u64 value address, 12-byte hash slots and about 1.333 pilot bytes per distinct key. It increases output size estimates and can change compaction boundaries. The sorted prefix-compressed value data is unchanged.', '', '## Write and maintenance costs', '',
    'The write suite starts from 10k records and performs 1,000 calls per workload in fast and durable modes. Durable mode syncs each mutation. Per-call latencies exclude final flush. Completion time below includes sync, flush and verification. [WRITES.md](WRITES.md) retains paired p50/p99 and LMDB controls.', '']
lines += table(['Mode', 'Main finish seconds [range]', 'Final finish seconds [range]'])
for mode in ['fast', 'durable']:
    ds = {}
    for label in ['before', 'after']:
        ds[label] = [next(r for r in records(OUT/'writes'/mode/f'10000-bulk-32-r{t}-{label}-kiban.jsonl') if r['type'] == 'complete')['end_sync_flush_verify_seconds'] for t in range(1, 6)]
    lines.append(f'| {mode} | {dist(ds["before"])} | {dist(ds["after"])} |')
    summary['writes'].append(dict(mode=mode, finish_seconds=ds))
maintenance=[]
for line in (OUT/'maintenance-progress.log').read_text().splitlines():
    start=line.find('{"type":"maintenance"')
    if start>=0: maintenance.append(json.loads(line[start:]))
assert len(maintenance)==10 and all(r['errors']==0 and r['verified_records']==1000000 for r in maintenance)
summary['maintenance']=maintenance
lines += ['', 'Maintenance uses the final engine with the index off and on, five alternating pairs on CPU 8. Each pair writes 1M fixed 8-byte keys and 296-byte values, flushes two 500k-record tables, and compacts them. Every output value is checked after timing. BUILD includes merge, index construction, output installation and open-time validation. The standard Rust test allocator differs from the mimalloc read/write executables.', '']
lines += table(['Metric', 'Index off median [range]', 'Index on median [range]', 'Paired on/off [range]'])
for field, scale, name in [('flush_ns',1e9,'Two flushes, seconds'),('compaction_build_ns',1e9,'Compaction BUILD, seconds'),('compaction_commit_ns',1e6,'Compaction COMMIT, ms'),('output_bytes',2**20,'Output MiB'),('output_files',1,'Output SST count'),('point_resident_bytes',2**20,'Resident point/backing MiB')]:
    b,f=[[next(r[field] for r in maintenance if r['trial']==t and r['point_index']==on) for t in range(1,6)] for on in [False,True]]
    lines.append(f'| {name} | {dist(b,scale)} | {dist(f,scale)} | {dist(ratio(f,b)) if all(b) else "n/a"} |')
lines += ['', '## Memory and limits', ''] + rss_lines + ['',
    'Point locators and the complete sorted backing region share the existing raw block-cache capacity. Admitted scans reuse verified backing views. Denied tables verify the locator section in bounded chunks and use sorted blocks. Admission is per table and depends on open order and pinned snapshots/compaction inputs. Reopen can change admission; the latency matrix measures reopened, warm databases. It does not measure cold-open latency or concurrent foreground reads during compaction.', '',
    'The bulk open path currently reads backing bytes into a Vec and converts it into Arc-backed storage, temporarily copying the full data region. Bulk/512 process peak RSS rises from about 433 to 715 MiB. The logical resident-byte reservation does not bound this temporary copy or allocator retention. Arc overhead, saved block metadata and other existing SST metadata remain outside the raw-byte cache budget. Whole-SST builders and MPHF construction need temporary workspace proportional to one output. A bulk SST and oversized key/version groups retain the existing soft-size exceptions. Peak RSS above includes allocator and verification buffers, but does not bound builder workspace or the host page cache.', '',
    'Format 4 uses locator feature flag 2 and section magic KIBANPH2. It rejects the earlier unpublished duplicate-value encoding. Format-3 readers reject format 4. Rewriting with point_index=false is required before downgrading to an engine that only reads format 3. The final API default remains point_index=false.', '',
    'The five-trial results share host memory, LLC and power with the independent crash sweep and desktop processes. Alternating runs and adjacent LMDB controls reduce drift; they do not remove contention. Tail ratios retain the measured control variation. These results apply to this seed, workload and warm-cache policy.', '',
    'The earlier duplicate-value matrix is rejected and retained in [scan-regression-screening](scan-regression-screening/NOTICE.md). Its normal/512 latest-K p50 Kiban/LMDB ratio rose from 1.407 to 1.667 and SST count doubled from 84 to 167. The profile recorded 53.9B → 57.1B instructions and 25.3M → 46.2M branch misses for 1M latest-K calls. Those counters diagnose that rejected layout; they are not a profile of final locators.', '',
    '## Correctness gates', '',
    'All gates pass on the exact engine source hashes captured before the timed matrix. The seeded suites each cover 20,000 seeds starting at zero. Seed parity selects the initial write format, and crash reopens flip format policy. Mixed-format reads, acknowledged-prefix recovery, MVCC snapshots, tombstones, full-key comparison despite fingerprint collisions, corruption, and cache accounting have focused tests.', '']
lines += table(['Gate', 'Status', 'Seconds'])
for r in checks: lines.append(f'| {r["name"]} | PASS | {r["seconds"]:.2f} |')
lines += ['', 'Reproduce with [VERIFY.md](VERIFY.md). Engine source hashes, binary hashes, command receipts and per-trial samples are committed beside this report. [decisions.tsv](decisions.tsv) records the design changes and verdict. No push or PR was made.', '']
profile_rows=[]
for folder in ['profile', 'profile-scan']:
    for path in sorted((OUT/folder).glob('*.stat')):
        counts={}
        for line in path.read_text().splitlines():
            fields=line.split(';')
            if len(fields)>=3:
                try: counts[fields[2]]=float(fields[0].replace(',', ''))/1_000_000
                except ValueError: pass
        profile_rows.append(dict(workload=folder, name=path.stem, per_operation=counts))
if profile_rows:
    lines += ['', '## Final CPU counters', '', 'One diagnostic run per row, outside the five-trial latency matrix. Counters cover 1M full public operations, including key formatting and JSON decoding. Events are user-mode cpu_core counters. These observations diagnose CPU work; they are not additional latency trials.', '']
    lines += table(['Workload/build/backend', 'cycles/op', 'instructions/op', 'LLC misses/op', 'dTLB misses/op', 'branch misses/op'])
    for row in profile_rows:
        d=row['per_operation']
        values=[f'{d["cpu_core/"+event+"/u"]:.3f}' for event in ['cycles','instructions','LLC-load-misses','dTLB-load-misses','branch-misses']]
        lines.append('| '+row['workload']+'/'+row['name']+' | '+' | '.join(values)+' |')
lines += ['', 'The point gain combines hash routing and contiguous backing storage. In the diagnostic bulk get, instructions fall from 8,253 to 5,639 per call and dTLB misses from 0.156 to 0.020. This integrated measurement does not isolate hashing from memory layout or THP; the earlier ablation addresses those effects for its own layout.', '',
    'The remaining scan concern has supporting counter evidence. Bulk latest-K cycles rise from 11,759 to 12,141 per call, while branch misses rise from 13.890 to 22.591 despite slightly fewer instructions. Normal latest-K also records more branch misses. LMDB scan controls show branch-miss drift too, so these observations do not locate a source-level cause. The data justify holding the strict tail gate, not assigning the entire p99 shift to the index.', '']
summary['profile']=profile_rows
(OUT/'REPORT.md').write_text('\n'.join(lines))
(OUT/'final-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print('Revalidated final results and wrote REPORT.md and final-summary.json.')
