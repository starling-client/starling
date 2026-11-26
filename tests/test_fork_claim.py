import importlib.util
import os
import sys
import unittest
from pathlib import Path
from unittest import mock

from tools.ci import nightly_set as ns

ROOT = Path(__file__).resolve().parents[1]


class ForkClaimReviewersTest(unittest.TestCase):
    """The fork's `/claim` bot reads its reviewers file from the checkout its workflow makes: the file must exist
    in the repository and be in each job's sparse checkout, or every `/claim` fails with FileNotFoundError."""

    def reviewers_path(self):
        """The reviewers path `claim.py` sets on the `nightly_set` module it loads itself
        (through its own `sys.path` entry, a module object apart from `tools.ci.nightly_set`).
        The import's `sys.path` and `sys.modules` changes are undone afterwards."""
        path, modules = list(sys.path), dict(sys.modules)
        try:
            # A fresh top-level `nightly_set`: one another test imported keeps its globals.
            sys.modules.pop("nightly_set", None)
            with mock.patch.dict(os.environ, {"FORK_OWNER": "owner", "FORK_OWNER_ID": "1"}):
                spec = importlib.util.spec_from_file_location("fork_claim", ROOT / "tools/fork/claim.py")
                claim = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(claim)
            return claim.ns.REVIEWERS
        finally:
            sys.path[:] = path
            for name in set(sys.modules) - set(modules):
                del sys.modules[name]
            sys.modules.update(modules)

    def test_the_reviewers_file_exists_and_reads(self):
        path = ROOT / self.reviewers_path()
        self.assertTrue(path.is_file(), path)
        ns.read_reviewers(path.read_text())

    def test_every_job_checks_the_reviewers_file_out(self):
        workflow = (ROOT / ".github/workflows/fork-claim.yml").read_text()
        jobs = workflow.count("run: python3 tools/fork/claim.py ")
        self.assertEqual(jobs, 2)
        self.assertEqual(workflow.count(f"            {self.reviewers_path()}\n"), jobs)


if __name__ == "__main__":
    unittest.main()
