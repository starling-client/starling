"""Which jobs may run on our own runners (docs/ci-runners.md).

- sunrise-light (playsunrise's and other small runners, which refuse a fork's pull request): main's own scripts only.
- sunrise-checks (isolated runners that take any pull request, forks included): no secret and no token that writes.
- Neither takes a job that holds a secret or whose token can push, publish or start workflows: playsunrise is a
  partner's server, and a checks runner keeps what a fork's job left behind.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.actions_expression import branch_values, embedded_expressions, parsed_expression, truthy
from tests.os_support import BASH, needs_runner_shell
from tools.ci import vendored_yaml

yaml = vendored_yaml.load()

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
LIGHT = ("sunrise-light", "playsunrise-light", "playsunrise-ci")
CHECKS = ("sunrise-checks",)
FORK_GUARD = parsed_expression("github.event.pull_request.head.repo.full_name == github.repository")
FORK_FLAG = parsed_expression("github.event.pull_request.head.repo.fork")
# dyna-ci.yml decides trust once, in its changes job (a fork's pull request, dispatched with `pr` or not).
TRUSTED = parsed_expression("needs.changes.outputs.trusted == 'true'")
TOKEN_WRITE_SCOPES = {"contents", "actions", "packages", "id-token", "deployments", "attestations"}
# actions: write can start workflows too; cancel-on-close needs it to cancel runs and runs no checkout at all.
CANCELS_RUNS = {"cancel-on-close.yml cancel"}


def workflow(path):
    """The parsed workflow, every scalar a string (BaseLoader keeps `on` a key, not True)."""
    return yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)


def template_expressions(node):
    """Embedded expressions in a scalar, sequence, or mapping of template values."""
    if isinstance(node, dict):
        node = node.values()
    elif isinstance(node, str):
        yield from embedded_expressions(node)
        return
    for value in node:
        yield from template_expressions(value)


def fork_flag_expressions(node):
    """Expressions in `if`, `runs-on` and `env`: where a job is routed or classified."""
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "if":
                yield parsed_expression(value)
            elif key in ("runs-on", "env"):
                yield from template_expressions(value)
            else:
                yield from fork_flag_expressions(value)
    elif isinstance(node, list):
        for value in node:
            yield from fork_flag_expressions(value)


def jobs(document):
    """(name, job) for each job of a parsed workflow."""
    return list(document["jobs"].items())


def triggers(document):
    on = document["on"]
    return set(on) if isinstance(on, (dict, list)) else {on}


def permissions(*owners):
    """Every scope a permissions mapping of these workflow or job mappings grants, with its level."""
    return {scope: level for owner in owners for scope, level in owner.get("permissions", {}).items()}


def checkouts(job):
    return [step for step in job.get("steps", []) if step.get("uses", "").startswith("actions/checkout")]


def labels_of(runs_on, facts=None):
    """Runner labels named in the results, excluding dynamic values and literals used only as inputs."""
    if isinstance(runs_on, dict):
        return labels_of(runs_on.get("labels", []), facts)
    if isinstance(runs_on, list):
        return set().union(*(labels_of(label, facts) for label in runs_on))
    if not runs_on.lstrip().startswith("${{"):
        return {runs_on} if runs_on else set()
    labels = set()
    for value in branch_values(parsed_expression(runs_on), facts):
        if isinstance(value, str):
            labels.add(value)
        elif isinstance(value, list):
            labels.update(label for label in value if isinstance(label, str))
    return labels


SWITCH = parsed_expression("vars.SELF_HOSTED_CI")


def switch_facts(route, setting):
    """The value of each comparison of SELF_HOSTED_CI with a literal in `route` when it is `setting`."""
    facts = {}
    for comparison in route.comparisons():
        left, right = comparison.operands
        if comparison.operator in ("==", "!=") and left == SWITCH and right.operator == "literal" \
                and isinstance(right.value, str):
            facts[comparison] = (right.value.lower() == setting) == (comparison.operator == "==")
    return facts


def routed():
    """(where, triggers, workflow, job, runs-on, pools) for every job that can run on our runners."""
    for path in sorted(WORKFLOWS.glob("*.y*ml")):
        document = workflow(path)
        on = triggers(document)
        for name, job in jobs(document):
            value = job.get("runs-on", "")
            reached = labels_of(value)
            pools = {pool for pool, labels in (("light", LIGHT), ("checks", CHECKS)) if reached & set(labels)}
            if pools:
                yield f"{path.name} {name}", on, document, job, value, pools


class OwnRunnerTests(unittest.TestCase):
    def test_workflows_do_not_classify_pull_requests_by_fork_flag(self):
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            with self.subTest(workflow=path.name):
                for expression in fork_flag_expressions(workflow(path)):
                    self.assertNotIn(FORK_FLAG, expression.walk())

    def test_the_fork_flag_is_found_in_an_if_a_runs_on_and_an_env_value_only(self):
        document = yaml.load("""
