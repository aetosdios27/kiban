"""Behavior checks for the public feed contract and its source extraction."""

import copy
import importlib.util
import json
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest.mock import patch

MODULE = Path(__file__).resolve().parents[1] / "site_facts.py"
spec = importlib.util.spec_from_file_location("site_facts", MODULE)
facts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(facts)


class ClaimsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.claims = facts.read_json(facts.ROOT / "site/claims.json")
        cls.names = {name for group in cls.claims["guarantees"].values()
                     for claim in group for name in claim.get("evidence", [])}

    def test_repository_validates(self):
        claims, package, examples, metrics = facts.validate()
        self.assertEqual(claims, self.claims)
        self.assertEqual(package["version"], "0.1.0")
        self.assertEqual(set(examples), {"basic", "batch", "range", "snapshot", "shared", "python"})
        self.assertEqual(metrics, (128, 4))

    def test_invalid_claims_fail_with_context(self):
        mutations = [
            (lambda c: c.update(extra=[]), "unknown fields"),
            (lambda c: c.pop("roadmap"), "missing fields"),
            (lambda c: c.update(specs="Rust"), "expected array"),
            (lambda c: c["specs"].append(c["specs"][0]), "duplicate"),
            (lambda c: c["guarantees"]["promised"][0].pop("evidence"), "evidence required"),
            (lambda c: c["guarantees"]["promised"][0].update(evidence=[]), "must not be empty"),
            (lambda c: c["guarantees"]["promised"][0].update(evidence=["imaginary_test"]), "imaginary_test"),
            (lambda c: c["guarantees"]["not_built"].append(c["guarantees"]["not_built"][0]), "duplicate"),
            (lambda c: c["capabilities"][0].update(status="maybe"), "invalid status"),
            (lambda c: c["capabilities"][0].update(status=[]), "string"),
            (lambda c: c["capabilities"][0].update(link="http://example.com"), "HTTPS"),
            (lambda c: c["capabilities"].append(c["capabilities"][0]), "duplicate"),
            (lambda c: c["roadmap"][0].update(id="not a slug"), "slug"),
            (lambda c: c["bugs_found"][0].update(date="2026-02-30"), "invalid date"),
            (lambda c: c["bugs_found"][0].update(pr=True), "integer"),
            (lambda c: c["local_sweeps"].append({"date": "2026-10-16", "commit": "abc", "suite": "local", "seeds": 1, "result": "passed"}), "full Git SHA"),
            (lambda c: c["guarantees"]["promised"][0].update(clause="x" * 65), "keep it terse"),
            (lambda c: c["guarantees"]["promised"][0].update(key="k" * 33), "short label"),
            (lambda c: c["guarantees"]["promised"][0].update(detail=[]), "string"),
            (lambda c: c["correctness_stats"].pop(), "exactly 6"),
            (lambda c: c["correctness_stats"][0].update(value={"ref": "tests.imaginary"}), "unknown ref"),
            (lambda c: c["correctness_stats"][0].update(value=True), "expected integer"),
        ]
        for mutate, message in mutations:
            with self.subTest(message=message, mutate=mutate):
                claims = copy.deepcopy(self.claims)
                mutate(claims)
                with self.assertRaisesRegex(facts.Invalid, message):
                    facts.validate_claims(claims, self.names)

    def test_stat_refs_resolve_to_plain_values(self):
        generated = facts.resolve_stats({
            "tests": {"crash_seeds_per_pr": 128, "crashes_per_seed": 4, "lib_tests": 300, "nightly_seeds_total": 0},
            "bugs_found": [{}, {}],
            "local_sweeps": [{"seeds": 10}, {"seeds": 30}],
            "correctness_stats": [
                {"value": {"ref": "tests.crash_seeds_per_pr"}, "label": "a"},
                {"value": {"ref": "bugs_found.length"}, "label": "b"},
                {"value": {"ref": "local_sweeps.max_seeds"}, "label": "c"},
                {"value": 1000, "label": "d"},
            ],
        })
        self.assertEqual([c["value"] for c in generated["correctness_stats"]], [128, 2, 30, 1000])

    def test_duplicate_json_keys_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "claims.json"
            path.write_text('{"specs": [], "specs": []}')
            with self.assertRaisesRegex(facts.Invalid, "duplicate JSON key"):
                facts.read_json(path)

    def test_specs_follow_cargo(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text((facts.ROOT / "Cargo.toml").read_text().replace('edition = "2024"', 'edition = "2021"'))
            (root / "README.md").write_text("targets Linux")
            with self.assertRaisesRegex(facts.Invalid, "match Cargo"):
                facts.check_specs(root, self.claims)


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "tests").mkdir()
        self.path = self.root / "tests/examples.rs"
        self.base = "".join(f"// site:begin {name}\n    code();\n// site:end\n"
                            for name in ("basic", "batch", "range", "snapshot", "shared"))
        self.path.write_text(self.base)

    def test_extracts_verbatim_and_dedents(self):
        self.path.write_text(self.base + '// site:begin extra\n    a(); // keep\n\n        b();\n// site:end\n')
        self.assertEqual(facts.extract_examples(self.root)["extra"], 'a(); // keep\n\n    b();\n')

    def test_broken_regions_fail(self):
        for suffix, message in (
            ("// site:begin extra\n", "unclosed"),
            ("// site:end\n", "without begin"),
            ("// site:begin basic\n", "duplicate"),
            ("// site:begin a\n// site:begin b\n", "nested"),
            ("// site:begin bad id\n", "malformed"),
            ("// site:begin extra\n// site:end\n", "empty region"),
        ):
            with self.subTest(suffix=suffix):
                self.path.write_text(self.base + suffix)
                with self.assertRaisesRegex(facts.Invalid, message):
                    facts.extract_examples(self.root)

    def test_missing_tab_fails(self):
        self.path.write_text("")
        with self.assertRaisesRegex(facts.Invalid, "missing required site tabs"):
            facts.extract_examples(self.root)

    def test_binding_python_markers(self):
        path = self.root / "bindings/python/tests/test_binding.py"
        path.parent.mkdir(parents=True)
        path.write_text("# site:begin python-basic\n    db.put(b'a', b'b')\n# site:end\n")
        self.assertEqual(facts.extract_examples(self.root)["python-basic"], "db.put(b'a', b'b')\n")

    def test_counts_real_tests_and_masks_comments_and_strings(self):
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text('''
// #[test] fn fake_comment() {}
/* /* nested */ #[test] fn fake_block() {} */
const S: &str = r###"#[test] fn fake_raw() {}"###;
const T: &str = "#[test] fn fake_string() {}";
#[test]
#[ignore = "slow"]
fn real_test() {}
''')
        self.assertEqual(facts.lib_tests(self.root), 1)
        code = facts.rust_sources(self.root)[0][1]
        self.assertNotIn("fake_", code)
        self.assertIn("real_test", code)

    def test_test_list_counts_tests_excluding_benchmarks(self):
        path = self.root / "list.txt"
        path.write_text("module::one: test\nmodule::two: test\nbench: benchmark\n\n2 tests, 1 benchmark\n")
        self.assertEqual(facts.lib_tests(self.root, path), 2)
        for text in ("", "one: test\n", "one: test\n\n2 tests, 0 benchmarks\n", "one: test\none: test\n\n2 tests, 0 benchmarks\n"):
            path.write_text(text)
            with self.assertRaises(facts.Invalid):
                facts.lib_tests(self.root, path)

    def test_corpus_and_crash_counts_follow_source(self):
        source = (facts.ROOT / "src/simulation_tests.rs").read_text()
        self.assertEqual(facts.simulation_metrics(source.replace("for seed in 0..64", "for seed in 2..100").replace("for round in 0..3", "for round in 0..5")), (196, 6))
        with self.assertRaisesRegex(facts.Invalid, "cannot parse fixed corpus"):
            facts.simulation_metrics(source.replace("for seed in 0..64", "for seed in seeds()"))
        with self.assertRaisesRegex(facts.Invalid, "crash loop changed"):
            facts.simulation_metrics(source.replace("for round in 0..3", "for round in rounds()"))


