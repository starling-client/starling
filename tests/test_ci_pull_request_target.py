"""pull_request_target workflows run main's code only (Sunrise's condition for allowing the event, 2026-11-16).

Such a job gets a token that can write and the repository's secrets, even for a fork's pull request. So every
checkout in it is main's (`ref: main`, this repository), no step checks out or switches to another commit, and no
step names the pull request's head or merge commit: its code is never on the runner as a working tree.

The repository's Actions policy allows the event for these files by name only, so renaming one, or giving the
event to another file, stops it running until the policy's list changes too.
"""
import re
import unittest
from pathlib import Path

from tests.test_ci_concurrency import yaml

ROOT = Path(__file__).resolve().parents[1]
# The files the repository's Actions policy lets run on pull_request_target; change it with the policy.
POLICY_FILES = {"cancel-on-close.yml", "overlap-note.yml", "pr-labels.yml"}
# A pull request's code, by name: its head commit or branch, its merge commit, or its refs.
PULL_REQUEST_CODE = re.compile(r"pull_request\.head\.(sha|ref)\b|github\.head_ref\b|merge_commit_sha|refs/pull/|"
                               r"\bpull/(\d+|\$\{\{[^}]*\}\})/(head|merge)\b")
SWITCHES_TREE = re.compile(r"\bgit\s+(checkout|switch|worktree|reset\s+--hard|restore)\b|\bgh\s+pr\s+checkout\b")


def pull_request_target_jobs():
    for path in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        workflow = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
        on = workflow.get("on", {})
        if "pull_request_target" in (on if isinstance(on, (dict, list)) else [on]):
            for name, job in workflow["jobs"].items():
                yield f"{path.name} {name}", job


def problems(job):
    if "uses" in job:
        return ["calls a reusable workflow, whose checkouts this check cannot see"]
    found = []
    for step in job.get("steps", []):
        label = step.get("name") or step.get("uses") or step.get("run", "")[:40]
        if step.get("uses", "").startswith("actions/checkout"):
            options = step.get("with", {})
            if options.get("ref") != "main" or options.get("repository", "${{ github.repository }}") not in (
                    "${{ github.repository }}", "sunrise-org/sunrise"):
                found.append(f"{label}: checks out something other than main")
        if SWITCHES_TREE.search(step.get("run", "")):
            found.append(f"{label}: switches the working tree to another commit")
        if PULL_REQUEST_CODE.search(repr(step)):
            found.append(f"{label}: names the pull request's code")
    return found


class PullRequestTargetTests(unittest.TestCase):
    def test_pull_request_target_jobs_never_check_out_pull_request_code(self):
        jobs = list(pull_request_target_jobs())
        self.assertTrue(jobs, "the workflows this guards are found")
        for where, job in jobs:
            with self.subTest(job=where):
                self.assertEqual(problems(job), [])

    def test_only_the_files_the_actions_policy_names_use_pull_request_target(self):
        self.assertEqual({where.split(" ", 1)[0] for where, _ in pull_request_target_jobs()}, POLICY_FILES)

    def test_a_checkout_of_the_pull_requests_head_is_refused(self):
        head = {"steps": [{"uses": "actions/checkout@v7",
                           "with": {"ref": "${{ github.event.pull_request.head.sha }}"}}]}
        unpinned = {"steps": [{"uses": "actions/checkout@v7"}]}
        fetched = {"steps": [{"run": "git fetch origin pull/7/head && git checkout FETCH_HEAD"}]}
        self.assertEqual(len(problems(head)), 2)
        self.assertEqual(problems(unpinned), ["actions/checkout@v7: checks out something other than main"])
        self.assertEqual([problem.split(": ")[-1] for problem in problems(fetched)],
                         ["switches the working tree to another commit", "names the pull request's code"])
        self.assertEqual(problems({"steps": [{"uses": "actions/checkout@v7", "with": {"ref": "main"}}]}), [])


if __name__ == "__main__":
    unittest.main()