jobs:
  a:
    if: github.event.pull_request.head.repo.fork
    runs-on: ubuntu-latest
  b:
    runs-on: ${{ github.event.pull_request.head.repo['fork'] && 'p' || 'q' }}
  c:
    runs-on: ubuntu-latest
    steps:
      - env:
          FORK: prefix-${{ github.event.pull_request.head.repo.fork }}
          TEXT: ${{ 'head.repo.fork' }}
        run: echo head.repo.fork
""", Loader=yaml.BaseLoader)
        found = [FORK_FLAG in expression.walk() for expression in fork_flag_expressions(document)]
        self.assertEqual(found, [True, True, True, False])

    def test_a_runs_on_expression_is_read_for_every_label_it_picks(self):
        light = "${{ vars.SELF_HOSTED_CI != 'off' && 'sunrise-light' || 'ubuntu-latest' }}"
        self.assertEqual(labels_of(light), {"sunrise-light", "ubuntu-latest"})
        both = "${{ a == 'x' && fromJSON('[\"self-hosted\", \"sunrise-checks\"]') || 'ubuntu-24.04' }}"
        self.assertEqual(labels_of(both), {"self-hosted", "sunrise-checks", "ubuntu-24.04"})

    def test_runner_labels_come_from_results_not_conditions_or_function_arguments(self):
        expression = """${{ contains(github.event.pull_request.labels.*.name, 'sunrise-checks') &&
            fromJSON(matrix.labels || '["self-hosted", "sunrise-light"]') || 'GitHub''s ! && || runner' }}"""
        self.assertEqual(labels_of({"group": "example", "labels": ["linux", expression]}),
                         {"linux", "self-hosted", "sunrise-light", "GitHub's ! && || runner"})
        self.assertEqual(labels_of("${{ contains(labels.*.name, 'sunrise-light') && 'ubuntu-latest' }}"),
                         {"ubuntu-latest"})
        self.assertEqual(labels_of("${{ true || 'sunrise-light' }}"), set())
        self.assertEqual(labels_of("${{ null && 'sunrise-light' || -0xF && 'ubuntu-24.04' }}"), {"ubuntu-24.04"})

    def test_comparisons_include_nested_operands_but_not_text_in_literals(self):
        condition = parsed_expression("""${{ !contains(fromJSON(inputs.labels).include[0].*.name, 'a <= b') &&
            matrix.jobs[inputs.slot] >= -0xF && inputs.ratio < 2.5e-1 || inputs.flag != false }}""")
        comparisons = condition.comparisons()
        self.assertEqual({comparison.operator for comparison in comparisons}, {">=", "<", "!="})
        self.assertEqual({comparison.operands[1].value for comparison in comparisons}, {-15, 0.25, False})

    def test_every_workflow_job_uses_parseable_expressions(self):
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            for name, job in jobs(workflow(path)):
                with self.subTest(workflow=path.name, job=name):
                    for owner in [job, *job.get("steps", ())]:
                        if "if" in owner:
                            parsed_expression(owner["if"])
                    if "runs-on" in job:
                        list(template_expressions(job["runs-on"]))

    def test_every_job_on_our_runners_can_go_back_to_github(self):
        count = 0
        for where, _, _, _, value, pools in routed():
            count += 1
            # dyna-ci.yml (ours) routes through the changes job's self_hosted
            # output and dyna-medium, or SELF_HOSTED_CI=all with the fork guard
            # before the playsunrise-ci labels; the carried upstream files use
            # SELF_HOSTED_CI != 'off'. Both fall back to GitHub's runners.
            if "dyna-ci.yml" in where:
                self.assertIn("needs.changes.outputs.self_hosted == 'true'", value, where)
                self.assertIn("vars.SELF_HOSTED_CI == 'all'", value, where)
                self.assertIn(TRUSTED, value, where)
            else:
                # Starling's ci.yml routes to the self-hosted labels with SELF_HOSTED_CI == 'all' (fork guard
                # included); the carried upstream files use SELF_HOSTED_CI != 'off'. Both fall back to GitHub's.
                self.assertTrue("vars.SELF_HOSTED_CI != 'off'" in value or "vars.SELF_HOSTED_CI == 'all'" in value,
                                where)
            # Both labels select a GitHub-hosted Ubuntu runner when our pools are disabled.
            self.assertRegex(value, r"\|\| 'ubuntu-(?:latest|24\.04)' \}\}$", where)
            if "checks" in pools:
                # The desktop paused: every checks job back to GitHub, the bots stay.
                self.assertIn("vars.SELF_HOSTED_CI != 'light'", value, where)
        self.assertGreaterEqual(count, 5)

    def assert_switch_routes_back(self, where, value, pools):
        """SELF_HOSTED_CI=off reaches neither pool and =light no checks runner, on every branch of `runs-on`."""
        self.assertTrue(isinstance(value, str) and value.lstrip().startswith("${{"),
                        f"{where}: a fixed label cannot go back to GitHub")
        route = parsed_expression(value)
        self.assertFalse(labels_of(value, switch_facts(route, "off")) & {*LIGHT, *CHECKS},
                         f"{where}: our runners with SELF_HOSTED_CI=off")
        if "checks" in pools:
            # The desktop paused: every checks job back to GitHub, the bots stay.
            self.assertFalse(labels_of(value, switch_facts(route, "light")) & set(CHECKS),
                             f"{where}: a checks runner with SELF_HOSTED_CI=light")
        # The final fallback is a GitHub-hosted Ubuntu runner, not another conditional route.
        fallback = route
        while fallback.operator == "||":
            fallback = fallback.operands[1]
        self.assertTrue(all(value in ("ubuntu-latest", "ubuntu-24.04") for value in branch_values(fallback)), where)

    def test_a_route_around_the_switch_is_refused(self):
        guarded = [
            ("${{ vars.SELF_HOSTED_CI != 'off' && 'sunrise-light' || 'ubuntu-latest' }}", {"light"}),
            ("${{ vars.SELF_HOSTED_CI != 'off' && vars.SELF_HOSTED_CI != 'light' && "
             "fromJSON('[\"self-hosted\", \"sunrise-checks\"]') || 'ubuntu-latest' }}", {"checks"}),
            ("${{ needs.x.outputs.trusted == 'true' && vars.SELF_HOSTED_CI == 'all' && "
             "fromJSON('[\"self-hosted\", \"playsunrise-ci\"]') || 'ubuntu-24.04' }}", {"light"}),
        ]
        for value, pools in guarded:
            with self.subTest(route=value):
                self.assert_switch_routes_back("fixture", value, pools)
        around = [
            # The switch is compared, but another branch reaches the pool whatever it says.
            ("${{ vars.SELF_HOSTED_CI != 'off' && 'sunrise-light' || inputs.fast && 'sunrise-light' || "
             "'ubuntu-latest' }}", {"light"}, "SELF_HOSTED_CI=off"),
            ("${{ (vars.SELF_HOSTED_CI != 'off' || inputs.force) && 'sunrise-light' || 'ubuntu-latest' }}",
             {"light"}, "SELF_HOSTED_CI=off"),
            # A computed operand is not a literal: format(...) can be 'off' too.
            ("${{ vars.SELF_HOSTED_CI == format('{0}', inputs.mode) && 'sunrise-light' || 'ubuntu-latest' }}",
             {"light"}, "SELF_HOSTED_CI=off"),
            ("${{ vars.SELF_HOSTED_CI != 'off' && (vars.SELF_HOSTED_CI != 'light' || inputs.force) && "
             "fromJSON('[\"self-hosted\", \"sunrise-checks\"]') || 'ubuntu-latest' }}", {"checks"},
             "SELF_HOSTED_CI=light"),
        ]
        for value, pools, message in around:
            with self.subTest(route=value), self.assertRaisesRegex(AssertionError, message):
                self.assert_switch_routes_back("fixture", value, pools)

    def test_a_label_picked_from_a_constant_list_is_on_our_runners(self):
        fixture = "on: pull_request\njobs:\n  verify:\n    runs-on: >-\n      ${{{{ {route} }}}}\n"
        routes = [
            "fromJSON('[\"ubuntu-latest\", \"sunrise-light\"]')[1]",
            "fromJSON('{\"Pool\": [\"self-hosted\", \"sunrise-light\"]}').pool",
            "fromJSON('[\"ubuntu-latest\", \"sunrise-light\"]')[inputs.slot]",
            "fromJSON('[\"ubuntu-latest\", \"sunrise-light\"]')['1']",
            "fromJSON('[\"ubuntu-latest\", \"sunrise-light\"]')[1.5]",
        ]
        with tempfile.TemporaryDirectory() as directory, patch(f"{__name__}.WORKFLOWS", Path(directory)):
            path = Path(directory) / "fixture.yml"
            for route in routes:
                with self.subTest(route=route):
                    path.write_text(fixture.format(route=route))
                    self.assertEqual([pools for *_, pools in routed()], [{"light"}])
                    with self.assertRaisesRegex(AssertionError, "a fork's pull request"):
                        self.test_light_runners_take_main_code_only()
            path.write_text(fixture.format(route="fromJSON('[\"ubuntu-latest\", \"sunrise-light\"]')[0]"))
            self.assertEqual(list(routed()), [])

    def test_light_runners_take_main_code_only(self):
        for where, on, _, job, value, pools in routed():
            if "light" not in pools:
                continue
            # The guard lives in the job's `if` or on the branch of `runs-on` that picks our labels.
            # Merely comparing the repositories does not guard a label reached by another branch.
            if "pull_request" in on:
                fork = {FORK_GUARD: False, TRUSTED: False}
                condition = parsed_expression(job.get("if", "true"))
                if any(map(truthy, branch_values(condition, fork))):
                    self.assertFalse(labels_of(value, fork) & set(LIGHT), f"{where}: a fork's pull request")
            for checkout in checkouts(job):
                options = checkout.get("with", {})
                self.assertIn(options.get("repository"), (None, "${{ github.repository }}"),
                              f"{where}: checkout of another repository")
                ref = options.get("ref")
                if on & {"pull_request_target", "issue_comment"}:
                    # Bots acting on a PR must explicitly choose main, never the PR head or merge ref.
                    self.assertIn(ref, ("main", "refs/heads/main"), f"{where}: checkout does not pin main")
                elif where.startswith("dyna-ci.yml ") and ref == "${{ needs.changes.outputs.pr_merge }}":
                    # The gate can check out a PR chosen on dispatch. Its trust output guards the label independently
                    # of the event that started the job.
                    self.assertFalse(labels_of(value, {TRUSTED: False}) & set(LIGHT),
                                     f"{where}: checkout of an untrusted selected PR")
                else:
                    # A schedule uses main; a trusted push or dispatch may also use its own repository ref.
                    self.assertIn(ref, (None, "main", "refs/heads/main"), f"{where}: checkout of an untrusted ref")

    def test_light_runner_guard_must_block_the_fork_branch(self):
        fixture = (
            "on: pull_request\njobs:\n  verify:\n"
            "    runs-on: ${{ contains(github.event.pull_request.labels.*.name, 'sync') && "
            "(github.event.pull_request.head.repo.full_name == github.repository {guard}) && "
            "'sunrise-light' || 'ubuntu-latest' }}\n"
        )
        with tempfile.TemporaryDirectory() as directory, patch(f"{__name__}.WORKFLOWS", Path(directory)):
            path = Path(directory) / "fixture.yml"
            path.write_text(fixture.replace("{guard}", ""))
            self.test_light_runners_take_main_code_only()
            path.write_text(fixture.replace("{guard}", "|| inputs.bypass"))
            with self.assertRaisesRegex(AssertionError, "a fork's pull request"):
                self.test_light_runners_take_main_code_only()

    def test_a_light_comment_job_checks_out_main_not_the_pull_request(self):
        fixture = (
            "on: issue_comment\njobs:\n  comment:\n    runs-on: sunrise-light\n"
            "    steps:\n      - uses: actions/checkout@v7\n        with:\n          ref: "
        )
        with tempfile.TemporaryDirectory() as directory, patch(f"{__name__}.WORKFLOWS", Path(directory)):
            path = Path(directory) / "fixture.yaml"
            path.write_text(fixture + "main\n")
            self.test_light_runners_take_main_code_only()
            path.write_text(fixture + "refs/pull/1/head\n")
            with self.assertRaisesRegex(AssertionError, "checkout"):
                self.test_light_runners_take_main_code_only()

    def test_no_secret_or_writing_token_on_our_runners(self):
        for where, _, document, job, _, pools in routed():
            self.assertNotIn("secrets.", json.dumps(job), where)
            granted = permissions(document, job)
            writes = {scope for scope, level in granted.items() if level == "write" and scope in TOKEN_WRITE_SCOPES}
            if where in CANCELS_RUNS:
                writes -= {"actions"}
                self.assertEqual(checkouts(job), [], where)
            self.assertEqual(writes, set(), where)
            if "checks" in pools:
                # A checks runner keeps what a fork's job left: its jobs' tokens write nothing at all.
                self.assertNotIn("write", granted.values(), where)

    def test_no_deployment_environment_on_our_runners(self):
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            document = yaml.safe_load(path.read_text(encoding="utf-8"))
            for name, job in jobs(document):
                if labels_of(job.get("runs-on", "")) & {*LIGHT, *CHECKS}:
                    self.assertNotIn("environment", job, f"{path.name} {name}")

    def test_deployment_environment_guard_distinguishes_script_variables(self):
        safe = ("jobs:\n  verify:\n    runs-on: sunrise-checks\n    steps:\n"
                "      - run: |\n          with open('output') as environment:\n              pass\n")
        forbidden = [
            safe.replace("    steps:", f"    {key} deployment\n    steps:")
            for key in ("environment:", "\"environment\":", "'environment' :")
        ] + [
            "jobs:\n    verify:\n        runs-on: sunrise-checks\n        environment: deployment\n",
            "jobs:\n  verify: {runs-on: sunrise-checks, environment: deployment}\n",
            "env: &defaults {environment: deployment}\njobs:\n  verify:\n"
            "    <<: *defaults\n    runs-on: sunrise-checks\n",
        ]
        with tempfile.TemporaryDirectory() as directory, patch(f"{__name__}.WORKFLOWS", Path(directory)):
            workflow = Path(directory) / "fixture.yml"
            workflow.write_text(safe)
            self.test_no_deployment_environment_on_our_runners()
            for text in forbidden:
                with self.subTest(workflow=text):
                    workflow.write_text(text)
                    with self.assertRaises(AssertionError):
                        self.test_no_deployment_environment_on_our_runners()

    def test_the_light_runners_refuse_forks_themselves(self):
        hook = ROOT / "tools/ci/job-started.sh"
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)

            def job(event, head=None):
                path = home / "event.json"
                path.write_text(json.dumps({"pull_request": {"head": {"repo": {"full_name": head}}}}))
                return subprocess.run(["bash", str(hook)], capture_output=True, text=True, env={
                    "PATH": os.environ["PATH"], "GITHUB_EVENT_NAME": event, "GITHUB_EVENT_PATH": str(path),
                    "GITHUB_REPOSITORY": "sunrise-org/sunrise"})

            fork = job("pull_request", "someone/sunrise")
            self.assertEqual(fork.returncode, 1, fork.stderr)
            self.assertIn("someone/sunrise", fork.stdout)
            self.assertEqual(job("pull_request", "sunrise-org/sunrise").returncode, 0)
            # Events that carry no fork's code are not refused.
            self.assertEqual(job("push", "someone/sunrise").returncode, 0)

    @needs_runner_shell
    @unittest.skipUnless(shutil.which("jq"), "the hook reads the event with jq")
    def test_a_failed_size_check_of_the_checkout_does_not_fail_the_job(self):
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            (home / "workspace/.git").mkdir(parents=True)
            (home / "bin").mkdir()
            du = home / "bin/du"
            du.write_text("#!/bin/sh\nexit 1\n")
            du.chmod(0o755)
            job = subprocess.run(["bash", str(ROOT / "tools/ci/job-started.sh")], capture_output=True, text=True, env={
                "PATH": f"{home / 'bin'}{os.pathsep}{os.environ['PATH']}", "GITHUB_EVENT_NAME": "push",
                "GITHUB_WORKSPACE": str(home / "workspace")})
            self.assertEqual(job.returncode, 0, job.stderr)
            self.assertTrue((home / "workspace/.git").is_dir())

    @needs_runner_shell
    def test_a_failed_size_check_of_the_cargo_target_does_not_fail_the_job(self):
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            target = home / "target"
            target.mkdir()
            du = home / "du"
            # du can print a partial total and still fail. Do not act on that total.
            du.write_bytes(b"#!/bin/sh\nprintf '41\\t%s\\n' \"$3\"\nexit 1\n")
            du.chmod(0o755)
            job = subprocess.run([BASH, str(ROOT / "tools/ci/job-started.sh")], capture_output=True, text=True, env={
                **os.environ, "PATH": f"{home}{os.pathsep}{os.environ['PATH']}", "GITHUB_EVENT_NAME": "push",
                "CARGO_TARGET_DIR": str(target), "MAX_TARGET_GB": "40", "GITHUB_WORKSPACE": "", "RUNNER_WORKSPACE": ""})
            self.assertEqual(job.returncode, 0, job.stderr)
            self.assertTrue(target.is_dir())


if __name__ == "__main__":
    unittest.main()
