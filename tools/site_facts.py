#!/usr/bin/env python3
"""Validate reviewed site claims and derive a versioned feed from this checkout."""

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
import textwrap
import tomllib
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[1]
IDENTIFIER = r"[a-zA-Z_][a-zA-Z_0-9]*"
TEST = re.compile(
    rf"#\[test\]\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+({IDENTIFIER})\b"
)


class Invalid(ValueError):
    """A source or feed does not meet the published contract."""


def require(condition, message):
    if not condition:
        raise Invalid(message)


def read_json(path):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, f"{path}: duplicate JSON key {key!r}")
            result[key] = value
        return result

    return json.loads(Path(path).read_text(), object_pairs_hook=pairs)


def fields(value, required, optional=(), where="object"):
    require(isinstance(value, dict), f"{where}: expected object")
    require(set(required) <= value.keys(), f"{where}: missing fields {set(required) - value.keys()}")
    require(value.keys() <= set(required) | set(optional), f"{where}: unknown fields {value.keys() - set(required) - set(optional)}")


def string(value, where):
    require(isinstance(value, str) and value.strip() == value and bool(value), f"{where}: expected nonempty trimmed string")


def integer(value, where, minimum=0):
    require(type(value) is int and value >= minimum, f"{where}: expected integer >= {minimum}")


def array(value, where, nonempty=False):
    require(isinstance(value, list), f"{where}: expected array")
    require(not nonempty or bool(value), f"{where}: must not be empty")


def unique(values, where):
    require(len(values) == len(set(values)), f"{where}: duplicate values")


def date(value, where):
    string(value, where)
    require(re.fullmatch(r"\d{4}-\d{2}-\d{2}", value), f"{where}: expected YYYY-MM-DD")
    try:
        dt.date.fromisoformat(value)
    except ValueError as error:
        raise Invalid(f"{where}: invalid date {value!r}") from error


def rust_code(source):
    """Mask comments and strings, retaining offsets and syntax for source checks."""
    pattern = re.compile(r'//[^\n]*|/\*|r(\#*)"|"(?:\\.|[^"\\])*"', re.S)
    chars = list(source)
    position = 0
    while match := pattern.search(source, position):
        start, end = match.span()
        token = match.group()
        if token == "/*":
            depth = 1
            while depth:
                nested = re.search(r"/\*|\*/", source[end:])
                require(nested is not None, "unclosed Rust block comment")
                depth += 1 if nested.group() == "/*" else -1
                end += nested.end()
        elif token.startswith('r'):
            closing = '"' + match.group(1)
            close = source.find(closing, end)
            require(close >= 0, "unclosed Rust raw string")
            end = close + len(closing)
        chars[start:end] = ["\n" if c == "\n" else " " for c in source[start:end]]
        position = end
    return "".join(chars)


def rust_sources(root, directories=("src", "tests")):
    return [(path, rust_code(path.read_text())) for directory in directories
            for path in sorted((root / directory).rglob("*.rs"))]


STAT_REFS = {
    "tests.crash_seeds_per_pr": lambda facts: facts["tests"]["crash_seeds_per_pr"],
    "tests.crashes_per_seed": lambda facts: facts["tests"]["crashes_per_seed"],
    "tests.lib_tests": lambda facts: facts["tests"]["lib_tests"],
    "tests.nightly_seeds_total": lambda facts: facts["tests"]["nightly_seeds_total"],
    "bugs_found.length": lambda facts: len(facts["bugs_found"]),
    "local_sweeps.max_seeds": lambda facts: max((s["seeds"] for s in facts["local_sweeps"]), default=0),
}


def resolve_stats(facts):
    facts["correctness_stats"] = [
        {"value": STAT_REFS[c["value"]["ref"]](facts) if isinstance(c["value"], dict) else c["value"], "label": c["label"]}
        for c in facts["correctness_stats"]
    ]
    return facts


