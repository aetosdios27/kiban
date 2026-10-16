#!/usr/bin/env python3
"""Run the unchanged historical concurrency transition against a frozen build.

The receipt supports dirty candidate sources; historical timing is not silently
pooled with this campaign. Data and result directories are exclusively created.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def host():
    paths = {
        "governor": "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
        "battery": "/sys/class/power_supply/BAT0/status",
    }
    return {key: Path(path).read_text().strip() if Path(path).exists() else None for key, path in paths.items()}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--source", type=Path, default=Path(__file__).resolve().parent.parent)
    p.add_argument("--binaries", type=Path, required=True)
    p.add_argument("--data", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--duration", type=int, default=600)
    a = p.parse_args()
    if a.duration < 60:
        p.error("duration must be at least 60 seconds")
    source, binaries, data, output = (path.resolve() for path in [a.source, a.binaries, a.data, a.output])
    if data == output or data.is_relative_to(output) or output.is_relative_to(data):
        p.error("data and results must be separate")
    data.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(data.parent).free < 2 * 1024**3:
        p.error("at least 2 GiB free required")
    filesystem = subprocess.check_output(["findmnt", "-n", "-o", "FSTYPE", "-T", str(data.parent)], text=True).strip()
    if filesystem == "tmpfs":
        p.error("data directory must be on a storage filesystem")
    data.mkdir(exist_ok=False)
    output.mkdir(parents=True, exist_ok=False)
    plan = json.loads((Path(__file__).parent / "foreground-endurance-plan.json").read_text())
    args = next(c["args"] for c in plan["cases"] if c["name"] == "concurrency-transition-10min").copy()
    args["duration"] = a.duration
    paths = [source / "Cargo.toml", source / "Cargo.lock", *source.glob("src/**/*.rs"), *source.glob("examples/**/*.rs")]
    receipt = {"label": a.label, "args": args, "diagnostic": False, "filesystem": filesystem,
               "source_hashes": {str(path.relative_to(source)): digest(path) for path in sorted(paths)},
               "binary_hashes": {name: digest(binaries / name) for name in ["scale", "maintenance_verify"]},
               "host_start": host(), "rustc": subprocess.check_output(["rustc", "-Vv"], text=True)}
    commands = []
    common = ["--db", str(data), "--output", str(output / "trial")]
    common += [item for key, value in args.items() for item in ["--" + key, str(value)]]
    run = [str(binaries / "scale"), *common]
    verify = [str(binaries / "maintenance_verify"), *common]
    receipt["commands"] = [run, verify]
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    for name, command in [("run", run), ("verify", verify)]:
        with (output / (name + ".log")).open("x") as log:
            try:
                result = subprocess.run(command, cwd=source, stdout=log, stderr=subprocess.STDOUT, timeout=a.duration + 300)
                code = result.returncode
            except subprocess.TimeoutExpired:
                code = 124
        commands.append({"name": name, "exit_code": code})
        (output / "results.json").write_text(json.dumps({"commands": commands, "host_end": host()}, indent=2) + "\n")
        if code:
            raise SystemExit(f"{name} failed: {code}; inspect preserved logs")
    with (output / "trial/status.csv").open() as f:
        statuses = list(csv.DictReader(f))
    if not any("complete" in row.values() for row in statuses):
        raise SystemExit("run lacks a complete status")
    rows = list(csv.DictReader((output / "trial/samples.csv").open()))
    windows = {}
    for row in rows:
        if row["stage"] not in ["low-concurrency", "high-concurrency"]:
            continue
        key = (row["stage"], int(float(row["elapsed_s"]) // 60))
        group = windows.setdefault(key, {"stage": key[0], "minute": key[1], "operations": 0, "seconds": 0})
        group["operations"] += int(row["reads"]) + int(row["writes"])
        group["seconds"] += float(row["interval_s"])
    summary = [{**group, "ops_s": group["operations"] / group["seconds"]} for group in windows.values() if group["seconds"] >= 40]
    (output / "windows.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"commands": commands, "windows": summary}, indent=2))


if __name__ == "__main__":
    main()
