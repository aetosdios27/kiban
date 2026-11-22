#!/usr/bin/env python3
"""Audit raw Fjall campaign receipts and regenerate result tables."""
import argparse
from collections import defaultdict
import csv
import json
from pathlib import Path
import re
from statistics import median

from fjall_bench import (
    CANDIDATES, ROOT, SIBLING, SUPPLEMENT, audit_command, campaign_rows, digest, dump,
    lines, phase_folder, select,
)

LABELS = ('kiban-index0', 'kiban-index1', 'fjall-tuned', 'fjall-lz4', 'lmdb')
HEADINGS = ('Kiban off', 'Kiban on', 'Fjall tuned', 'Fjall LZ4', 'LMDB')
LATENCY = ('p50_us', 'p99_us', 'call_kops_s', 'verified_kops_s')
RESOURCES = ('logical_mib', 'allocated_mib', 'peak_rss_mib')


def busy(receipt):
    t = receipt['cpu_tick_delta'][f'cpu{SIBLING}']
    # User and nice already include guest ticks. Iowait is not executed work.
    total = sum(t[:8])
    return 100 * (total - t[3] - t[4]) / total if total else 0


def flatten(raw, winners, candidates=CANDIDATES):
    folder = phase_folder(raw, winners, candidates)
    phase = folder.name
    for scenario, name, _, receipt, rows in campaign_rows(raw, winners, candidates):
        trial, label = name.split('-', 1)
        stamp = (folder / scenario / f'{name}.time.txt').read_text()
        rss = int(re.search(r'Maximum resident set size \(kbytes\): (\d+)', stamp)[1])
        files = receipt['disk']
        quant = dict(phase=phase, scenario=scenario, trial=int(trial[1:]), label=label,
                     logical_bytes=files['logical_bytes'], allocated_bytes=files['allocated_bytes'],
                     peak_rss_kib=rss, logical_mib=files['logical_bytes'] / 2**20,
                     allocated_mib=files['allocated_bytes'] / 2**20, peak_rss_mib=rss / 1024,
                     sibling_busy_pct=busy(receipt), load1=receipt['load'][0],
                     command_seconds=receipt['ended'] - receipt['started'])
        completion = next(row for row in rows if row['type'] == 'complete')
        quant['end_sync_flush_verify_seconds'] = completion.get('end_sync_flush_verify_seconds', '')
        for row in rows:
            if row['type'] != 'stage':
                continue
            h = row['latency']
            yield dict(quant, workload=row['workload'], calls=h['count'],
                       mutations=row['writes']['count'], p50_us=h['p50_ns'] / 1000,
                       p99_us=h['p99_ns'] / 1000, mean_us=row['call_seconds'] * 1e6 / h['count'],
                       call_kops_s=row['call_ops_s'] / 1000, verified_kops_s=row['verified_ops_s'] / 1000)


def table(headers, data):
    def format_row(row):
        return '| ' + ' | '.join(map(str, row)) + ' |'
    return '\n'.join([format_row(headers), format_row(['---'] * len(headers)), *map(format_row, data)])


def stats(values):
    return dict(median=median(values), minimum=min(values), maximum=max(values))


def interval(row):
    return f'{row["median"]:.3f} [{row["minimum"]:.3f}, {row["maximum"]:.3f}]'


def summarize(rows, fields):
    return {field: stats([row[field] for row in rows]) for field in fields}


def groups(rows, fields):
    grouped = defaultdict(list)
    for row in rows:
        grouped[tuple(row[f] for f in fields)].append(row)
    return grouped


