"""A contributor's pull request gets only the cheap checks; rollups, syncs, main and dispatches get the heavy ones."""

import importlib.util
import itertools
import json
import os
import re
import subprocess
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("pr_gate", ROOT / "tools/ci/pr_gate.py")
pr_gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pr_gate)

SHA = "a" * 40
MERGE = "b" * 40
SAME_REPO = "github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"


def never_read(_):
    raise AssertionError("only a dispatch with pr reads a pull request")


class GateTests(unittest.TestCase):
    def test_pull_request_gate(self):
        cases = [
            (True, ["rollup"], "work", "true"),
            (True, [], "fork-review-rollup-42", "true"),
            (True, [], "sunrise-review-rollup-1", "true"),
            (True, ["sync"], "work", "true"),
            (True, [], "sync-28-35", "true"),
            (True, [], "rollup-76", "true"),
            (True, [], "rollup-77", "true"),
            (True, [], "rollupx", "false"),
            (True, [], "cfix-hooks-rollup-76", "false"),
            (False, [], "rollup-76", "false"),
            (True, ["bug"], "ci-gate", "false"),
            (False, ["rollup", "sync"], "fork-review-rollup-42", "false"),
            (False, [], "sunrise-review-rollup-1", "false"),
        ]
        for same, labels, branch, expected in cases:
            with self.subTest(same=same, labels=labels, branch=branch):
                result = pr_gate.outputs("pull_request", same, "", never_read,
                                         labels=labels, head_ref=branch)
                self.assertEqual(result, {"heavy": expected, "pr_sha": "", "pr_merge": "", "trusted": ""})
                # An empty trusted output preserves the workflow's repository-based trust fallback.
        for event in ("push", "schedule", "workflow_dispatch"):
            self.assertEqual(pr_gate.outputs(event, False, "", never_read)["heavy"], "true")

    def test_a_dispatch_with_pr_runs_its_test_merge_reports_its_head_and_keeps_a_fork_off_our_runners(self):
        fork = pr_gate.outputs("workflow_dispatch", True, "3665", lambda n: {
            "sha": SHA, "merge": MERGE, "mergeable": True, "ref": "sync-upstream", "labels": ["rollup"],
            "same_repository": False})
        self.assertEqual(fork, {"heavy": "true", "pr_sha": SHA, "pr_merge": MERGE, "trusted": "false"})
        ours = pr_gate.outputs("workflow_dispatch", True, "3672", lambda n: {
            "sha": SHA, "merge": MERGE, "mergeable": True, "ref": "split/c3-bridge-docs", "labels": [],
            "same_repository": True})
        self.assertEqual(ours, {"heavy": "true", "pr_sha": SHA, "pr_merge": MERGE, "trusted": "true"})

    def test_a_dispatch_without_a_test_merge_is_refused(self):
        for mergeable in (False, None):
            head = {"sha": SHA, "merge": "", "mergeable": mergeable, "ref": "work", "labels": [],
                    "same_repository": True}
            with self.subTest(mergeable=mergeable), self.assertRaises(SystemExit):
                pr_gate.outputs("workflow_dispatch", True, "3241", lambda n, head=head: head)

    def test_a_dispatch_on_our_rollup_or_sync_is_refused_since_its_pushes_run_the_gate(self):
        for ref, labels in (("sunrise-review-rollup-43", []), ("sync-upstream-7", []), ("sync/upstream-acbd", []), ("integration", ["rollup"])):
            head = {"sha": SHA, "merge": MERGE, "mergeable": True, "ref": ref, "labels": labels,
                    "same_repository": True}
            with self.subTest(ref=ref, labels=labels), self.assertRaisesRegex(SystemExit, "rollup or sync"):
                pr_gate.outputs("workflow_dispatch", True, "2833", lambda n, head=head: head)

    def test_pr_must_be_a_number(self):
        for bad in ("0", "12a", "1;id", "refs/pull/1/head", "-1"):
            with self.subTest(pr=bad), self.assertRaises(SystemExit):
                pr_gate.outputs("workflow_dispatch", True, bad, never_read)