def validate_claims(claims, names):
    fields(claims, ("specs", "guarantees", "bugs_found", "capabilities", "roadmap", "local_sweeps", "correctness_stats"), where="claims")
    array(claims["specs"], "specs", True)
    for spec in claims["specs"]:
        string(spec, "specs[]")
    unique(claims["specs"], "specs")
    fields(claims["guarantees"], ("promised", "not_promised", "not_built"), where="guarantees")
    keys = []
    for group, entries in claims["guarantees"].items():
        array(entries, f"guarantees.{group}")
        for entry in entries:
            where = f"guarantees.{group}"
            fields(entry, ("key", "clause"), ("evidence", "detail"), where)
            for key in ("key", "clause"):
                string(entry[key], f"{where}.{key}")
                require("\n" not in entry[key], f"{where}.{key}: expected one line")
            require(len(entry["key"]) <= 32, f"{where}.key: use a short label (<=32 characters)")
            require(len(entry["clause"]) <= 64, f"{where}.clause: keep it terse (<=64 characters); put precise wording in detail")
            if "detail" in entry:
                string(entry["detail"], f"{where}.detail")
            keys.append(entry["key"])
            require(group != "promised" or "evidence" in entry, f"{where}.{entry['key']}: evidence required")
            if "evidence" in entry:
                array(entry["evidence"], f"{where}.evidence", True)
                for name in entry["evidence"]:
                    string(name, f"{where}.evidence[]")
                    require(re.fullmatch(IDENTIFIER, name), f"invalid evidence name {name!r}")
                    require(name in names, f"{where}.{entry['key']}: evidence fn {name!r} missing from src/ or tests/")
                unique(entry["evidence"], f"{where}.evidence")
    unique(keys, "guarantee keys")
    array(claims["bugs_found"], "bugs_found")
    for entry in claims["bugs_found"]:
        fields(entry, ("date", "title", "pr"), where="bugs_found[]")
        date(entry["date"], "bugs_found.date")
        string(entry["title"], "bugs_found.title")
        integer(entry["pr"], "bugs_found.pr", 1)
    for group, label, statuses, optional in (
        ("capabilities", "label", {"shipped", "in_review", "planned"}, ("link",)),
        ("roadmap", "title", {"done", "active", "next", "planned"}, ()),
    ):
        array(claims[group], group)
        ids = []
        for entry in claims[group]:
            fields(entry, ("id", label, "status"), optional, group)
            string(entry["id"], f"{group}.id")
            require(re.fullmatch(r"[a-z][a-z0-9-]*", entry["id"]), f"{group}.id: expected slug")
            ids.append(entry["id"])
            string(entry[label], f"{group}.{label}")
            string(entry["status"], f"{group}.status")
            require(entry["status"] in statuses, f"{group}: invalid status {entry['status']!r}")
            if "link" in entry:
                string(entry["link"], f"{group}.link")
                url = urlparse(entry["link"])
                require(url.scheme == "https" and bool(url.netloc), f"{group}.link: expected HTTPS URL")
        unique(ids, f"{group} ids")
    array(claims["correctness_stats"], "correctness_stats")
    require(len(claims["correctness_stats"]) == 6, "correctness_stats: expected exactly 6 cells (the site's 2x3 grid)")
    for cell in claims["correctness_stats"]:
        fields(cell, ("value", "label"), where="correctness_stats[]")
        string(cell["label"], "correctness_stats.label")
        value = cell["value"]
        if isinstance(value, dict):
            fields(value, ("ref",), where="correctness_stats.value")
            require(value["ref"] in STAT_REFS, f"correctness_stats: unknown ref {value['ref']!r}; expected one of {sorted(STAT_REFS)}")
        else:
            require(isinstance(value, (int, str)) and not isinstance(value, bool), "correctness_stats.value: expected integer, string or {ref}")
    array(claims["local_sweeps"], "local_sweeps")
    for entry in claims["local_sweeps"]:
        fields(entry, ("date", "commit", "suite", "seeds", "result"), where="local_sweeps[]")
        date(entry["date"], "local_sweeps.date")
        for key in ("commit", "suite", "result"):
            string(entry[key], f"local_sweeps.{key}")
        require(re.fullmatch(r"[0-9a-f]{40}", entry["commit"]), "local_sweeps.commit: expected full Git SHA")
        integer(entry["seeds"], "local_sweeps.seeds", 1)


def check_specs(root, claims):
    cargo = tomllib.loads((root / "Cargo.toml").read_text())
    package = cargo["package"]
    deps = dict(cargo.get("dependencies", {}))
    for target, settings in cargo.get("target", {}).items():
        deps.update({f"{target}:{name}": dep for name, dep in settings.get("dependencies", {}).items()})
    defaults = cargo.get("features", {}).get("default", [])
    require(not defaults, "default features changed; review runtime dependency claim/check")
    require(all(isinstance(dep, dict) and dep.get("optional") is True for dep in deps.values()), "default runtime dependencies changed; review specs")
    require("targets Linux" in (root / "README.md").read_text(), "declared Linux target changed; review specs")
    expected = {f"Rust {package['edition']}", "no runtime dependencies by default", "Linux", package["license"]}
    require(set(claims["specs"]) == expected, f"specs must match Cargo/Linux metadata: {sorted(expected)}")
    return package