def tables(primary):
    stages = groups(primary, ('scenario', 'workload', 'label'))
    samples = {}
    paired = []
    for scenario, workload, label in stages:
        rows = stages[scenario, workload, label]
        assert [r['trial'] for r in rows] == list(range(1, 6))
        samples[scenario, workload, label] = summarize(rows, (*LATENCY, 'mean_us'))
        if label.startswith('kiban-'):
            for peer in ('fjall-tuned', 'lmdb'):
                comparators = stages[scenario, workload, peer]
                for field in LATENCY:
                    ratio = stats([left[field] / right[field]
                                   for left, right in zip(rows, comparators, strict=True)])
                    paired.append(dict(scenario=scenario, workload=workload, label=label, peer=peer,
                                       metric=field, **ratio))
    lines_out = ['# Primary results',
                 'All cells contain the median of five trials. Percentiles are upper bucket bounds.',
                 '## Latency and throughput',
                 'Each cell gives p50 µs / p99 µs / call kops/s / verified kops/s.']
    stages_by_scenario = dict.fromkeys((k[0], k[1]) for k in stages)
    lines_out.append(table(('Scenario', 'Workload', *HEADINGS), (
        (scenario, workload, *(' / '.join(f'{samples[scenario, workload, label][field]["median"]:.3f}'
                                         for field in LATENCY) for label in LABELS))
        for scenario, workload in stages_by_scenario
    )))
    lines_out.extend(('## Same-round Kiban ratios', 'Each cell gives p50 / p99 / call throughput / verified throughput. A latency ratio under 1 favors Kiban. A throughput ratio above 1 favors Kiban.'))
    ratio_rows = groups(paired, ('scenario', 'workload', 'label', 'peer'))
    lines_out.append(table(('Scenario', 'Workload', 'Kiban', 'Comparator', 'Ratios'), (
        (*key, ' / '.join(f'{r["median"]:.3f}' for r in rows))
        for key, rows in ratio_rows.items()
    )))
    lines_out.extend(('## Resources', 'Each cell gives logical disk MiB / allocated disk MiB / peak RSS MiB. RSS covers the whole command.'))
    commands = [rows[0] for rows in groups(primary, ('scenario', 'trial', 'label')).values()]
    resources = {key: summarize(rows, RESOURCES)
                 for key, rows in groups(commands, ('scenario', 'label')).items()}
    lines_out.append(table(('Scenario', *HEADINGS), (
        (scenario, *(' / '.join(f'{resources[scenario, label][field]["median"]:.3f}'
                                for field in RESOURCES) for label in LABELS))
        for scenario in dict.fromkeys(r['scenario'] for r in primary)
    )))
    lines_out.extend(('## Ranges by scenario', 'Cells give median [minimum, maximum]. The paired ratio ranges are in AUDIT.json.'))
    for scenario in dict.fromkeys(r['scenario'] for r in primary):
        lines_out.extend((f'### {scenario}', table(
            ('Workload', 'Candidate', 'p50 µs', 'p99 µs', 'Call kops/s', 'Verified kops/s', 'Mean µs'),
            ((workload, label, *(interval(values[field]) for field in (*LATENCY, 'mean_us')))
             for (group, workload, label), values in samples.items() if group == scenario)),
            table(('Candidate', 'Logical disk MiB', 'Allocated disk MiB', 'Peak RSS MiB'),
                  ((label, *(interval(resources[scenario, label][field]) for field in RESOURCES))
                   for label in LABELS))))
    return '\n\n'.join(lines_out) + '\n', paired, commands


def screen_tables(screen, selection, candidates):
    lines_out = ['# Screening results', 'Scores are the geometric means specified in the protocol. Lower is better.',
                 table(('Candidate', 'Read score µs', 'Write score µs', 'Selected in'),
                       ((label, f'{selection["scores_seconds"]["read"][label]*1e6:.6f}',
                         f'{selection["scores_seconds"]["write"][label]*1e6:.6f}',
                         ', '.join(family for family in ('read', 'write') if selection[family] == label))
                        for label in candidates)),
                 'Each cell gives median [minimum, maximum] across five trials. The LMDB rows are controls and have no selection score.']
    for scenario, datasets in groups(screen, ('scenario',)).items():
        stages = groups(datasets, ('workload', 'label'))
        lines_out.extend((f'## {scenario[0]}', table(
            ('Workload', 'Candidate', 'p50 µs', 'p99 µs', 'Call kops/s', 'Verified kops/s'),
            ((*key, *(interval(metric) for metric in summarize(rows, LATENCY).values()))
             for key, rows in stages.items()))))
    return '\n\n'.join(lines_out) + '\n'


