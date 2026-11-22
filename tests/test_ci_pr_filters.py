"""Maintainer-requested fork verification and skips a job only when the complete diff leaves its inputs untouched."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml  # noqa: E402

# Each job a fork's pull request can skip, and the selections that keep it.
GATED = {"verify-tree": ("tooling_required", "rust_required"), "verify-lint": ("rust_required",),
         "verify-test": ("rust_required",)}


def workflow():
    return yaml.load((ROOT / ".github/workflows/ci.yml").read_text(), Loader=yaml.BaseLoader)


class PullRequestSelectionTests(unittest.TestCase):
    def test_the_trigger_has_no_path_filter(self):
        # GitHub matches a path filter against the first 300 changed files only: a Rust change later in a long list
        # would not start the workflow.
        on = workflow()["on"]
        self.assertEqual(set(on), {"pull_request", "schedule", "workflow_dispatch"})
        self.assertEqual(on["pull_request"], {"types": ["labeled"]})

    def test_each_job_skips_only_on_its_own_selection(self):
        jobs = workflow()["jobs"]
        changes = jobs["changes"]
        self.assertIn("github.event.pull_request.head.repo.full_name != github.repository", changes["if"])
        select = next(step for step in changes["steps"] if step.get("id") == "selection")
        self.assertEqual(select["run"],
                         'python3 tools/ci/rust_changes.py --all "$BASE_SHA" "$HEAD_SHA" >> "$GITHUB_OUTPUT"')
        for job, outputs in GATED.items():
            with self.subTest(job=job):
                self.assertEqual(jobs[job]["needs"], "changes")
                condition = jobs[job]["if"]
                # A skipped or failed selection leaves the outputs empty: `!= 'false'` then runs the job, and
                # `!cancelled()` keeps a skipped `changes` (the schedule) from skipping it.
                self.assertTrue(condition.startswith("!cancelled() && (github.event_name == 'schedule' ||"))
                for output in outputs:
                    self.assertIn(f"needs.changes.outputs.{output} != 'false'", condition)
                self.assertIn("github.event.label.name == 'ci:verify'", condition)


if __name__ == "__main__":
    unittest.main()