def extract_examples(root):
    examples = {}
    paths = [path for directory in ("src", "tests", "examples")
             for path in sorted((root / directory).rglob("*")) if path.suffix in {".rs", ".py"}]
    # Future binding tests may live outside tests/ (there are none on main).
    paths += sorted((root / "bindings").rglob("test*.py"))
    for path in paths:
        active = None
        lines = []
        for number, line in enumerate(path.read_text().splitlines(keepends=True), 1):
            prefix = r"//" if path.suffix == ".rs" else "#"
            marker = re.fullmatch(rf"\s*{prefix} site:(begin ([a-z][a-z0-9-]*)|end)\s*", line)
            if marker:
                if marker[1] == "end":
                    require(active is not None, f"{path}:{number}: site:end without begin")
                    examples[active] = textwrap.dedent("".join(lines))
                    require(bool(examples[active].strip()), f"{path}:{number}: empty region {active}")
                    active = None
                else:
                    require(active is None, f"{path}:{number}: nested site region")
                    active = marker[2]
                    require(active not in examples, f"{path}:{number}: duplicate site id {active}")
                    lines = []
            elif re.match(rf"\s*{prefix}\s*site:", line):
                raise Invalid(f"{path}:{number}: malformed site marker")
            elif active is not None:
                lines.append(line)
        require(active is None, f"{path}: unclosed site region {active}")
    require({"basic", "batch", "range", "snapshot", "shared"} <= examples.keys(), "missing required site tabs: basic, batch, range, snapshot, shared")
    if (root / "bindings/python/tests/test_kiban.py").exists():
        require("python" in examples, "missing required Python site tab")
    return dict(sorted(examples.items()))


def function_body(code, name):
    match = re.search(rf"\bfn\s+{re.escape(name)}\b[^{{]*{{", code)
    require(match is not None, f"simulation source: missing fn {name}")
    start, depth = match.end(), 1
    for end in range(start, len(code)):
        depth += (code[end] == "{") - (code[end] == "}")
        if depth == 0:
            return code[start:end]
    raise Invalid(f"simulation source: unclosed fn {name}")


def simulation_metrics(source):
    code = rust_code(source)
    corpora = re.findall(r"\bfn\s+(\w*seeded_crash_fixed_corpus)\b", code)
    require(len(corpora) == 2, "simulation source: expected direct and shared fixed corpora; review parser")
    seeds = 0
    for name in corpora:
        body = function_body(code, name)
        match = re.fullmatch(r"\s*for seed in (\d+)\.\.(\d+)\s*\{\s*run_seed\(seed, EngineMode::(?:Direct|Shared)\)\.unwrap_or_else\(.*?\);\s*\}\s*", body, re.S)
        require(match is not None, f"simulation source: cannot parse fixed corpus {name}; review parser")
        count = int(match[2]) - int(match[1])
        require(count > 0, f"simulation source: empty corpus {name}")
        seeds += count
    body = function_body(code, "run_seed")
    rounds = re.findall(r"for round in (\d+)\.\.(\d+)", body)
    require(len(rounds) == 1 and body.count("sys::power_loss_seeded(") == 1 and body.count("wal_creation_probe(") == 1, "simulation source: crash loop changed; review parser")
    probe = function_body(code, "wal_creation_probe")
    require(probe.count("sys::power_loss_seeded(") == 1 and not re.search(r"\b(for|while|loop)\b", probe), "simulation source: WAL probe changed; review parser")
    # Crash rounds in run_seed plus the one WAL-creation probe crash.
    crashes = int(rounds[0][1]) - int(rounds[0][0]) + 1
    require(crashes > 1, "simulation source: empty crash loop")
    return seeds, crashes


def validate(root=ROOT):
    claims = read_json(root / "site/claims.json")
    sources = rust_sources(root)
    names = {name for _, code in sources for name in re.findall(rf"\bfn\s+({IDENTIFIER})\b", code)}
    validate_claims(claims, names)
    package = check_specs(root, claims)
    examples = extract_examples(root)
    metrics = simulation_metrics((root / "src/simulation_tests.rs").read_text())
    return claims, package, examples, metrics


