#!/usr/bin/env python3
"""Trace one verified durable write command per LSM after benchmark timing."""
import argparse
from collections import defaultdict
import gzip
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from fjall_bench import BUILD, ROOT, SUPPLEMENT, audit_command, configs, digest, dump, lines, run

SYSCALLS = ("write", "writev", "pwrite64", "pwritev", "pwritev2", "fdatasync", "fsync")
WRITES = set(SYSCALLS[:5])
CALL = re.compile(r"^(\d+)\s+(\w+)\(\d+(?:<([^>]+)>)?.*\)\s+=\s+(-?\d+)")
RESUMED = re.compile(r"^(\d+)\s+<\.\.\. (\w+) resumed>(.*)")


def calls(trace):
    pending = {}
    for line in trace.splitlines():
        if line.endswith("<unfinished ...>"):
            tid, body = line.split(maxsplit=1)
            assert tid not in pending
            pending[tid] = body.removesuffix("<unfinished ...>")
            continue
        resumed = RESUMED.match(line)
        if resumed:
            tid, syscall, rest = resumed.groups()
            body = pending.pop(tid)
            assert body.startswith(syscall + "(")
            line = f"{tid} {body}{rest}"
        match = CALL.match(line)
        if match:
            _, syscall, fd_path, result = match.groups()
            assert syscall in SYSCALLS and int(result) >= 0, line
            yield syscall, fd_path or "", int(result)
        else:
            assert not re.match(r"^\d+\s+\w+\(", line), line
    assert not pending


def summarize_trace(trace, suffix, output, stages):
    results = defaultdict(lambda: defaultdict(lambda: dict(calls=0, bytes=0)))
    stage_counts = []
    stage_syncs = 0
    mutation_writes_started = False
    for syscall, fd_path, count in calls(trace):
        journal = fd_path.endswith(suffix)
        if syscall == "write" and fd_path == output and len(stage_counts) < len(stages):
            stage_counts.append(stage_syncs)
            stage_syncs = 0
        if journal and syscall in WRITES:
            mutation_writes_started = True
        if journal and syscall == "fdatasync":
            stage_syncs += 1
        # The existing journal is synced once on open before its first append.
        in_stages = mutation_writes_started and len(stage_counts) < len(stages)
        group = "journal_mutations" if journal and in_stages else "other"
        tally = results[group][syscall]
        tally["calls"] += 1
        tally["bytes"] += count if syscall in WRITES else 0
    assert stage_counts == [stage["writes"]["count"] for stage in stages], stage_counts
    summary = trace[trace.index("% time"):].strip() + "\n"
    totals = {field: sum(group[field]["calls"] for group in results.values() if field in group)
              for field in SYSCALLS}
    for line in summary.splitlines():
        fields = line.split()
        if fields and fields[-1] in SYSCALLS:
            assert len(fields) == 5 and int(fields[3]) == totals.pop(fields[-1]), line
    assert not any(totals.values()), totals
    return results, stage_counts, summary


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--strace", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=ROOT / "validation/fjall")
    args = parser.parse_args()
    assert set(os.sched_getaffinity(0)) <= set(range(13, 22)) and os.nice(0) >= 10
    strace = args.strace.resolve()
    out = args.out.resolve()
    folder = out / "checks/durable-syscalls"
    folder.mkdir(parents=True, exist_ok=True)
    raw = out / "raw/screen-supplement"
    selection = json.loads((raw / "selection.json").read_text())
    metadata = json.loads((raw / "metadata.json").read_text())
    binary = Path(metadata["binary"])
    assert digest(binary) == metadata["binary_sha256"]
    version = subprocess.check_output([strace, "-V"], text=True).splitlines()[0]
    counts = {}
    traces = {}
    for label, config in configs("write", "bulk", None, selection, candidates=SUPPLEMENT):
        if label not in ("kiban-index1", "fjall-tuned"):
            continue
        path = folder / f"{label}.trace.txt"
        zipped = path.with_suffix(".txt.gz")
        trace = [str(strace), "-f", "-C", "-s", "0", "-y", "-o", str(path),
                 "-e", "trace=" + ",".join(SYSCALLS)]
        run(dict(config, action="write", durable=True, operations=1000), folder, label, trace=trace)
        receipt = next(row for row in lines(folder / "commands.jsonl") if row["name"] == label)
        c, rows = audit_command(folder, receipt)
        stages = [row for row in rows if row["type"] == "stage"]
        suffix = ".wal" if label == "kiban-index1" else ".jnl"
        if path.exists():
            raw_trace = path.read_text()
        else:
            raw_trace = gzip.decompress(zipped.read_bytes()).decode()
        totals, stage_syncs, summary = summarize_trace(raw_trace, suffix, c["output"], stages)
        (folder / f"{label}.strace-c.txt").write_text(summary)
        if path.exists():
            with path.open("rb") as source, zipped.open("wb") as output:
                with gzip.GzipFile(filename="", fileobj=output, mode="wb", mtime=0) as stream:
                    shutil.copyfileobj(source, stream)
            path.unlink()
        counts[label] = dict(totals, journal_fdatasync_by_stage=stage_syncs)
        traces[label] = dict(argv=[*trace, str(binary), str(folder / f"{label}.config.json")],
                             sha256=digest(zipped))
    dump(folder / "summary.json", dict(strace=version, strace_sha256=digest(strace),
                                       binary_sha256=digest(binary), traces=traces, counts=counts))


if __name__ == "__main__":
    main()
