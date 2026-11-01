#!/usr/bin/env python3
"""Build the review tables from validated receipts, without selecting trials."""
from html import escape
from pathlib import Path

from smallcache_summary import load, median, pair, rebased_adjusted


def table(headers, rows):
    return '<div class="scroll"><table><thead><tr>'+''.join(
        '<th>'+escape(h)+'</th>' for h in headers
    )+'</tr></thead><tbody>'+''.join(
        '<tr>'+''.join('<td>'+escape(str(v))+'</td>' for v in row)+'</tr>' for row in rows
    )+'</tbody></table></div>'


def main():
    warm = []
    accepted = []
    for mode in ['mapped', 'pread']:
        for r in load(f'rebased-{mode}-read/summary.json'):
            if r['scenario'][2] != '32' or r['scenario'][-1] != 'recall-by-id':
                continue
            d = r['distributions']
            ratios = [median(a/b for a,b in zip(d['rebased-kiban'][p],d['rebased-lmdb'][p]))
                      for p in ['p50_ns','p99_ns']]
            if mode == 'mapped':
                accepted.append(ratios[0] <= 0.9 and ratios[1] <= 1)
            warm.append([mode, r['scenario'][0], pair(d,'before-kiban'),
                         pair(d,'rebased-kiban'), pair(d,'rebased-lmdb'),
                         ' / '.join(f'{v:.3f}' for v in ratios)])
    assert len(accepted) == 2 and all(accepted)
    cold = []
    for mode in ['mapped', 'pread']:
        for r in load(f'ph3-{mode}-cold/summary.json'):
            if r['scenario'][1:3] != ['1','32']:
                continue
            d = r['distributions']
            cold.append([mode, r['scenario'][0], r['scenario'][3],
                         pair(d,'before-kiban'), pair(d,'ph3-kiban')])
    writes = []
    for series in ['write','write-confirm']:
        for r in load(f'ph3-{series}/summary.json'):
            if r['scenario'][-1] not in ['remember','update']:
                continue
            d = r['distributions']
            writes.append(['original' if series == 'write' else 'confirmation',
                           r['scenario'][1], 'durable' if r['scenario'][4] == '1' else 'fast',
                           r['scenario'][-1], pair(d,'before-kiban'),pair(d,'ph3-kiban')])
    page = '''<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Small-cache read results</title><style>
body{font-family:var(--font-sans,system-ui);color:var(--foreground,#222);line-height:1.45;margin:0}
p{margin:8px 0 16px}h2{font-size:16px;margin:24px 0 6px}
.scroll{overflow-x:auto}table{width:100%;border-collapse:collapse;font-size:12px;font-variant-numeric:tabular-nums}
th,td{padding:7px 6px;text-align:right;border-bottom:1px solid var(--border,#ddd);white-space:nowrap}
th:first-child,td:first-child{text-align:left}th{font-weight:600;color:var(--muted-foreground,#666)}
.note{font-size:13px;color:var(--muted-foreground,#666)}
</style><body>
<p><strong>Warm mmap target passed. The full non-regression goal remains unproven.</strong></p>
<p class="note">1M records, 32 MiB block cache, point index enabled. All latency units are microseconds. Mapped means explicit mmap_max_level=Some(u32::MAX); pread means None.</p>
<h2>Warm point reads against current main 37feee99</h2>'''
    page += table(['Mode','Layout','Main p50 / p99','Candidate p50 / p99','LMDB p50 / p99','Paired K/L p50 / p99'],warm)
    page += '<p class="note">Three alternating pairs with adjacent LMDB controls. Required paired ratios: p50 ≤ 0.9 and p99 ≤ 1. Default and 512 MiB controls, latest-ten results, and all trial ranges are in REPORT.md.</p>'
    controls = []
    for r in load('rebased-controls-mapped-read/summary.json'):
        d = r['distributions']
        controls.append([r['scenario'][2].replace('None','default'),r['scenario'][-1],
                         pair(d,'before-kiban'),pair(d,'rebased-kiban'),
                         ' / '.join(f'{rebased_adjusted(r,p):.3f}' for p in ['p50_ns','p99_ns'])])
    page += '<h2>Bulk mapped controls against current main</h2><p class="note">Five additional pairs, retained separately. Ratios above one are measured relative increases. These controls prevent full non-regression acceptance.</p>'
    page += table(['Cache MiB','Workload','Main p50 / p99','Candidate p50 / p99','Adjusted p50 / p99'],controls)
    page += '<p class="note">All timed children use CPU 6. Ambient CPU 7 activity was recorded over whole child processes, so strict idle-sibling acceptance is not established.</p>'
    page += '<h2>Cold and concurrent point reads</h2><p class="note">Archived five-pair campaign against main 06a457db. File-specific eviction after open; pages warm during 10k calls. Four readers share CPU 6.</p>'
    page += table(['Mode','Layout','Readers','Main p50 / p99','Candidate p50 / p99'],cold)
    page += '<p class="note">Mapped normal single-reader p99 rises about 29%. Four-reader mapped tails improve against the old main but exceed pread. mmap remains opt-in.</p>'
    page += '<h2>Writes: both series retained</h2><p class="note">Five pairs per series against main 06a457db. Each stage performs 10,000 operations on fresh 10k-record fixtures. Index 1 means enabled. Final flush and verification are outside call timers and preserved in the report.</p>'
    page += table(['Series','Index','Mode','Stage','Main p50 / p99','Candidate p50 / p99'],writes)
    page += '<p class="note">Fast indexed update improves in confirmation. Durable unindexed insert p99 changes direction; durable update p99 remains higher in confirmation. The evidence does not establish the full requested write-tail guarantee.</p></body></html>'
    root = Path(__file__).resolve().parents[3]
    (root/'validation/smallcache/HEADLINE.html').write_text(page+'\n')


if __name__ == '__main__':
    main()
