#!/usr/bin/env python3
"""Render the complete frozen small-cache evidence without selecting trials."""
import hashlib
import json
from pathlib import Path
import re
import statistics as st

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'validation/smallcache'

def load(name):
    return json.loads((OUT/name).read_text())

def median(values):
    return st.median(values)

def pair(d, side):
    return ' / '.join(f'{median(d[side][p])/1000:.3f}' for p in ['p50_ns','p99_ns'])

def normalized(row, pct):
    d=row['distributions']
    return median((a/b)/(c/e) for a,b,c,e in zip(
        d['ph3-kiban'][pct],d['ph3-lmdb'][pct],d['before-kiban'][pct],d['before-lmdb'][pct]))

def rebased_adjusted(row, pct):
    d = row['distributions']
    return median((a/b)/(c/e) for a,b,c,e in zip(
        d['rebased-kiban'][pct], d['rebased-lmdb'][pct],
        d['before-kiban'][pct], d['before-lmdb'][pct]))

def control_confirmation_lines():
    path = 'rebased-controls-mapped-read/summary.json'
    if not (OUT/path).exists():
        return []
    rows = load(path)
    assert len(rows) == 6
    lines = ['', '### Bulk control confirmation', '',
             'Five additional alternating pairs repeat the three uncertain mapped bulk cache cases with the same binaries and fixtures. The original three-pair matrix is retained in full. Latencies are microseconds.', '',
             '| Cache MiB / workload | Main p50/p99 | Candidate p50/p99 | Adjusted candidate/main p50/p99 |',
             '| --- | ---: | ---: | ---: |']
    for r in rows:
        d = r['distributions']
        name = f'{r["scenario"][2].replace("None", "default")} / {r["scenario"][-1]}'
        lines.append(f'| {name} | {pair(d,"before-kiban")} | {pair(d,"rebased-kiban")} | {rebased_adjusted(r,"p50_ns"):.3f} / {rebased_adjusted(r,"p99_ns"):.3f} |')
    lines += ['', 'The large initial bulk/32 latest-ten slowdown does not reproduce at five pairs: adjusted p50/p99 is 1.029/0.966. Other controls remain above main. Default point is 1.038/0.899, 512 MiB point is 1.045/1.049, and default latest-ten is 1.050/1.139. The data do not establish the requested larger-cache and latest-ten non-regression bounds. All trial ranges and both series remain visible.', '',
              '[All five-pair controls and ranges](rebased-controls-mapped-read/SUMMARY.md).']
    return lines