def lib_tests(root, test_list=None):
    if test_list is not None:
        listing = Path(test_list).read_text()
        entries = re.findall(r"^(.+): test$", listing, re.M)
        require(bool(entries), "test list contains no tests; use cargo test --lib -- --list")
        unique(entries, "test list")
        summaries = re.findall(r"^(\d+) tests?, \d+ benchmarks?$", listing, re.M)
        require(len(summaries) == 1 and int(summaries[0]) == len(entries), "test list must contain one complete library test binary")
        return len(entries)
    return sum(len(TEST.findall(code)) for _, code in rust_sources(root, ("src",)))


def benchmarks(root):
    report = "validation/COMPARISON.md"
    source = "validation/comparison-results/summary/comparison.json"
    fallback = {"source": report}
    path = root / source
    if not path.exists():
        print("benchmarks: no machine-readable summary; publishing report source only", file=sys.stderr)
        return fallback
    summary = read_json(path)
    require(isinstance(summary, dict), "benchmark summary: expected object")
    aggregates = summary.get("aggregates")
    if not isinstance(aggregates, list) or not aggregates:
        print("benchmarks: unsupported summary; publishing report source only", file=sys.stderr)
        return fallback
    rows, commits = [], set()
    for row in aggregates:
        require(isinstance(row, dict), "benchmark aggregate: expected object")
        campaign = row.get("campaign", "")
        require(isinstance(campaign, str) and campaign.startswith("validation/comparison-results/") and ".." not in Path(campaign).parts, "benchmark campaign must be a results directory")
        metadata_path = root / campaign / "metadata.json"
        if not metadata_path.exists():
            print("benchmarks: measured revision unavailable; publishing report source only", file=sys.stderr)
            return fallback
        metadata = read_json(metadata_path)
        require(isinstance(metadata, dict), "benchmark metadata: expected object")
        commit = metadata.get("kiban_commit", "")
        if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
            print("benchmarks: measured revision unavailable; publishing report source only", file=sys.stderr)
            return fallback
        commits.add(commit)
        fields_needed = {"case", "engine", "trials", "total_ops_s", "read_p99_us", "write_p99_us"}
        require(fields_needed <= row.keys(), "unsupported benchmark aggregate fields")
        for key in ("case", "engine"):
            string(row[key], f"benchmarks.{key}")
        integer(row["trials"], "benchmarks.trials", 1)
        for key in ("total_ops_s", "read_p99_us", "write_p99_us"):
            metric = row[key]
            if metric is not None:
                fields(metric, ("median", "min", "max"), where=f"benchmarks.{key}")
                require(all(type(n) in (int, float) and 0 <= n < float("inf") for n in metric.values()), f"invalid benchmark metric {key}")
                require(metric["min"] <= metric["median"] <= metric["max"], f"invalid benchmark range {key}")
        rows.append({"campaign": campaign, "case": row["case"], "engine": row["engine"], "trials": row["trials"], "ops_per_second": row["total_ops_s"], "read_p99_us": row["read_p99_us"], "write_p99_us": row["write_p99_us"]})
    require(len(commits) == 1, "benchmark campaigns have different engine commits; revise schema before merging")
    return {"source": report, "data_source": source, "commit": commits.pop(), "rows": rows}


def nightly_totals(previous, additional, run_id):
    integer(additional, "--add-nightly-seeds")
    total, ids = 0, []
    if previous is not None:
        require(isinstance(previous, dict), "previous facts: expected object")
        require(type(previous.get("schema")) is int and previous["schema"] == 1, "previous facts: expected schema 1")
        tests = previous.get("tests")
        require(isinstance(tests, dict), "previous facts: missing tests")
        total = tests.get("nightly_seeds_total")
        ids = tests.get("nightly_run_ids")
        integer(total, "previous nightly_seeds_total")
        array(ids, "previous nightly_run_ids")
        for item in ids:
            string(item, "nightly_run_ids[]")
        unique(ids, "nightly_run_ids")
    if additional:
        string(run_id, "--nightly-run-id (required with --add-nightly-seeds)")
        if run_id not in ids:
            total += additional
            ids = [*ids, run_id]
    return total, ids




