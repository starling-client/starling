"""The release workflows' job graphs: which jobs run, as GitHub decides it, on the ordinary paths.

A job whose `if:` names no status function gets GitHub's implicit `success()`, which is false when any job upstream
of it, however far, was skipped. The break-glass jobs are skipped on every ordinary release, so a job after them
without an explicit status check never ran (no Nightly or Dev built after 81def48ebc). These tests evaluate each
job's condition the way GitHub does, from the workflow files, and walk the graph.
"""

import json
import re
import unittest
from pathlib import Path
from tools.ci import vendored_yaml

ROOT = Path(__file__).resolve().parents[1]
yaml = vendored_yaml.load()

WORKFLOWS = ROOT / ".github/workflows"
STATUS = re.compile(r"\b(always|cancelled|failure)\(\)")
TOKEN = re.compile(r"\s*(?:(?P<str>'(?:[^']|'')*')|(?P<op>&&|\|\||==|!=|[!(),])|(?P<num>\d+)|(?P<name>[A-Za-z_][\w.\-]*))")


def load(name):
    return yaml.safe_load((WORKFLOWS / name).read_text())


def needs_of(job):
    needs = job.get("needs") or []
    return [needs] if isinstance(needs, str) else needs


def condition(job):
    text = str(job.get("if", "")).strip()
    if text.startswith("${{") and text.endswith("}}"):
        text = text[3:-2].strip()
    return text


def upstream(jobs, name):
    seen = set()
    stack = list(needs_of(jobs[name]))
    while stack:
        job = stack.pop()
        if job not in seen:
            seen.add(job)
            stack.extend(needs_of(jobs[job]))
    return seen


class Expression:
    """The subset of GitHub's expression language these workflows' `if:`s use."""

    def __init__(self, text, lookup, functions):
        self.tokens = []
        position = 0
        while position < len(text):
            match = TOKEN.match(text, position)
            if not match or match.end() == position:
                if text[position:].strip():
                    raise ValueError(f"cannot read {text[position:]!r}")
                break
            kind = match.lastgroup
            self.tokens.append((kind, match.group(kind)))
            position = match.end()
        # dead > 0 while reading an operand && or || does not evaluate (GitHub short-circuits both).
        self.lookup, self.functions, self.at, self.dead = lookup, functions, 0, 0

    def value(self):
        result = self.disjunction()
        if self.at != len(self.tokens):
            raise ValueError(f"unread tokens {self.tokens[self.at:]}")
        return result

    def peek(self):
        return self.tokens[self.at][1] if self.at < len(self.tokens) else None

    def take(self, expected=None):
        kind, text = self.tokens[self.at]
        if expected is not None and text != expected:
            raise ValueError(f"expected {expected}, got {text}")
        self.at += 1
        return kind, text

    def disjunction(self):
        left = self.conjunction()
        while self.peek() == "||":
            self.take()
            left = self.operand(self.conjunction, skip=truthy(left), keep=left)
        return left

    def conjunction(self):
        left = self.comparison()
        while self.peek() == "&&":
            self.take()
            left = self.operand(self.comparison, skip=not truthy(left), keep=left)
        return left

    def operand(self, read, skip, keep):
        self.dead += skip
        right = read()
        self.dead -= skip
        return keep if skip else right

    def comparison(self):
        left = self.unary()
        while self.peek() in ("==", "!="):
            _, op = self.take()
            right = self.unary()
            same = equal(left, right)
            left = same if op == "==" else not same
        return left

    def unary(self):
        if self.peek() == "!":
            self.take()
            return not truthy(self.unary())
        return self.primary()

    def primary(self):
        kind, text = self.take()
        if text == "(":
            inner = self.disjunction()
            self.take(")")
            return inner
        if kind == "str":
            return text[1:-1].replace("''", "'")
        if kind == "num":
            return int(text)
        if text in ("true", "false"):
            return text == "true"
        if text == "null":
            return None
        if self.peek() == "(":
            self.take()
            args = []
            while self.peek() != ")":
                args.append(self.disjunction())
                if self.peek() == ",":
                    self.take()
            self.take(")")
            return None if self.dead else self.functions[text](*args)
        return None if self.dead else self.lookup(text)


def truthy(value):
    return value not in (None, False, 0, "")


def equal(left, right):
    if isinstance(left, str) and isinstance(right, str):
        return left.lower() == right.lower()
    if type(left) is type(right):
        return left == right
    # Mixed types: GitHub compares them as numbers; none of these conditions relies on it.
    return False