class FeedTests(unittest.TestCase):
    def test_totals_carry_forward_and_deduplicate_runs(self):
        self.assertEqual(facts.nightly_totals(None, 0, None), (0, []))
        previous = {"schema": 1, "tests": {"nightly_seeds_total": 800000, "nightly_run_ids": ["42"]}}
        self.assertEqual(facts.nightly_totals(previous, 800000, "43"), (1600000, ["42", "43"]))
        self.assertEqual(facts.nightly_totals(previous, 800000, "42"), (800000, ["42"]))
        self.assertEqual(previous["tests"]["nightly_run_ids"], ["42"])
        for previous, extra, run in (({}, 0, None), (None, -1, "42"), (None, 1, None)):
            with self.assertRaises(facts.Invalid):
                facts.nightly_totals(previous, extra, run)

    def test_generation_uses_engine_git_and_historical_benchmarks(self):
        with patch.object(facts, "git", side_effect=["a" * 40, "2026-10-16T12:00:00+00:00"]):
            feed = facts.generate()
        self.assertEqual(feed["schema"], 1)
        self.assertEqual(feed["engine"]["commit"], "a" * 40)
        self.assertEqual(feed["benchmarks"]["commit"], "76d9d857c61bed3127fa3f7a2136701cd9b49380")
        self.assertTrue(feed["benchmarks"]["rows"])
        self.assertEqual(feed["tests"]["nightly_seeds_total"], 0)
        self.assertNotIn("site:begin", feed["examples"]["batch"])
        json.dumps(feed, allow_nan=False)

    def test_noop_generation_preserves_timestamp(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "facts.json"
            old = {"generated_at": "old", "schema": 1, "specs": []}
            new = {**old, "generated_at": "new"}
            facts.write_facts(path, new, old)
            self.assertEqual(json.loads(path.read_text()), old)
            new["specs"] = ["changed"]
            facts.write_facts(path, new, old)
            self.assertEqual(json.loads(path.read_text()), new)

    def test_missing_benchmarks_fall_back_explicitly(self):
        with tempfile.TemporaryDirectory() as directory, redirect_stderr(StringIO()) as output:
            self.assertEqual(facts.benchmarks(Path(directory)), {"source": "validation/COMPARISON.md"})
            self.assertIn("source only", output.getvalue())

    def test_invalid_cli_returns_clear_nonzero(self):
        with patch.object(facts, "validate", side_effect=facts.Invalid("missing evidence fn real_test")), redirect_stderr(StringIO()) as output:
            self.assertEqual(facts.main(["validate"]), 1)
            self.assertIn("missing evidence fn real_test", output.getvalue())


class NightlyReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.receipts = []
        for shard in range(4):
            receipt = {"run_id": "42", "commit": "a" * 40, "shard": shard,
                       "shard_count": 4, "seed_start": shard * 100000,
                       "seeds_per_suite": 100000,
                       "suites": ["seeded_crash_long", "shared_seeded_crash_long"]}
            path = self.root / str(shard) / "receipt.json"
            path.parent.mkdir()
            path.write_text(json.dumps(receipt))
            self.receipts.append((path, receipt))

    def test_receipt_requires_actual_completed_sweeps(self):
        environment = {"KIBAN_SIM_SEEDS": "100000", "KIBAN_SIM_SEED_START": "400000",
                       "GITHUB_RUN_ID": "42", "GITHUB_SHA": "a" * 40,
                       "SHARD": "0", "SHARD_COUNT": "4"}
        line = "checked 100000 seeds (400000..500000), 400000 crashes in 1s\n"
        result = "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured;\n"
        receipt = facts.nightly_receipt(line * 2 + result, environment)
        self.assertEqual(receipt["seeds_per_suite"], 100000)
        for log in (result, line + result, line * 3 + result,
                    (line * 2 + result).replace("500000", "500001"),
                    (line * 2 + result).replace("2 passed", "1 passed")):
            with self.subTest(log=log), self.assertRaises(facts.Invalid):
                facts.nightly_receipt(log, environment)

    def test_credits_both_suites_in_every_shard(self):
        self.assertEqual(facts.nightly_seeds(self.root, "42", "a" * 40), 800000)

    def test_rejects_incomplete_shards(self):
        self.receipts[0][0].unlink()
        with self.assertRaisesRegex(facts.Invalid, "incomplete shard set"):
            facts.nightly_seeds(self.root, "42", "a" * 40)

    def test_rejects_duplicate_shards(self):
        path, receipt = self.receipts[0]
        path.with_name("unused.json").write_text("{}")
        receipt["shard"] = 1
        path.write_text(json.dumps(receipt))
        with self.assertRaisesRegex(facts.Invalid, "duplicate"):
            facts.nightly_seeds(self.root, "42", "a" * 40)

    def test_rejects_wrong_revision_run_or_suite(self):
        path, receipt = self.receipts[0]
        for key, value in (("run_id", "41"), ("commit", "b" * 40),
                           ("suites", ["seeded_crash_long"]), ("seeds_per_suite", 0)):
            with self.subTest(key=key):
                invalid = {**receipt, key: value}
                path.write_text(json.dumps(invalid))
                with self.assertRaises(facts.Invalid):
                    facts.nightly_seeds(self.root, "42", "a" * 40)


if __name__ == "__main__":
    unittest.main()