def nightly_receipt(log, environment):
    """Require actual sweep completion lines before issuing a success receipt."""
    count = int(environment["KIBAN_SIM_SEEDS"])
    start = int(environment["KIBAN_SIM_SEED_START"])
    expected = (count, start, start + count)
    completions = [tuple(map(int, match)) for match in re.findall(
        r"\bchecked (\d+) seeds \((\d+)\.\.(\d+)\),", log)]
    require(count > 0 and completions == [expected, expected], "nightly log: expected completed direct and shared seed ranges")
    require(re.search(r"test result: ok\. 2 passed; 0 failed; 0 ignored;", log), "nightly log: expected two successful long tests")
    return {"run_id": environment["GITHUB_RUN_ID"], "commit": environment["GITHUB_SHA"],
            "shard": int(environment["SHARD"]), "shard_count": int(environment["SHARD_COUNT"]),
            "seed_start": start, "seeds_per_suite": count,
            "suites": ["seeded_crash_long", "shared_seeded_crash_long"]}


def nightly_seeds(directory, run_id, commit):
    """Count only a complete set of successful, matching nightly shard receipts."""
    paths = sorted(Path(directory).rglob("receipt.json"))
    require(bool(paths), "nightly receipts: none found")
    shards, shard_counts, total = [], set(), 0
    for path in paths:
        receipt = read_json(path)
        fields(receipt, ("run_id", "commit", "shard", "shard_count", "seed_start", "seeds_per_suite", "suites"), where=str(path))
        require(receipt["run_id"] == run_id and receipt["commit"] == commit, f"{path}: nightly run/revision mismatch")
        integer(receipt["shard"], f"{path}: shard")
        integer(receipt["shard_count"], f"{path}: shard_count", 1)
        require(receipt["shard_count"] <= 64, f"{path}: unexpected shard count")
        integer(receipt["seed_start"], f"{path}: seed_start")
        integer(receipt["seeds_per_suite"], f"{path}: seeds_per_suite", 1)
        require(receipt["suites"] == ["seeded_crash_long", "shared_seeded_crash_long"], f"{path}: expected both seeded suites")
        shards.append(receipt["shard"])
        shard_counts.add(receipt["shard_count"])
        total += receipt["seeds_per_suite"] * len(receipt["suites"])
    unique(shards, "nightly receipt shards")
    require(len(shard_counts) == 1 and set(shards) == set(range(next(iter(shard_counts)))), "nightly receipts: incomplete shard set")
    return total


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def generate(root=ROOT, previous=None, additional=0, run_id=None, test_list=None):
    claims, package, examples, (seeds, crashes) = validate(root)
    total, ids = nightly_totals(previous, additional, run_id)
    return resolve_stats({
        "schema": 1,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "engine": {"version": package["version"], "commit": git(root, "rev-parse", "HEAD"), "commit_date": git(root, "show", "-s", "--format=%cI", "HEAD")},
        **claims,
        "tests": {"lib_tests": lib_tests(root, test_list), "crash_seeds_per_pr": seeds, "crashes_per_seed": crashes, "nightly_seeds_total": total, "nightly_run_ids": ids},
        "examples": examples,
        "benchmarks": benchmarks(root),
    })


def write_facts(path, facts, previous=None):
    if previous is not None:
        comparable = lambda value: {k: v for k, v in value.items() if k != "generated_at"}
        if comparable(facts) == comparable(previous):
            facts = previous
    Path(path).write_text(json.dumps(facts, indent=2, ensure_ascii=False, allow_nan=False) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("validate")
    gen = commands.add_parser("generate")
    gen.add_argument("--out", type=Path, required=True)
    gen.add_argument("--previous", type=Path)
    gen.add_argument("--add-nightly-seeds", type=int, default=0)
    gen.add_argument("--nightly-run-id")
    gen.add_argument("--test-list", type=Path)
    receipts = commands.add_parser("nightly-seeds")
    receipts.add_argument("--receipts", type=Path, required=True)
    receipts.add_argument("--run-id", required=True)
    receipts.add_argument("--commit", required=True)
    receipt = commands.add_parser("nightly-receipt")
    receipt.add_argument("--log", type=Path, required=True)
    receipt.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "validate":
            validate()
            print("site facts: claims, evidence, examples and source counts valid")
        elif args.command == "nightly-receipt":
            receipt = nightly_receipt(args.log.read_text(), os.environ)
            args.out.write_text(json.dumps(receipt) + "\n")
        elif args.command == "nightly-seeds":
            print(nightly_seeds(args.receipts, args.run_id, args.commit))
        else:
            previous = read_json(args.previous) if args.previous else None
            facts = generate(previous=previous, additional=args.add_nightly_seeds, run_id=args.nightly_run_id, test_list=args.test_list)
            write_facts(args.out, facts, previous)
    except (Invalid, OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"site facts: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
