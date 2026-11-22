import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "ci"))
import gate_reuse  # noqa: E402

REPO = "o/r"
SHA, TREE = "s" * 40, "t" * 40


def job(name, conclusion="success", step_conclusion="success"):
    step = gate_reuse.RUST_STEPS.get(name, "Setup done")
    return {"name": name, "conclusion": conclusion, "steps": [{"name": step, "conclusion": step_conclusion}]}


def passed():
    return [job("changes"), job("lint"), job("verify"), job("verify-static")]


def run(run_id, *, path=gate_reuse.WORKFLOW, repo=REPO, event="pull_request", created="2026-11-18T10:00:00Z", jobs=None):
    return {"id": run_id, "path": path, "repo": repo, "event": event, "created": created,
            "jobs": passed() if jobs is None else jobs}


def github(*runs, orphans=()):
    answers = {
        f"repos/{REPO}/commits/{SHA}": {"commit": {"tree": {"sha": TREE}}},
        f"repos/{REPO}/actions/artifacts?name=gate-tree-{TREE}&per_page=30": {"artifacts": [
            {"created_at": r["created"], "workflow_run": {"id": r["id"]}} for r in runs] + list(orphans)},
    }
    for r in runs:
        answers[f"repos/{REPO}/actions/runs/{r['id']}"] = {
            "path": r["path"], "event": r["event"], "head_repository": {"full_name": r["repo"]}}
        answers[f"repos/{REPO}/actions/runs/{r['id']}/jobs?per_page=100"] = {"jobs": r["jobs"]}
    return answers.__getitem__


class GateReuse(unittest.TestCase):
    def test_a_passed_gate_on_the_same_tree_is_reused(self):
        self.assertEqual(gate_reuse.reuse(REPO, SHA, github(run(7)))[0], 7)

    def test_the_newest_passed_run_on_the_tree_is_taken(self):
        read = github(run(5, created="2026-11-18T09:00:00Z"), run(7),
                      run(9, created="2026-11-18T11:00:00Z", jobs=[job("lint", conclusion="failure")]))
        self.assertEqual(gate_reuse.reuse(REPO, SHA, read)[0], 7)

    def test_no_run_on_the_tree_reuses_nothing(self):
        self.assertIsNone(gate_reuse.reuse(REPO, SHA, github())[0])

    def test_a_push_run_on_the_tree_is_reused(self):
        self.assertEqual(gate_reuse.reuse(REPO, SHA, github(run(7, event="push")))[0], 7)

    def test_an_artifact_without_its_run_is_skipped(self):
        orphans = [{"created_at": "2026-11-18T12:00:00Z", "workflow_run": None},
                   {"created_at": "2026-11-18T12:00:00Z"}]
        self.assertEqual(gate_reuse.reuse(REPO, SHA, github(run(7), orphans=orphans))[0], 7)

    def test_a_dispatch_a_fork_or_another_workflow_proves_nothing(self):
        for case in (run(7, repo="fork/r"), run(7, path=".github/workflows/other.yml"),
                     run(7, event="workflow_dispatch")):
            with self.subTest(case=case):
                self.assertIsNone(gate_reuse.reuse(REPO, SHA, github(case))[0])

    def test_a_gate_that_skipped_or_failed_a_part_is_not_reused(self):
        for name in gate_reuse.GATE:
            broken_jobs = [job(name, conclusion="skipped"), job(name, conclusion="failure")]
            if name in gate_reuse.RUST_STEPS:
                broken_jobs.append(job(name, step_conclusion="skipped"))
            for broken in broken_jobs:
                with self.subTest(name=name, broken=broken):
                    jobs = [broken if j["name"] == name else j for j in passed()]
                    self.assertIsNone(gate_reuse.reuse(REPO, SHA, github(run(7, jobs=jobs)))[0])


if __name__ == "__main__":
    unittest.main()