def rebased_lines():
    if not (OUT/'rebased-provenance.json').exists():
        return []
    provenance = load('rebased-provenance.json')
    status = load('rebased-checks/status.json')
    assert len(status) == 11 and all(r['code'] == 0 for r in status)
    assert all(hashlib.sha256((ROOT/p).read_bytes()).hexdigest() == h
               for p,h in provenance['engine_source_hashes'].items())
    lines = [
        '**The mapped warm point target passes; full acceptance is not established.** The final engine is rebased onto main `37feee99`. Larger-cache and latest-ten controls remain above main in the additional confirmation, and durable-write p99 non-regression remains unproven. Keep mmap opt-in and do not claim the full requested goal is met.', '',
        f'Rebased source was frozen at `{provenance["head"]}`. Engine hashes in [rebased provenance](rebased-provenance.json) identify the validated source independently of later evidence-only commits.', '',
        '## Rebased warm confirmation against current main', '',
        'Three alternating pairs per layout, cache, and access mode. Every Kiban process has an adjacent LMDB control. Timed children use CPU 6; builds, gates, and coordinators use CPUs 12–21 at nice 10. Other host work remained active. Each trial runs 100,000 point calls and 100,000 latest-ten calls and verifies all 1M records. Ratios are medians of within-pair ratios. Latencies are microseconds.', '',
        'Both binaries include main\'s SQLite benchmark support. Main fixtures are fresh flag-zero PH3, while candidate fixtures are the frozen CRC-bearing PH3 datasets. The production engine logic matches the frozen campaign; the reader has only main\'s explanatory comment, and the ignored maintenance benchmark gains a trial selector. See [source comparison](rebased-source-comparison.json).', '',
        '| Mode / layout / cache MiB | Main p50/p99 | Candidate p50/p99 | LMDB p50/p99 | Paired K/L p50/p99 | Adjusted candidate/main p50/p99 |',
        '| --- | ---: | ---: | ---: | ---: | ---: |']
    controls = []
    target = []
    for mode in ['mapped', 'pread']:
        for r in load(f'rebased-{mode}-read/summary.json'):
            layout, _, cache, _, _, stage = r['scenario']
            d = r['distributions']
            name = f'{mode} / {layout} / {cache.replace("None", "default")}'
            if stage == 'recent-for-agent':
                controls.append(f'| {name} | {pair(d,"before-kiban")} | {pair(d,"rebased-kiban")} | {rebased_adjusted(r,"p50_ns"):.3f} / {rebased_adjusted(r,"p99_ns"):.3f} |')
                continue
            ratios = [median(a/b for a,b in zip(d['rebased-kiban'][pct], d['rebased-lmdb'][pct])) for pct in ['p50_ns', 'p99_ns']]
            if mode == 'mapped' and cache == '32':
                target.append(ratios[0] <= 0.9 and ratios[1] <= 1)
            lines.append(f'| {name} | {pair(d,"before-kiban")} | {pair(d,"rebased-kiban")} | {pair(d,"rebased-lmdb")} | {ratios[0]:.3f} / {ratios[1]:.3f} | {rebased_adjusted(r,"p50_ns"):.3f} / {rebased_adjusted(r,"p99_ns"):.3f} |')
    assert len(target) == 2 and all(target), target
    lines += ['', '[Mapped trial ranges](rebased-mapped-read/SUMMARY.md), [pread trial ranges](rebased-pread-read/SUMMARY.md). Values above one in the adjusted columns remain visible; three pairs are a confirmation, not a replacement for the five-pair campaign.', '',
              '| Latest-ten mode / layout / cache MiB | Main p50/p99 | Candidate p50/p99 | Adjusted candidate/main p50/p99 |',
              '| --- | ---: | ---: | ---: |', *controls, *control_confirmation_lines(), '',
              '## Rebase and final gates', '',
              'PH3 base `4feca511` is already present through squash merge `f3d21dc1` and was dropped from the replay. Main\'s Bloom bypass is retained. Duplicate engine commit `0283fc7a` was dropped, with its unique tools and measurement evidence preserved in the campaign evidence commit. The locator lookup follows the same exact-key-before-Bloom ordering. The CRC extension, resident packing, exact reservation accounting, and L0 prefix fix remain. Main\'s PH3 field offsets and frozen flag-zero contract are retained in the design document. SQLite benchmarking from `37feee99` is unchanged.', '',
              'All 11 gates passed again on the rebased source, including 353 debug and release library tests, fixed corpora, loom, and both complete 20,000-seed suites. No timing runs overlapped this task\'s builds or tests. See [gate status](rebased-checks/status.json), [source hashes](rebased-checks/source-hashes.json), and [reproduction commands](VERIFY.md).', '',
              'The verifier now adds only the niceness increment needed to reach nice 10. A partial run with inherited nice 19 was preserved under rebased-checks-inherited-nice19, then all 11 gates were repeated at effective nice 10. The final metadata records coordinator nice 10 and increment zero.', '',
              'No task process uses sibling CPU 7, but the sibling was not established to be idle. Whole-child CPU 7 busy fractions were median 12.3% for mapped, 10.8% for pread, and 11.9% for the extra controls, with maxima 48.8%, 87.9%, and 23.2%. These samples include open and verification, so they do not isolate activity during API timers. This is an active-host measurement, and strict idle-sibling acceptance is not established. See [sibling summary](rebased-sibling-summary.json) and the per-child sibling.jsonl receipts.', '',
              'Cold, concurrent, profiling, and write rows below retain the original five-pair campaign against `06a457db`; they were not repeated against updated main. The separate write confirmation is also preserved. Those results establish the observed cold mmap tradeoff and leave durable-write p99 uncertain. The source comparison supports carrying that evidence forward, while the new three-pair warm matrix directly checks the rebased binary and main\'s PH3 baseline.', '',
              'The [frozen source patch](experiments/frozen-source-from-37feee99.patch) reconstructs the archived engine and Rust readbench source from public main `37feee99`, independently of old local commit reachability after rebase. Applying it to a private Git index reproduced all 40 frozen engine file hashes. See the [reconstruction receipt](frozen-reconstruction.json).', '',
              '## Archived five-pair campaign', '']
    return lines