def audit_checks(checks):
    sync = json.loads((checks / 'sync.json').read_text())
    for r in lines(checks / 'sync/commands.jsonl'):
        c, _ = audit_command(checks / 'sync', r)
        assert r['diagnostic'] and c['operations'] == 1000
        match = re.search(r'^sync_probe fsync=(\d+) fdatasync=(\d+) failures=(\d+)$',
                          (checks / 'sync' / (r['name'] + '.log')).read_text(), re.M)
        assert [sync[r['name']][field] for field in ('fsync', 'fdatasync', 'failures')] == list(map(int, match.groups()))
    assert len(sync) == 6
    for label in ('kiban-index1', 'fjall-tuned', 'lmdb'):
        slow, fast = (sync[f'{label}-{mode}'] for mode in ('durable', 'fast'))
        assert slow['failures'] == fast['failures'] == 0
        assert slow['fdatasync'] - fast['fdatasync'] == 2100
    crash = json.loads((checks / 'crash.json').read_text())
    expected = [f'ack {id}' for id in range(10000, 10025)]
    assert (checks / 'crash.acks.log').read_text().splitlines() == expected
    assert crash['returncode'] == -9 and crash['verified_records'] == 10025
    recovered = lines(checks / 'crash-reopen.jsonl')[-1]
    assert recovered['type'] == 'complete' and recovered['verified_records'] == 10025
    return sync, crash


def workload_audit(rows, phase):
    commands = [r[0] for r in groups(rows, ('scenario', 'trial', 'label')).values()]
    return {
        f'{phase}_commands': len(commands),
        f'{phase}_calls': sum(r['calls'] for r in rows),
        f'{phase}_mutations': sum(r['mutations'] for r in rows),
        f'{phase}_host': summarize(commands, ('sibling_busy_pct', 'load1')),
    }


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--supplement', action='store_true')
    parser.add_argument('--out', type=Path, default=ROOT / 'validation/fjall')
    args = parser.parse_args()
    out = args.out.resolve()
    candidates = SUPPLEMENT if args.supplement else CANDIDATES
    raw = out / 'raw'
    if args.supplement:
        raw /= 'screen-supplement'
    metadata = json.loads((raw / 'metadata.json').read_text())
    assert metadata['candidates'] == candidates
    assert all(digest(ROOT / path) == sha for path, sha in metadata['sources'].items())
    binary = Path(metadata['binary'])
    if binary.is_file():
        assert digest(binary) == metadata['binary_sha256']
    selection = json.loads((raw / 'selection.json').read_text())
    assert select(raw, candidates) == selection
    prep = raw / 'prepare'
    prep_rows = lines(prep / 'commands.jsonl')
    repicked = args.supplement and (selection['read'], selection['write']) != ('block4', 'block4')
    has_primary = not args.supplement or (repicked and len(prep_rows) > 32)
    expected_preps = (35 if has_primary else 32) if args.supplement else 23
    assert len(prep_rows) == len({r['name'] for r in prep_rows}) == expected_preps
    for r in prep_rows:
        audit_command(prep, r)
    screen = list(flatten(raw, None, candidates))
    assert len(screen) == (810 if args.supplement else 450)
    screen_path = out / ('SCREEN-SUPPLEMENT.md' if args.supplement else 'SCREEN.md')
    screen_path.write_text(screen_tables(screen, selection, candidates))
    audit = dict(
        **workload_audit(screen, 'screen'),
        prepare_commands=len(prep_rows),
        source_hashes_verified=True,
        binary_hash_verified=binary.is_file(),
    )
    if args.supplement:
        dump(out / 'AUDIT-SUPPLEMENT.json', audit)
    if has_primary:
        primary = list(flatten(raw, selection, candidates))
        assert len(primary) == 450
        sync, crash = audit_checks(out / 'checks')
        results, pairs, _ = tables(primary)
        (out / 'RESULTS.md').write_text(results)
        with (out / 'TRIALS.csv').open('w', newline='') as f:
            writer = csv.DictWriter(f, fieldnames=list(screen[0]), lineterminator='\n')
            writer.writeheader()
            writer.writerows([*screen, *primary])
        dump(out / 'AUDIT.json', dict(
            **workload_audit(primary, 'primary'),
            **audit,
            sync=sync, crash=crash, paired_ratios=pairs,
        ))


if __name__ == '__main__':
    main()
