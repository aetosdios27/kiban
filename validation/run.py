#!/usr/bin/env python3
"""Generate or execute immutable, explicitly recorded validation campaigns."""
import argparse
import json
import os
import signal
import pathlib
import subprocess
import sys

p = argparse.ArgumentParser()
p.add_argument("campaign", choices=["workloads", "concurrency", "long", "large-memory", "torture"])
p.add_argument("--operations", type=int, default=1_000_000)
p.add_argument("--keys", type=int, default=1_000_000)
p.add_argument("--value-size", type=int, default=128)
p.add_argument("--working-set", type=int)
p.add_argument("--threads", type=int, default=4)
p.add_argument("--read-percent", type=int, default=50)
p.add_argument("--duration", type=int, default=600)
p.add_argument("--seed", type=int, default=42)
p.add_argument("--sync-every", type=int, default=0)
p.add_argument("--data-root", type=pathlib.Path, required=True)
p.add_argument("--output-root", type=pathlib.Path, required=True)
p.add_argument("--timeout", type=int, default=43_200)
p.add_argument("--execute", action="store_true", help="default only prints commands")
a = p.parse_args()
repo = pathlib.Path(__file__).resolve().parent.parent
scale = repo / "target/release/examples/scale"
torture = repo / "target/release/examples/torture"
if min(a.operations, a.keys, a.value_size, a.threads, a.duration, a.timeout) <= 0:
    p.error("numeric sizes and limits must be positive")
working = a.working_set if a.working_set is not None else a.keys
runs = []
def add(label, workload, threads, extra=(), preload=0, ops=None):
    cmd = [str(scale), "--workload", workload, "--operations", str(a.operations if ops is None else ops),
           "--keys", str(a.keys), "--working-set", str(working), "--value-size", str(a.value_size),
           "--threads", str(threads), "--read-percent", str(a.read_percent), "--seed", str(a.seed),
           "--sync-every", str(a.sync_every), "--preload-keys", str(preload),
           "--db", str(a.data_root / label), "--output", str(a.output_root / label), *extra]
    runs.append((label, cmd))
if a.campaign == "workloads":
    for w in ["sequential-write", "random-write", "random-read", "mixed", "update-heavy", "burst-read", "sustained-write", "burst-recovery", "concurrency-jump"]:
        add(w, w, a.threads, preload=a.keys if w in ["random-read", "mixed", "update-heavy", "concurrency-jump"] else 0)
elif a.campaign == "concurrency":
    for n in [1, 2, 4, 8, 16, 32, 64]:
        add(f"mixed-{n}t", "mixed", n, preload=a.keys)
elif a.campaign == "long":
    add(f"sustained-{a.duration}s", "sustained-write", a.threads, ["--duration", str(a.duration)], ops=0)
elif a.campaign == "large-memory":
    # Explicit caller-selected footprint; don't infer RAM and quietly alter parameters.
    add("populate", "sequential-write", 1)
    for cache in ["warm", "reduced"]:
        add(cache, "random-read", a.threads, ["--cache", cache, "--reuse", "true", "--block-cache", "4194304"])
        # Reuse the SAME data population, preserving seed and value shape.
        cmd = runs[-1][1]; cmd[cmd.index("--db") + 1] = str(a.data_root / "populate")
    if a.operations < a.keys:
        p.error("large-memory population needs operations >= keys")
else:
    for mode in ["probes", "simulated", "process"]:
        label = f"torture-{mode}"
        runs.append((label, [str(torture), "--mode", mode, "--seed", str(a.seed), "--cycles", "1000",
                    "--operations", "1000", "--keys", "1000", "--threads", str(a.threads),
                    "--snapshots", "true", "--db", str(a.data_root / label), "--output", str(a.output_root / label)]))
for label, cmd in runs:
    import shlex
    print(shlex.join(cmd), flush=True)
if not a.execute:
    sys.exit(0)
a.output_root.mkdir(parents=True, exist_ok=False)
a.data_root.mkdir(parents=True, exist_ok=True)
subprocess.run(["cargo", "build", "--release", "--examples"], cwd=repo, check=True)
with (a.output_root / "commands.json").open("x") as f:
    json.dump({"campaign": a.campaign, "timeout_s": a.timeout, "runs": runs}, f, indent=2)
failed = False
with (a.output_root / "exit-codes.csv").open("x") as exits:
    exits.write("run,exit_code\n")
    for label, cmd in runs:
        with (a.output_root / f"{label}.log").open("x") as log:
            try:
                proc = subprocess.Popen(cmd, cwd=repo, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                code = proc.wait(timeout=a.timeout)
            except subprocess.TimeoutExpired:
                # Kill this campaign's process group, including torture child processes.
                # Otherwise an engine deadlock in the child could survive the supervisor.
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
                code = 124
            exits.write(f"{label},{code}\n"); exits.flush()
            failed |= code != 0
sys.exit(1 if failed else 0)