def table(rows, mode, cold=False):
    lines=['', '| Layout / index / cache / readers | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Paired K/L p50 | Adjusted PH3/main p50/p99 |',
           '| --- | ---: | ---: | ---: | ---: | ---: |']
    for r in rows:
        layout,index,cache,readers,_,_=r['scenario'];d=r['distributions']
        scenario=f'{layout} / {index} / {cache.replace("None","default")} / {readers}'
        if cold and index == '1' and cache in ['None', '512']:
            scenario += ' †'
        lines.append(f'| {scenario} | {pair(d,"before-kiban")} | {pair(d,"ph3-kiban")} | {pair(d,"ph3-lmdb")} | {median(r["ratios"]["ph3"]):.3f} | {normalized(r,"p50_ns"):.3f} / {normalized(r,"p99_ns"):.3f} |')
    if cold:
        lines += ['', '† Retained verified heap backing; file-page eviction does not make these values cold.']
    lines += ['', f'Raw trials, controls and ranges: [ph3-{mode}/SUMMARY.md](ph3-{mode}/SUMMARY.md).']
    return lines

def main():
    provenance=load('ph3-provenance.json');gates=load('ph3-checks/status.json')
    assert len(gates)==11 and all(r['code']==0 for r in gates)
    assert provenance['engine_source_hashes'] == load('ph3-checks/source-hashes.json')
    if not (OUT/'rebased-provenance.json').exists():
        assert all(hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==h for p,h in provenance['engine_source_hashes'].items())
    target=load('ph3-target.json')
    folders=['mapped-read','pread-read','mapped-cold','pread-cold','write']
    results={name:load(f'ph3-{name}/summary.json') for name in folders}
    lines=['# Small-cache point reads', '', *rebased_lines(),
        f'Engine source: `{provenance["engine_commit"]}`; baseline main `06a457db`. All timing children use CPU 6; build/test/coordinator processes use CPUs 12–21 at nice 10. No task work runs on sibling CPU 7. Other host workloads were active; this is not an isolated-host claim.', '',
        'The opt-in configuration is **CRC-bearing PH3 SSTs, point_index=true, block_cache_bytes=32 MiB, mmap_max_level=Some(u32::MAX)**. Both warm layout targets are evaluated on five alternating main/PH3 pairs, each side followed by its own LMDB control. Ratios use the median of within-trial ratios, rather than dividing unrelated medians. All five trials are retained. The warm read target is met; durable-write p99 non-regression remains uncertain after the additional confirmation block, as detailed below.', '',
        '| Warm 32 MiB layout | Paired PH3/LMDB p50 | Paired PH3/LMDB p99 | Meets ≤0.9 p50 and ≤1 p99 |',
        '| --- | ---: | ---: | --- |']
    lines += [f'| {r["layout"]} | {r["p50_ratio"]:.3f} | {r["p99_ratio"]:.3f} | {r["accepted"]} |' for r in target]
    lines += ['', '## Verification and read path', '',
        'PH3 feature bit 0 appends an IEEE value CRC per version, preserving flag-zero PH3 and PH2 reads. Admission checks the section CRC, every sorted block CRC, exact key/MVCC/address/coverage correspondence, and each stored value CRC. External hits copy exact value bytes and CRC the owned return buffer against immutable validated metadata. Resident full-data hits retain verified owned backing; scans and sorted fallbacks retain block verification. No mapped value is returned unchecked. See [DESIGN.md](DESIGN.md).', '',
        'The resident view preserves the PH3 arena and packs its hash tables (u24 pilots, no redundant fingerprints; full keys still compare). The variable locator buffer is charged exactly to the same block-cache budget; fixed table/index bookkeeping remains outside that byte accounting. Actual 32 MiB locator charges are 32,601,232 bytes bulk and 32,503,515 bytes normal. Locator mode retains no whole-SST heap backing for values; per-call owned return buffers remain necessary. Temporary open allocations, allocator retention and the OS page cache still contribute to RSS. Inspection receipts include peak/RSS and open time.', '',
        'Admitted point indexes compare complete keys directly and skip redundant Bloom probes; the Bloom section is still verified at open and remains in the sorted fallback. The immutable Version caches its L0 prefix boundary, avoiding a per-point walk across every leveled table merely to filter L0. Probe order and cross-table sequence resolution remain unchanged. mmap_max_level retains its existing level-selection and best-effort fallback semantics. The option is explicitly enabled for mapped rows, and remains None for pread rows. Only CRC-bearing PH3 tables qualify for locator-only admission; existing PH2 and flag-zero PH3 remain readable through their original full-resident or sorted paths. Reservations are decided when a table handle opens and are not dynamically promoted after another snapshot or compaction releases budget.', '',
        '## Measurement conditions', '',
        'Mode labels refer to Kiban; LMDB always uses its native mmap-backed engine. Warm rows run 100,000 point calls and 100,000 latest-ten calls per trial, with complete 1M-record verification. Main indexed fixtures are PH2; candidate indexed fixtures use PH3+CRC, prepared from the same seed and logical workload. Normal compaction topology and bytes change with the section size; the measurements include that effect. Index-off fixtures and LMDB fixtures are shared. The computed default is 449,030,592 bytes on this host (about 428.23 MiB); explicit 512 MiB is separate.', '',
        'Cold rows use cold.rs: post-open madvise(DONTNEED) and file-specific posix_fadvise(DONTNEED), then 10,000 point calls with an oracle check on each result. Pages warm during those calls. Four readers share CPU 6 and exercise I/O concurrency, not four-core CPU scaling. Storage bytes/faults are retained. Main can retain verified full backing for a subset of normal-layout tables even at 32 MiB; the new locator mode uses that budget for metadata instead. Default/512 index-on configurations retain full verified heap values through file-page eviction, so those rows are retained-resident tests, not cold-storage speedups. Open time is excluded from latency, and locator validation reads the full data region at open.', '',
        'All latency tables use microseconds, median of five trials. The adjusted PH3/main columns divide each side by its adjacent LMDB control before comparing the paired results. This shows host drift but does not remove every source of noise; full min/max ranges are linked. A value above one in an adjusted column is a measured relative regression, not hidden by the headline target.']
    for mode in ['mapped','pread']:
        lines += ['', f'## Warm point: {mode}', '', 'Mapped means Some(u32::MAX); pread means None.']
        lines += table([r for r in results[mode+'-read'] if r['scenario'][-1]=='recall-by-id'], mode+'-read')
        lines += ['', f'## Warm latest-ten: {mode}']
        lines += table([r for r in results[mode+'-read'] if r['scenario'][-1]=='recent-for-agent'], mode+'-read')
        lines += ['', f'## Cold-start point: {mode}', '', 'Default/512 index-on rows retain heap values.']
        lines += table(results[mode+'-cold'], mode+'-cold',True)
    lines += ['', '## 10k writes', '',
        'Each trial starts from a fresh 10k-record fixture and uses 10,000 operations per harness stage, both fast and durable modes, with index off/on. The write helper keeps its existing default cache/mapping settings; these populations fit the cache. Remember and update each perform 10,000 writes; mixed-90-10 performs 1,000 writes and 9,000 reads, with separate histograms retained in raw receipts. Flush/sync/final verification are outside call timers and retained in completion receipts.', '',
        '| Index / durable / stage | Main p50/p99 | PH3 p50/p99 | LMDB p50/p99 | Adjusted PH3/main p50/p99 |',
        '| --- | ---: | ---: | ---: | ---: |']
    for r in results['write']:
        _,index,_,_,durable,stage=r['scenario'];d=r['distributions']
        lines.append(f'| {index} / {durable} / {stage} | {pair(d,"before-kiban")} | {pair(d,"ph3-kiban")} | {pair(d,"ph3-lmdb")} | {normalized(r,"p50_ns"):.3f} / {normalized(r,"p99_ns"):.3f} |')
    ends = {}
    for path in (OUT/'ph3-write').glob('*.jsonl'):
        match = re.fullmatch(r'bulk-index([01])-cache32-readers1-durable([01])-r([1-5])-(before|ph3)-(kiban|lmdb)', path.stem)
        if not match:
            continue
        index,durable,trial,side,engine = match.groups()
        row = next(r for r in map(json.loads,path.read_text().splitlines()) if r['type']=='complete')
        ends.setdefault((index,durable,side,engine),[]).append(row['end_sync_flush_verify_seconds']*1000)
    assert len(ends)==16 and all(len(v)==5 for v in ends.values())
    lines += ['', '| Index / durable | Main end sync+flush+verify ms | PH3 ms | LMDB main-control ms | LMDB candidate-control ms |',
              '| --- | ---: | ---: | ---: | ---: |']
    for index in ['0','1']:
        for durable in ['0','1']:
            vals=[median(ends[index,durable,side,engine]) for side,engine in [('before','kiban'),('ph3','kiban'),('before','lmdb'),('ph3','lmdb')]]
            lines.append(f'| {index} / {durable} | '+' | '.join(f'{v:.3f}' for v in vals)+' |')
    lines += ['', '### Write confirmation', '',
        'A second five-pair block uses the same active database path for both labels. It confirms fast indexed insert/update controls and repeats the uncertain unindexed durable case. The original run is retained in full; neither run replaces the other.', '',
        '| Index / durable / stage | Confirm main p50/p99 | Confirm PH3 p50/p99 | LMDB p50/p99 | Adjusted PH3/main p50/p99 |',
        '| --- | ---: | ---: | ---: | ---: |']
    for r in load('ph3-write-confirm/summary.json'):
        if r['scenario'][-1] not in ['remember','update']:
            continue
        _,index,_,_,durable,stage=r['scenario'];d=r['distributions']
        lines.append(f'| {index} / {durable} / {stage} | {pair(d,"before-kiban")} | {pair(d,"ph3-kiban")} | {pair(d,"ph3-lmdb")} | {normalized(r,"p50_ns"):.3f} / {normalized(r,"p99_ns"):.3f} |')
    lines += ['', 'The fast indexed update slowdown does not reproduce: confirmation p50/p99 improves 0.848/5.504 to 0.816/5.248 us. Durable unindexed insert p99 changes direction (original 3.080 to 7.078 ms; confirmation 5.636 to 3.932 ms). Durable update confirmation p50 stays close (557 to 573 us), but p99 remains higher (3.539 to 3.932 ms; adjusted ratio 1.140). The host data do not establish a durable-write p99 no-regression guarantee. All stored values and final 20k-record states verify. This uncertainty is separate from the passing warm read target and passing correctness gates.', '',
        '[Write trials and ranges](ph3-write/SUMMARY.md), [confirmation and ranges](ph3-write-confirm/SUMMARY.md).', '', '## Profiling', '',
        'Round 2 already rejected verified mapped points: bulk/32 improved 3.840/6.528 to 2.432/3.584 us and normal/32 4.096/7.296 to 2.688/4.352, but all remained behind LMDB; normal/512 p50 rose 2.368 to 2.688 us. Its four-reader cold mapped p99 was 2949.120 us versus pread 589.824 us in the same prototype. Those prior results use CPU 3 and are historical, not comparable absolute timings for this CPU-6 campaign. This implementation retains full verified heap backing when the dataset fits, uses resident exact locators when it does not, and checks every returned external value rather than caching a per-block verification bit. The new cold/larger-cache matrices are required to judge the old tradeoffs. See [round-2 rejection](../read-path-results/round2/REJECTED.md).', '',
        'The original 32 MiB sorted path used about 9,916/10,432 user cycles and 13,881/15,447 instructions per bulk/normal operation, versus LMDB 5,154/5,614 cycles and 7,893 instructions. Exclusive samples placed about 13% in cache insertion, 8–13% in block loading and 5% in block CRC, with routing/index/seek work in the inlined getter. See baseline-profile and profile-summary.json.', '',
        'The rejected PH2-derived path already used per-value CRCs, derived at open. Its remaining mapped profile put 18–22% of samples in memmove and about 2–3% in CRC, with locator reconstruction in the inlined getter. Normal routing sampled at 11.86%. PH3 removes key reconstruction, and the L0 prefix removes a topology-sized filter walk. These samples identify hot code; they are not exact nanosecond attribution. DWARF reconstruction was incomplete and perf lost samples; raw coverage/loss receipts are preserved. User-only counters exclude kernel syscall instructions.', '',
        'Full-path final counters and stacks: ph3-pread-profile and ph3-mapped-profile. Syscall interposition is diagnostic only: it counts positioned-read calls/bytes/time inside the perf FIFO operation boundary and includes clock overhead. It is never enabled for latency trials. The isolated component results overlap and must not be added into a full-read prediction.']
    expected={f'{side}-{layout}-{engine}' for side in ['before','ph3'] for layout in ['bulk','normal'] for engine in ['kiban','lmdb']}
    diagnostic=[]
    for mode in ['pread','mapped']:
        folder=OUT/f'ph3-{mode}-profile'
        assert {p.stem for p in folder.glob('*.stat')} == expected, folder
        for p in sorted(folder.glob('*.stat')):
            receipt=next(json.loads(x) for x in p.with_suffix('.log').read_text().splitlines() if x.startswith('{'))
            counters={x.split(';')[2]:float(x.split(';')[0])/receipt['operations'] for x in p.read_text().splitlines() if ';;cpu_core/' in x}
            diagnostic.append(dict(mode=mode,name=p.stem,ns_per_op=receipt['ns']/receipt['operations'],events_per_op=counters))
    (OUT/'ph3-profile-summary.json').write_text(json.dumps(diagnostic,indent=2)+'\n')
    lines += ['', '| Final profile / mode | Loop mean us | User cycles/op | Instructions/op | LLC misses/op | Branch misses/op |', '| --- | ---: | ---: | ---: | ---: | ---: |']
    for r in diagnostic:
        e=r['events_per_op'];lines.append(f'| {r["name"]} / {r["mode"]} | {r["ns_per_op"]/1000:.3f} | {e["cpu_core/cycles/u"]:.1f} | {e["cpu_core/instructions/u"]:.1f} | {e["cpu_core/LLC-load-misses/u"]:.3f} | {e["cpu_core/branch-misses/u"]:.3f} |')
    lines += ['', '| Syscall probe / mode | pread calls / million | Returned bytes / million | Diagnostic pread ns/call |','| --- | ---: | ---: | ---: |']
    for mode in ['pread','mapped']:
        folder=OUT/f'ph3-{mode}-syscalls'
        assert {p.stem for p in folder.glob('*.log')} == expected, folder
        for p in sorted(folder.glob('*.log')):
            match=re.search(r'SYSCALL_PROBE calls=(\d+) bytes=(\d+) ns=(\d+)',p.read_text())
            assert match,p
            calls,size,ns=map(int,match.groups());lines.append(f'| {p.stem} / {mode} | {calls} | {size} | {ns/calls if calls else 0:.1f} |')
    lines += ['', '| Isolated unchanged-path component | ns/op |','| --- | ---: |']
    for r in load('final-components/summary.json'):
        lines.append(f'| {r["name"]} | {r["ns_per_op"]:.1f} |')
    lines += ['', '## Rejected experiments and gates', '',
        'The private PH2-derived locator candidate is preserved only as experiments/ph2-locators.patch. Its final pread paired K/LMDB point p50 ratios were 1.065 bulk and 1.532 normal, missing the stricter target. Earlier bulk pilot p50/p99 improved 3.712/5.504 to 1.536/2.176 us, versus LMDB 1.344/1.824; near parity was explicitly rejected. Historical final-* directories, development receipts and old source hashes refer to that experiment, not this PH3 source.', '',
        'The first frozen PH3 candidate, with the Bloom precheck retained, missed the full normal target: paired p50/p99 ratios 0.930/0.864, despite bulk 0.800/0.759. The pipeline stopped before cold/writes; all 240 warm commands remain in ph3-before-bloom-read. The direct no-Bloom pilot against that source reduced bulk paired p50 K/L from 0.857 to 0.732 and normal from 0.829 to 0.791. Its p99 ratios remain below LMDB, and all trials are retained in no-bloom-pilot. The earlier PH3 mapped pilot before the L0 change measured 1.152/1.536 us bulk versus LMDB 1.504/2.240 and 1.248/1.600 us normal versus LMDB 1.440/2.176; paired p50 ratios 0.795/0.886. It was superseded by the frozen full matrix above. A missing-SST classification failure in the earlier candidate was fixed at the admission prepass root, preserving DbError::Corrupt. It was a caught development regression, not an existing-engine integrity bug. decisions.tsv records the parent clarification and fix.', '',
        'All 11 final-source gates passed: fmt, clippy -D warnings, debug and release workspace tests (353 library tests), fixed crash corpora, loom, site facts, readbench checks, and both 20,000-seed suites. New tests cover mixed PH2/PH3 flags, snapshots, tombstones, binary/empty keys, address widths, checksum metadata corruption with recomputed section CRC, pre-open backing corruption, post-open repeated mapped/pread value corruption, short pread, concurrent reads after unlink, address-space eligibility, and exact reservation release. See [gate receipts](ph3-checks/status.json), [source hashes](ph3-checks/source-hashes.json), [provenance](ph3-provenance.json), [design](DESIGN.md), and [reproduction](VERIFY.md).', '',
        'Recommendation: keep mmap_max_level opt-in. Warm mapped point targets pass, but normal/32 single-reader file-page-cold p99 rises 557.056 to 720.896 us versus main (about 29%), while its four-reader p99 improves 2686.976 to 2097.152 us. Both remain below their LMDB controls. The single-reader candidate reads 172,793,856 storage bytes versus main 146,546,688; admission retention and changed SST topology differ, so this is an observed tradeoff rather than an isolated causal attribution. Across modes, the candidate four-reader cold p99 is also higher with mmap: bulk 1540.096 us versus pread 434.176 us, normal 2097.152 us versus pread 491.520 us (about 3.5x and 4.3x). The existing option lets deployments choose this warm-page-cache advantage explicitly, and the report separately measures pread. Open-time validation, mixed old tables, and snapshot-dependent admission also argue against a default change. The complete acceptance decision and any other material controls are recorded in decisions.tsv.']
    (OUT/'REPORT.md').write_text('\n'.join(lines)+'\n')

if __name__ == '__main__':
    main()
