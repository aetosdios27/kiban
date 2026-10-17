#!/usr/bin/env python3
"""Build a standalone, interactive view of the committed final summary."""
import html
import json
import statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'validation/v2'
s=json.loads((OUT/'final-summary.json').read_text())
med=statistics.median
panels=[]
for work,title in [('recall-by-id','Point reads'),('recent-for-agent','Latest-K')]:
    rows=[]
    for row in s['reads']:
        if row['workload']!=work:continue
        r=row['ratios'];d=row['latency']
        ratios=lambda label:' / '.join(f'{med(r[label+"-"+p]):.3f}×' for p in ['p50','p99'])
        latency=' / '.join(f'{med(d["after-kiban-"+p])/1000:.3f}' for p in ['p50','p99'])
        rows.append(f'<tr><td>{row["layout"]} · {row["cache"]}</td><td>{ratios("before")}</td><td>{ratios("after")}</td><td>{latency}</td></tr>')
    panels.append(f'<section id="{work}" {"hidden" if panels else ""}><div class="scroll"><table><thead><tr><th>Layout · cache MiB</th><th>Main / LMDB<br>p50 / p99</th><th>Final / LMDB<br>p50 / p99</th><th>Final µs<br>p50 / p99</th></tr></thead><tbody>{"".join(rows)}</tbody></table></div></section>')
m=s['maintenance'];costs=[]
for field,scale,label in [('flush_ns',1e9,'Two flushes, s'),('compaction_build_ns',1e9,'Compaction BUILD, s'),('compaction_commit_ns',1e6,'Compaction COMMIT, ms')]:
    b,f=[[next(r[field] for r in m if r['trial']==t and r['point_index']==on) for t in range(1,6)] for on in [False,True]]
    costs.append(f'<tr><td>{label}</td><td>{med(b)/scale:.3f}</td><td>{med(f)/scale:.3f}</td><td>{med([y/x for x,y in zip(b,f)]):.3f}×</td></tr>')
normal=[next(r for r in s['space'] if r['n']==1000000 and r['layout']=='normal' and r['label']==l) for l in ['before','after']]
body='''<!doctype html><html><head><meta charset="utf-8"><style>
*{box-sizing:border-box}body{margin:0;font:14px/1.5 var(--font-sans,system-ui);color:var(--foreground,#ddd)}p{margin:0 0 14px}h3{font-size:15px;margin:22px 0 8px}nav{display:flex;gap:8px;margin:14px 0}button{font:inherit;border:1px solid var(--border,#555);border-radius:var(--radius,6px);padding:6px 13px;background:var(--secondary,#222);color:inherit;cursor:pointer}button[aria-pressed=true]{background:var(--primary,#ddd);color:var(--primary-foreground,#111)}table{border-collapse:collapse;width:100%;font-size:12px;font-variant-numeric:tabular-nums}td,th{text-align:right;padding:8px 6px;border-bottom:1px solid var(--border,#444);white-space:nowrap}td:first-child,th:first-child{text-align:left}th{font-weight:600;color:var(--muted-foreground,#aaa)}.scroll{overflow-x:auto}.muted{font-size:12px;color:var(--muted-foreground,#aaa)}.status{font-weight:650;color:var(--warning,#d6a953)}[hidden]{display:none}
</style></head><body>'''
body+=f'<p><span class="status">{html.escape(s["verdict"])}</span> · {html.escape(s["reason"])}</p>'
body+=f'<p class="muted">1M records · five alternating paired trials · CPU 8 · main 8bbb4934 · 100k verified calls per point/latest-K stage. Ratios below 1 favor Kiban. Default is {s["cache_bytes"]["default"]/2**20:.3f} MiB.</p>'
body+='<nav><button aria-pressed="true" data-panel="recall-by-id">Point reads</button><button aria-pressed="false" data-panel="recent-for-agent">Latest-K</button></nav>'+''.join(panels)
body+='<h3>Index construction costs</h3><p class="muted">Final engine, index off/on. Five pairs; 1M binary records per run. BUILD includes publication and validation.</p><div class="scroll"><table><thead><tr><th>Metric</th><th>Off</th><th>On</th><th>On / off</th></tr></thead><tbody>'+''.join(costs)+'</tbody></table></div>'
body+=f'<p class="muted" style="margin-top:12px">Normal SST bytes: {normal[1]["sst_bytes"]/normal[0]["sst_bytes"]:.3f}× main. SST count: {normal[0]["sst_files"]} → {normal[1]["sst_files"]}. Values are stored once.</p>'
body+='<h3>10k write population</h3>'
for mode in ['fast','durable']:
    values={}
    for label in ['before','after']:
        d={}
        for t in range(1,6):
            path=OUT/'writes'/mode/f'10000-bulk-32-r{t}-{label}-kiban.jsonl'
            for row in map(json.loads,path.read_text().splitlines()):
                if row['type']=='stage' and row['workload'] in ['remember','update']:
                    d.setdefault(row['workload'],[]).append(row['latency']['p50_ns'])
        values[label]='/'.join(f'{med(d[w])/1000:.3f}' for w in ['remember','update'])
    body+=f'<p class="muted">{mode.capitalize()} insert/update p50 µs: main {values["before"]}, final {values["after"]}. Per-call timings exclude final flush.</p>'
for row in s['writes']:
    d=row['finish_seconds']
    body+=f'<p class="muted">{row["mode"].capitalize()} final sync/flush/verification: {med(d["before"])*1000:.2f} → {med(d["after"])*1000:.2f} ms.</p>'
body+='<h3>Validation and limits</h3><p>All 11 correctness gates pass, including both 20,000-seed mixed-format crash/model suites and loom. No payload errors were recorded.</p><p class="muted">Admission is per SST under the shared cache budget. At 32 MiB, large tables fall back to sorted reads. Pinned compaction inputs can deny output admission until reopen. Warm-read timings exclude cold-open cost and concurrent compaction. Bulk/512 peak RSS rises from 433 to 715 MiB, including an open-time copy of the backing region. Temporary workspace and SST metadata sit outside the raw-byte cache capacity. The write option remains disabled by default.</p>'
body+='''<script>document.querySelectorAll('button[data-panel]').forEach(b=>b.onclick=()=>{document.querySelectorAll('button[data-panel]').forEach(x=>x.setAttribute('aria-pressed',String(x===b)));document.querySelectorAll('section').forEach(s=>s.hidden=s.id!==b.dataset.panel)});console.assert(document.querySelectorAll('section').length===2);console.assert(document.querySelectorAll('section tbody tr').length===12);</script></body></html>'''
(OUT/'summary.html').write_text(body)
print('Wrote summary.html')
