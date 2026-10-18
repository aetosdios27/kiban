#!/usr/bin/env python3
"""Build a standalone interactive view of the checked filter experiment."""
import json
from pathlib import Path

root = Path(__file__).resolve().parents[3]
out = root/'validation/fuse-engine'
rows = json.loads((out/'paired-summary.json').read_text())
checks = json.loads((out/'analysis-check.json').read_text())
maintenance = json.loads((out/'maintenance-summary.json').read_text())
page = r'''<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Kiban filter measurements</title><style>
body{margin:0;color:var(--foreground,#172033);font-family:var(--font-sans,system-ui);font-size:14px;line-height:1.5}p{margin:0 0 14px}strong{font-weight:650}.muted,small{color:var(--muted-foreground,#64748b)}.controls{display:flex;flex-wrap:wrap;gap:12px;margin:16px 0}label{display:grid;gap:5px;font-size:12px}select{font:inherit;font-size:14px;padding:6px 9px;border:1px solid var(--border,#cbd5e1);border-radius:6px;background:var(--background,#fff);color:inherit}.scroll{overflow:auto}table{width:100%;border-collapse:collapse;font-size:13px}th,td{text-align:left;vertical-align:top;padding:9px 8px;border-bottom:1px solid var(--border,#dbe2ea)}th:first-child,td:first-child{padding-left:0}th:last-child,td:last-child{padding-right:0}th{font-size:11px;font-weight:600;color:var(--muted-foreground,#64748b)}.number{font-variant-numeric:tabular-nums;white-space:nowrap}small{display:block;font-size:11px}details{margin-top:15px}summary{cursor:pointer;font-weight:550}details p{margin-top:10px}.note{padding:16px;border-radius:var(--radius,8px);background:var(--secondary,#f1f5f9);margin-top:18px}.pill{display:inline-block;padding:2px 7px;border-radius:12px;background:var(--success-surface,#e7f5eb);color:var(--success-foreground,#166534);font-size:12px;margin-right:5px}.wrap{white-space:normal}a{color:var(--primary,#2563eb)}@media(max-width:480px){table{font-size:12px}th,td{padding:8px 5px}.controls{gap:8px}.scroll table{min-width:535px}}
</style></head><body>
<p><strong>Fuse stays opt-in.</strong> Five paired trials per cell compare main’s Bloom filter with the integrated fuse8 + XXH3 filter. Times include the existing engine path and timer overhead.</p>
<div class="controls"><label>Layout<select id="layout"><option value="bulk">Bulk · 1M keys</option><option value="normal">Normal · 1M keys</option></select></label><label>Cache<select id="cache"><option value="32">32 MiB</option><option value="None">Default · 428.23 MiB</option></select></label><label>Point index<select id="index"><option value="False">Off</option><option value="True">On</option></select></label></div>
<div class="scroll"><table><thead><tr><th>Workload</th><th>p50 µs<br>Bloom → fuse</th><th>p99 µs<br>Bloom → fuse</th><th>Paired call-time ratio<small>fuse / Bloom; &lt;1 faster</small></th><th>Relative to LMDB<small>paired ratio / control ratio</small></th></tr></thead><tbody id="engine"></tbody></table></div>
<details><summary>LMDB control and five-run ranges</summary><div class="scroll"><table><thead><tr><th>Workload</th><th>Kiban before p50 µs</th><th>Kiban after p50 µs</th><th>LMDB before → after p50 µs</th><th>LMDB call-time ratio</th></tr></thead><tbody id="controls"></tbody></table></div></details>
<div class="note"><p id="validation"></p><p id="sibling" class="muted"></p><p class="muted" style="margin:0">Strict idle-sibling conditions were not met. Results establish this run’s behavior; they do not establish quiet-host tails or an unconditional default win. Range routing may reject absent IDs before a filter query.</p></div>
<details><summary>Flush and compaction costs · 1M keys</summary><p class="muted">Median [min, max] in milliseconds. Includes CRC, full source-key verification, atomic SST writes and reopening. Fill and final member checks are excluded.</p><div class="scroll"><table><thead><tr><th>PH3</th><th>Phase</th><th>Bloom ms</th><th>Fuse ms</th><th>Paired ratio</th></tr></thead><tbody id="cost"></tbody></table></div></details>
<p class="muted" style="margin-top:15px">New footer versions 5/6 and a checked filter header preserve mixed old/new databases. Resident PH3 still runs first. Default writes remain legacy-compatible until the option is enabled.</p>
<script>
const rows=__ROWS__, check=__CHECKS__, maintenance=__MAINT__;
const names={'recall-by-id':'Hit','absent-uniform':'Absent · all agents','absent-agent-prefix':'Absent · agent 7','recent-for-agent':'Latest-K'};
const workloads=Object.keys(names);
const number=x=>x.split(' ')[0];
function find(layout,cache,index,backend,w){return rows.find(r=>r.group.join('/')===[layout,backend,cache,index,w].join('/'));}
function render(){const l=document.querySelector('#layout').value,c=document.querySelector('#cache').value,i=document.querySelector('#index').value;
 document.querySelector('#engine').innerHTML=workloads.map(w=>{const r=find(l,c,i,'kiban',w);return `<tr><td>${names[w]}</td><td class="number">${number(r.before_p50)} → ${number(r.after_p50)}</td><td class="number">${number(r.before_p99)} → ${number(r.after_p99)}</td><td>${r.paired_ratio}</td><td>${r.control_adjusted_ratio}</td></tr>`}).join('');
 document.querySelector('#controls').innerHTML=workloads.map(w=>{const r=find(l,c,i,'kiban',w),m=find(l,c,i,'lmdb',w);return `<tr><td>${names[w]}</td><td>${r.before_p50}</td><td>${r.after_p50}</td><td>${m.before_p50}<small>→ ${m.after_p50}</small></td><td>${m.paired_ratio}</td></tr>`}).join('');
}
for(const l of ['bulk','normal'])for(const c of ['32','None'])for(const i of ['False','True'])for(const b of ['kiban','lmdb'])for(const w of workloads){if(!find(l,c,i,b,w))throw Error('missing comparison cell');}
document.querySelectorAll('select').forEach(x=>x.addEventListener('change',render));render();
document.querySelector('#validation').innerHTML='<span class="pill">11 gates passed</span><span class="pill">40k crash seeds</span><span class="pill">zero false negatives</span><br><small style="margin-top:8px">360 read/write/control processes and 10 maintenance processes validated; no positive fallback in measured SSTs.</small>';
document.querySelector('#sibling').textContent=`CPU 8 only. CPU 9 activity across paired processes: median ${check.sibling_median.toFixed(1)}%, range ${check.sibling_min.toFixed(1)}–${check.sibling_max.toFixed(1)}%.`;
document.querySelector('#cost').innerHTML=maintenance.filter(r=>r.field.endsWith('_ns')).map(r=>`<tr><td>${r.indexed?'On':'Off'}</td><td>${r.field.replace('_ns','').replaceAll('_',' ')}</td><td>${r.before}</td><td>${r.after}</td><td>${r.paired_ratio}</td></tr>`).join('');
console.log(JSON.stringify({configurations:8,workloads:4,pairedProcesses:check.processes,maintenanceProcesses:check.maintenance_processes,gates:check.gates,missingCells:0}));
</script></body></html>'''
page = page.replace('__ROWS__',json.dumps(rows)).replace('__CHECKS__',json.dumps(checks)).replace('__MAINT__',json.dumps(maintenance))
(out/'results.html').write_text(page)
print(out/'results.html')
