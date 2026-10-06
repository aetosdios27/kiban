#!/usr/bin/env python3
"""Read preserved CSV evidence; report observed trends without assigning causes."""
import argparse
import csv
import json
import pathlib

p = argparse.ArgumentParser()
p.add_argument("root", type=pathlib.Path)
a = p.parse_args()
for summary in sorted(a.root.rglob("summary.csv")):
    run = summary.parent
    with summary.open() as f:
        totals = [r for r in csv.DictReader(f) if r["stage"] == "total"]
    if not totals:
        continue
    with (run / "samples.csv").open() as f:
        samples = list(csv.DictReader(f))
    r = totals[0]
    final = samples[-1]
    timed = [s for s in samples if int(s["reads"]) + int(s["writes"]) > 0]
    split = max(1, len(timed) // 3)
    def rate(window):
        dt = sum(float(s["interval_s"]) for s in window)
        return sum(int(s["reads"]) + int(s["writes"]) for s in window) / dt if dt else None
    def peak(field):
        return max((float(s[field]) for s in samples if s[field]), default=None)
    print(json.dumps({"run": str(run), "operations": int(r["operations"]), "seconds": float(r["elapsed_s"]),
                      "ops_s": float(r["total_ops_s"]), "read_p99_ns": r["read_p99_ns"], "write_p99_ns": r["write_p99_ns"],
                      "first_third_ops_s": rate(timed[:split]), "last_third_ops_s": rate(timed[-split:]),
                      "peak_l0": peak("l0_files"), "peak_debt_bytes": peak("debt_bytes"), "final_debt_bytes": int(final["debt_bytes"]),
                      "peak_rss_bytes": peak("rss_bytes"), "peak_fds": peak("fds"), "final_sst_write_amp": final["sst_write_amp"],
                      "stalls": int(final["stalls"]), "stall_ns": int(final["stall_ns"]),
                      "process_read_bytes": final["process_read_bytes"], "process_write_bytes": final["process_write_bytes"]}))