def simulate(workflow, inputs, outputs, github=None):
    """Each job's result when every job that runs succeeds, with `outputs` per job and `inputs` for the run."""
    jobs = workflow["jobs"]
    github = {"run_attempt": "1", "event_name": "workflow_dispatch", "ref": "refs/heads/main",
              "repository": "sunrise-org/sunrise", **(github or {})}
    results = {}

    def ready(name):
        return all(need in results for need in needs_of(jobs[name]))

    while len(results) < len(jobs):
        pending = [name for name in jobs if name not in results and ready(name)]
        if not pending:
            raise AssertionError("the job graph has a cycle")
        for name in pending:
            before = upstream(jobs, name)

            def lookup(path, name=name):
                head, _, rest = path.partition(".")
                if head == "needs":
                    job, _, field = rest.partition(".")
                    if job not in needs_of(jobs[name]):
                        raise AssertionError(f"{name} reads needs.{job}, which it does not need")
                    if field == "result":
                        return results[job]
                    output = field.removeprefix("outputs.")
                    return outputs.get(job, {}).get(output, "") if results[job] == "success" else ""
                if head == "inputs":
                    return inputs.get(rest, "")
                if head == "github":
                    return github.get(rest, "")
                if head == "vars":
                    return ""
                raise AssertionError(f"{name}: unknown context {path}")

            functions = {
                "always": lambda: True,
                "cancelled": lambda: False,
                "failure": lambda: False,
                "success": lambda before=before: all(results[job] == "success" for job in before),
                "contains": lambda haystack, needle: (needle in haystack if isinstance(haystack, list)
                                                      else str(needle).lower() in str(haystack).lower()),
                "fromJSON": json.loads,
            }
            text = condition(jobs[name])
            run = truthy(Expression(text, lookup, functions).value()) if text else True
            if not STATUS.search(text):
                run = run and functions["success"]()
            results[name] = "success" if run else "skipped"
    return results


class ExplicitStatusTests(unittest.TestCase):
    def test_every_release_job_after_a_job_that_can_skip_checks_status_explicitly(self):
        for name in ("nightly-build.yml", "release.yml"):
            jobs = load(name)["jobs"]
            for job in jobs:
                skippable = sorted(other for other in upstream(jobs, job) if "if" in jobs[other])
                if skippable:
                    with self.subTest(workflow=name, job=job):
                        self.assertRegex(condition(jobs[job]), STATUS,
                                         f"{job} comes after {skippable}, which can be skipped: GitHub's implicit "
                                         "success() would skip it too")

    def test_the_daily_nightly_follows_green_whatever_was_skipped_before_it(self):
        jobs = load("daily.yml")["jobs"]
        for job in ("nightly-branch", "nightly"):
            self.assertRegex(condition(jobs[job]), STATUS)


class NightlyBuildPathTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("nightly-build.yml")

    def run_kind(self, kind, break_glass="false", attempt="1", sdk_needed="true"):
        outputs = {"pick": {"build": "true", "publish": "true", "break_glass": break_glass},
                   "complete": {"complete": "true"}, "sdk": {"needed": sdk_needed}}
        return simulate(self.workflow, {"kind": kind, "dry_run": False, "issue": ""}, outputs,
                        {"run_attempt": attempt})

    def test_an_ordinary_release_builds_signs_and_publishes_with_break_glass_skipped(self):
        for kind in ("dev", "nightly", "stable"):
            with self.subTest(kind=kind):
                results = self.run_kind(kind)
                for job in ("break-glass-umceko", "break-glass-aria", "break-glass"):
                    self.assertEqual(results[job], "skipped")
                for job in ("exporters", "build", "complete", "sign", "publish"):
                    self.assertEqual(results[job], "success", job)
                self.assertEqual(results["full-daily"], "success" if kind == "stable" else "skipped")
                for job in ("sdk", "sdk-publish", "sdk-index"):
                    self.assertEqual(results[job], "success" if kind == "dev" else "skipped", job)

    def test_a_dev_build_signs_and_publishes_whether_or_not_its_sdk_had_to_be_built(self):
        for needed, publish in (("true", "success"), ("false", "skipped")):
            with self.subTest(sdk_needed=needed):
                results = self.run_kind("dev", sdk_needed=needed)
                self.assertEqual(results["sdk-publish"], publish)
                for job in ("sdk", "sdk-index", "build", "sign", "publish"):
                    self.assertEqual(results[job], "success", job)

    def test_a_break_glass_release_publishes_after_both_approvals_in_its_first_attempt_only(self):
        results = self.run_kind("stable", break_glass="true")
        for job in ("break-glass-umceko", "break-glass-aria", "break-glass", "build", "sign", "publish"):
            self.assertEqual(results[job], "success", job)
        retry = self.run_kind("stable", break_glass="true", attempt="2")
        for job in ("break-glass", "exporters", "build", "sign", "publish"):
            self.assertEqual(retry[job], "skipped", job)


class ReleasePathTests(unittest.TestCase):
    def test_a_dev_build_starts_right_after_the_decision(self):
        results = simulate(load("release.yml"), {"kind": "dev", "dry_run": False},
                           {"decide": {"release": "true", "publish": "true"}})
        self.assertEqual(results, {"decide": "success", "nightly": "skipped", "dev": "success"})


class DailyPathTests(unittest.TestCase):
    def test_a_green_daily_reaches_the_nightly_when_its_sdk_already_exists(self):
        workflow = load("daily.yml")
        for needed, publish in (("true", "success"), ("false", "skipped")):
            with self.subTest(sdk_needed=needed):
                outputs = {"resolve": {"skip": "false", "legs": "", "dry_run": "false", "branch": "main",
                                       "parity_required": "true"},
                           "summary": {"green": "true", "sdk_green": "true"}, "sdk": {"needed": needed}}
                results = simulate(workflow, {}, outputs)
                self.assertEqual(results["sdk-publish"], publish)
                for job in ("green", "nightly-branch", "nightly"):
                    self.assertEqual(results[job], "success", job)


if __name__ == "__main__":
    unittest.main()