class WorkflowTests(unittest.TestCase):
    workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()

    def job(self, name):
        match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z0-9_-]+:\n|\Z)", self.workflow, re.M | re.S)
        self.assertIsNotNone(match, name)
        return match[1]

    def test_heavy_jobs_wait_for_the_gate_and_fail_closed(self):
        for name in ("lint", "verify"):
            with self.subTest(job=name):
                self.assertIn("(needs.changes.result != 'success' || needs.changes.outputs.heavy == 'true')",
                              self.job(name))

    def test_cheap_jobs_run_without_the_heavy_gate(self):
        # verify-static runs outside the gate too: only where it runs depends on the gate.
        self.assertNotIn("outputs.heavy", self.job("verify-static").split("\n", 2)[1])
        self.assertIn("needs.changes.outputs.heavy == 'false'", self.job("format"))
        self.assertNotRegex(self.job("format"), r"cargo (clippy|build|test|check)")

    def test_a_dispatched_test_merge_is_checked_out_with_a_read_only_token_and_reported_on_the_head(self):
        for name in ("lint", "verify", "verify-static"):
            with self.subTest(job=name):
                self.assertIn("ref: ${{ needs.changes.outputs.pr_merge }}", self.job(name))
                self.assertNotIn("outputs.pr_sha", self.job(name))
                self.assertNotIn("statuses: write", self.job(name))
        status = self.job("pr-status")
        self.assertIn("statuses: write", status)
        self.assertIn("SHA: ${{ needs.changes.outputs.pr_sha }}", status)
        self.assertNotIn("actions/checkout", status)
        self.assertIn("permissions:\n  contents: read\n", self.workflow)

    def test_no_fork_code_reaches_our_runners(self):
        # The one trust decision: the gate's answer for a dispatched pull request, else the event's own repository.
        self.assertIn(f"trusted: ${{{{ steps.gate.outputs.trusted || (({SAME_REPO}) && 'true' || 'false') }}}}",
                      self.job("changes"))
        self.assertNotIn("github.event.pull_request.head.repo.full_name == github.repository) &&", self.job("lint"))
        for name in ("lint", "verify", "verify-static", "boat-watch"):
            body = self.job(name)
            if "head.repo.full_name" in body:
                self.fail(f"{name} decides trust itself instead of needs.changes.outputs.trusted")

    def runs_on(self, name, outputs, self_hosted_ci):
        """Evaluates a job's runs-on expression with these changes outputs, the way Actions would."""
        expression = re.search(r"^    runs-on: \$\{\{ (.*) \}\}$", self.job(name), re.M)[1]
        names = {"needs.changes.outputs." + key: value for key, value in outputs.items()}
        names |= {"vars.SELF_HOSTED_CI": self_hosted_ci, "inputs.github-fallback": False}
        python = re.sub(r"fromJSON\('([^']*)'\)", lambda m: repr(json.loads(m[1])), expression)
        python = re.sub(r"[a-z]+(?:\.[A-Za-z_-]+)+", lambda m: repr(names[m[0]]), python)
        python = python.replace("&&", " and ").replace("||", " or ").replace("!=", " != ")
        python = re.sub(r"!(?!=)", " not ", python)
        return eval(python, {})

    def test_an_untrusted_head_never_gets_our_runners(self):
        # A fork's dispatched pr: trusted is false and the runner script answers self_hosted=false.
        for name in ("lint", "verify", "verify-static"):
            for rust, forced, label, pool in itertools.product(("true", "false"), ("true", "false"),
                                                               ("", "boat"), ("all", "")):
                outputs = {"rust_required": rust, "forced_github": forced, "hosted_label": label,
                           "self_hosted": "false", "trusted": "false", "heavy": "true"}
                with self.subTest(job=name, outputs=outputs, pool=pool):
                    runner = self.runs_on(name, outputs, pool)
                    # verify-static, one of a contributor's two checks, never waits on a paid pool either.
                    self.assertIn(runner, ("ubuntu-latest",) if name == "verify-static" else ("ubuntu-latest", "boat"))
        # The control: the same expressions do reach our runners for a trusted head.
        trusted = {"rust_required": "true", "forced_github": "false", "hosted_label": "",
                   "self_hosted": "true", "trusted": "true", "heavy": "true"}
        self.assertEqual(self.runs_on("verify", trusted, "all"), "dyna-large")
        self.assertEqual(self.runs_on("verify-static", trusted, "all"), "dyna-medium")

    def test_a_pull_request_outside_the_gate_gets_githubs_runner_for_verify_static(self):
        # Our own branch, Rust changed, every switch pointing at our runners or boat: still GitHub's runner.
        for rust, label, pool in itertools.product(("true", "false"), ("", "boat-large"), ("all", "")):
            outputs = {"rust_required": rust, "forced_github": "false", "hosted_label": label,
                       "self_hosted": "false", "trusted": "true", "heavy": "false"}
            with self.subTest(rust=rust, label=label, pool=pool):
                self.assertEqual(self.runs_on("verify-static", outputs, pool), "ubuntu-latest")

    def test_the_runner_script_hears_the_gates_trust(self):
        self.assertIn("SAME_REPO: ${{ steps.gate.outputs.trusted || (", self.job("changes"))
        lease = str(int(time.time()) + 3600)
        for same_repo, expected in (("false", "self_hosted=false"), ("true", "self_hosted=true")):
            env = {"PATH": os.environ["PATH"], "ENABLED": "true", "SAME_REPO": same_repo,
                   "EVENT": "workflow_dispatch", "LEASE": lease}
            result = subprocess.run(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"], env=env, capture_output=True,
                                    text=True, check=True)
            self.assertEqual(result.stdout.strip(), expected, same_repo)

    def test_a_pull_requests_cheap_checks_run_on_githubs_runners(self):
        # format and verify-static are all a pull request outside the gate gets: nothing gates on them, so they leave
        # Dyna's counted runners to the gates, and a paid pool's balance must not fail them.
        self.assertIn("runs-on: ubuntu-latest\n", self.job("format"))
        self.assertIn("self_hosted: ${{ steps.gate.outputs.heavy == 'true' && ", self.job("changes"))
        # Outside the gate, the first term decides (Actions' || and && short-circuit): no switch, paid pool or boat
        # label reached through the later terms can move verify-static, and boat's watcher never starts.
        static = self.job("verify-static")
        self.assertIn("runs-on: ${{ needs.changes.outputs.heavy == 'false' && 'ubuntu-latest' || ", static)
        self.assertIn("RUNNER_KIND: ${{ needs.changes.outputs.heavy == 'false' && 'github' || ", static)
        self.assertIn("if: needs.changes.outputs.heavy == 'true' && ", self.job("boat-watch"))
        # A contributor's dispatched pull request is heavy but untrusted: GitHub's runner too.
        self.assertIn("needs.changes.outputs.trusted != 'true' && 'ubuntu-latest' ||", static)
        self.assertIn("|| needs.changes.outputs.trusted != 'true' && 'github' ||", static)

    def test_windows_jobs_run_daily_and_by_dispatch_only(self):
        import yaml

        workflow = yaml.safe_load((ROOT / ".github/workflows/windows-runtime-leases.yml").read_text())
        # GitHub's Windows runners: no push, pull request, rollup or sync waits on them; the daily run calls it.
        self.assertEqual(set(workflow[True]), {"workflow_call", "workflow_dispatch"})
        jobs = workflow["jobs"]
        for name in ("runtime-leases",):
            self.assertEqual(jobs[name]["if"], "inputs.commit != '' || github.event_name == 'workflow_dispatch'")

    def test_other_pull_request_workflows_skip_what_compiles_for_forks(self):
        for workflow, jobs in (("cargo-sunrise-mod-build.yml", ("build",)), ("modding-book.yml", ("book",))):
            text = (ROOT / ".github/workflows" / workflow).read_text()
            for name in jobs:
                with self.subTest(workflow=workflow, job=name):
                    body = re.search(rf"^  {name}:\n(.*?)(?=^  [a-z0-9_-]+:\n|\Z)", text, re.M | re.S)[1]
                    self.assertIn(f"if: {SAME_REPO}\n", body)

    def test_a_pull_request_builds_the_mod_tool_for_linux_only(self):
        import yaml

        workflow = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod-build.yml").read_text())
        # Every other archive is the daily run's (and a tag's): a pull request push holds no macOS or Windows runner.
        self.assertEqual(set(workflow[True]), {"workflow_call"})
        step = next(step for step in workflow["jobs"]["plan"]["steps"] if step.get("id") == "platforms")
        def targets(event):
            import tempfile
            with tempfile.NamedTemporaryFile("r") as output:
                subprocess.run(["bash", "-e", "-c", step["run"]], cwd=ROOT, check=True,
                               env={**os.environ, "EVENT_NAME": event, "GITHUB_OUTPUT": output.name})
                return [platform["target"] for platform in json.loads(output.read().split("=", 1)[1])]
        self.assertEqual(targets("pull_request"), ["x86_64-unknown-linux-gnu"])
        from tools import sunrise_mod_binstall
        self.assertEqual(targets("workflow_dispatch"), [platform["target"] for platform in sunrise_mod_binstall.PLATFORMS])


    def test_only_a_version_tag_push_publishes_the_mod_tool_archives(self):
        import yaml

        build = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod-build.yml").read_text())
        entry = yaml.safe_load((ROOT / ".github/workflows/cargo-sunrise-mod.yml").read_text())
        daily = yaml.safe_load((ROOT / ".github/workflows/daily.yml").read_text())
        # The daily calls the build alone, read-only: nothing it runs can publish.
        self.assertEqual(daily["jobs"]["cargo-sunrise-mod"]["uses"], "./.github/workflows/cargo-sunrise-mod-build.yml")
        self.assertEqual(daily["jobs"]["cargo-sunrise-mod"]["permissions"], {"contents": "read"})
        self.assertEqual(build["permissions"], {"contents": "read"})
        self.assertTrue(all("permissions" not in job for job in build["jobs"].values()))
        # The entry point publishes the archives it built itself, and only for the push of a version tag: a dispatch
        # may name a tag too.
        self.assertEqual(entry["jobs"]["build"]["uses"], "./.github/workflows/cargo-sunrise-mod-build.yml")
        self.assertNotIn("with", entry["jobs"]["build"])
        self.assertEqual(entry["jobs"]["publish"]["needs"], "build")
        self.assertEqual(entry["jobs"]["publish"]["if"],
                         "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/cargo-sunrise-mod-v')")
        self.assertEqual(set(entry["jobs"]), {"build", "publish"})


if __name__ == "__main__":
    unittest.main()
