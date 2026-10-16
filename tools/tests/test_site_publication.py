"""Run the workflow's actual publication shell against a disposable local remote."""

import json
import os
import shlex
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run(*args, cwd=None, env=None, check=True, input=None):
    return subprocess.run(args, cwd=cwd, env=env, check=check, input=input,
                          text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="kiban-site-publish-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.remote = self.root / "remote.git"
        run("git", "init", "--bare", "--initial-branch=main", str(self.remote))
        source = self.root / "source"
        self.source = source
        source.mkdir()
        for directory in ("src", "tests", "examples", "bindings", "site"):
            shutil.copytree(ROOT / directory, source / directory)
        for filename in ("Cargo.toml", "README.md"):
            shutil.copyfile(ROOT / filename, source / filename)
        (source / "tools").mkdir()
        shutil.copyfile(ROOT / "tools/site_facts.py", source / "tools/site_facts.py")
        run("git", "init", "--initial-branch=main", str(source))
        run("git", "config", "user.name", "Publication test", cwd=source)
        run("git", "config", "user.email", "test@example.invalid", cwd=source)
        run("git", "add", ".", cwd=source)
        run("git", "commit", "-m", "fixture", cwd=source)
        run("git", "remote", "add", "origin", str(self.remote), cwd=source)
        run("git", "push", "origin", "main", cwd=source)
        self.main = self.git("rev-parse", "main")
        workflow = (ROOT / ".github/workflows/site-facts.yml").read_text()
        block = workflow.split("      - name: Generate and publish facts\n", 1)[1]
        block = block.split("        run: |\n", 1)[1].split("      - name:", 1)[0]
        self.script = self.root / "publish.sh"
        self.script.write_text(textwrap.dedent(block))
        self.worker = self.clone("worker")

    def git(self, *args):
        return run("git", "--git-dir", str(self.remote), *args).stdout.strip()

    def clone(self, name):
        path = self.root / name
        run("git", "clone", str(self.remote), str(path))
        return path

    def environment(self, worker, seeds=0, run_id=""):
        scratch = self.root / (worker.name + "-scratch")
        scratch.mkdir(exist_ok=True)
        return {**os.environ, "RUNNER_TEMP": str(scratch),
                "GITHUB_OUTPUT": str(scratch / "outputs"),
                "NIGHTLY_SEEDS": str(seeds), "NIGHTLY_RUN": run_id,
                "PYTHONDONTWRITEBYTECODE": "1"}

    def publish(self, seeds=0, run_id="", check=True):
        return run("bash", "-e", str(self.script), cwd=self.worker,
                   env=self.environment(self.worker, seeds, run_id), check=check)

    def feed(self):
        return json.loads(self.git("show", "site-data:facts.json"))

    def test_orphan_noop_nightly_retry_and_competing_publishers(self):
        self.publish()
        first = self.git("rev-parse", "site-data")
        self.assertEqual(self.git("rev-list", "--parents", "-n", "1", "site-data"), first)
        self.assertEqual(self.git("ls-tree", "--name-only", "site-data"), "facts.json")
        self.assertEqual(self.feed()["engine"]["commit"], self.main)
        self.publish()
        self.assertEqual(self.git("rev-parse", "site-data"), first)
        self.publish(800000, "42")
        second = self.git("rev-parse", "site-data")
        self.assertNotEqual(first, second)
        self.publish(800000, "42")
        self.assertEqual(self.git("rev-parse", "site-data"), second)
        self.assertEqual(self.feed()["tests"]["nightly_seeds_total"], 800000)

        # Before A's push, B wins with another credit. A must refetch and retry.
        competitor = self.clone("competitor")
        environment = self.environment(competitor, 800000, "43")
        hook = self.worker / ".git/hooks/pre-push"
        assignments = " ".join(shlex.quote(f"{key}={environment[key]}") for key in
                               ("RUNNER_TEMP", "GITHUB_OUTPUT", "NIGHTLY_SEEDS", "NIGHTLY_RUN"))
        hook.write_text("#!/bin/bash\nset -euo pipefail\nmv \"$0\" \"$0.used\"\n"
                        f"cd {shlex.quote(str(competitor))}\n"
                        # Git exports the caller's GIT_DIR to hooks; clear it.
                        f"env -u GIT_DIR -u GIT_WORK_TREE {assignments} bash -e {shlex.quote(str(self.script))}\n")
        hook.chmod(0o755)
        result = self.publish(800000, "44")
        self.assertIn("Publication raced or push failed", result.stdout)
        self.assertEqual(self.feed()["tests"]["nightly_seeds_total"], 2400000)
        self.assertEqual(self.feed()["tests"]["nightly_run_ids"], ["42", "43", "44"])
        self.assertEqual(self.git("rev-parse", "main"), self.main)
        self.assertEqual(self.git("rev-list", "--count", "site-data"), "4")

    def test_stale_checkout_publishes_current_main(self):
        self.publish(800000, "42")
        (self.source / "README.md").write_text((self.source / "README.md").read_text() + "\nUpdated main.\n")
        run("git", "add", "README.md", cwd=self.source)
        run("git", "commit", "-m", "new main", cwd=self.source)
        run("git", "push", "origin", "main", cwd=self.source)
        latest = self.git("rev-parse", "main")
        self.assertNotEqual(latest, self.main)
        self.publish()
        self.assertEqual(self.feed()["engine"]["commit"], latest)
        self.assertEqual(self.feed()["tests"]["nightly_seeds_total"], 800000)

    def test_bad_previous_feed_fails_without_resetting_history(self):
        self.publish()
        parent = self.git("rev-parse", "site-data")
        blob = run("git", "hash-object", "-w", "--stdin", cwd=self.worker,
                   input='{"schema": 99}').stdout.strip()
        tree = run("git", "mktree", cwd=self.worker,
                   input=f"100644 blob {blob}\tfacts.json\n").stdout.strip()
        commit = run("git", "commit-tree", tree, "-p", parent, "-m", "bad fixture",
                     cwd=self.worker).stdout.strip()
        run("git", "push", "origin", f"{commit}:refs/heads/site-data", cwd=self.worker)
        result = self.publish(check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("previous facts: expected schema 1", result.stderr)
        self.assertEqual(self.git("rev-parse", "site-data"), commit)


if __name__ == "__main__":
    unittest.main()
