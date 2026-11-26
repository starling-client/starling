"""tools/parallel_unittest.py: one test file per worker, the full discovery's outcome, unittest's own summary."""
import argparse
import io
from pathlib import Path
import re
import tempfile
import textwrap
import unittest
from unittest import mock

from tools import parallel_unittest as parallel
from tools import pr_ready

FAILING = """\
.F
======================================================================
FAIL: test_sum (test_maths.Maths.test_sum)
----------------------------------------------------------------------
Traceback (most recent call last):
AssertionError: 3 != 4

----------------------------------------------------------------------
Ran 2 tests in 0.004s

FAILED (failures=1, skipped=1)
"""

# A small tests/ directory: one file passes with a skip, one fails, one cannot be imported, one ends its process
# from inside a test, and one is not a test file at all.
FILES = {
    "test_passes.py": """\
        import unittest

        class Passes(unittest.TestCase):
            def test_passes(self):
                self.assertEqual(1 + 1, 2)

            @unittest.skip("not on this platform")
            def test_skipped(self):
                self.fail("must not run")
        """,
    "test_fails.py": """\
        import unittest

        class Fails(unittest.TestCase):
            def test_fails(self):
                self.assertEqual("kept", "lost", "the failure text stays")
        """,
    "test_broken.py": """\
        import unittest
        import no_such_module_for_parallel_unittest
        """,
    "test_exits.py": """\
        import os
        import unittest

        class Exits(unittest.TestCase):
            def test_exits(self):
                print("about to leave", flush=True)
                os._exit(3)
        """,
    "helper.py": """\
        raise AssertionError("not a test file: never imported")
        """,
}


class ParseTests(unittest.TestCase):
    def test_a_failed_file(self):
        result = parallel.parse("test_maths.py", FAILING, 1)
        self.assertEqual((result.ran, result.finished, result.failed), (2, True, True))
        self.assertEqual(result.counts, {"failures": 1, "errors": 0, "skipped": 1, "expected failures": 0,
                                         "unexpected successes": 0})

    def test_a_passed_file_and_a_file_without_tests(self):
        result = parallel.parse("test_a.py", "...\n" + "-" * 70 + "\nRan 3 tests in 0.1s\n\nOK (expected failures=1)\n")
        self.assertEqual((result.ran, result.counts["expected failures"], result.failed), (3, 1, False))
        empty = parallel.parse("test_b.py", "\n" + "-" * 70 + "\nRan 0 tests in 0.000s\n\nNO TESTS RAN\n", 5)
        self.assertEqual((empty.ran, empty.finished, empty.failed), (0, True, False))

    def test_only_the_last_summary_counts(self):
        """A test may print text that looks like a summary; unittest's own comes last."""
        output = "Ran 9 tests in 1.0s\n\nFAILED (errors=9)\n" + "-" * 70 + "\nRan 1 test in 0.0s\n\nOK\n"
        result = parallel.parse("test_prints.py", output)
        self.assertEqual((result.ran, result.counts["errors"], result.failed), (1, 0, False))

    def test_a_worker_that_ends_without_a_summary_is_one_error(self):
        for output, status in (("about to leave\n", 3), (FAILING, -11), ("Ran 1 test in 0.0s\n\nOK\n", 1),
                               ("could not start\n", None)):
            with self.subTest(status=status):
                result = parallel.parse("test_x.py", output, status)
                self.assertFalse(result.finished)
                self.assertTrue(result.failed)
                self.assertEqual(result.counts["errors"], 1)


class SummaryTests(unittest.TestCase):
    def test_counts_add_up_in_unittests_order(self):
        results = [parallel.parse("test_maths.py", FAILING, 1),
                   parallel.parse("test_a.py", "Ran 3 tests in 0.1s\n\nOK (skipped=2, expected failures=1)\n"),
                   parallel.parse("test_x.py", "about to leave\n", 3)]
        text, status = parallel.summary(results, 1.5)
        self.assertEqual(text, "-" * 70 + "\nRan 5 tests in 1.500s\n\n"
                         "FAILED (failures=1, errors=1, skipped=3, expected failures=1)")
        self.assertEqual(status, 1)
        self.assertEqual(pr_ready.unittest_passed(text),
                         "5 run, FAILED (failures=1, errors=1, skipped=3, expected failures=1)")

    def test_passing_files(self):
        results = [parallel.parse("test_a.py", "Ran 1 test in 0.1s\n\nOK\n"),
                   parallel.parse("test_b.py", "Ran 0 tests in 0.0s\n\nNO TESTS RAN\n", 5)]
        text, status = parallel.summary(results, 0.25)
        self.assertEqual((text, status), ("-" * 70 + "\nRan 1 test in 0.250s\n\nOK", 0))
        self.assertEqual(pr_ready.unittest_passed(text), "1 passed")
        self.assertEqual(parallel.summary([], 0)[1], 5)

    def test_a_failing_files_report_keeps_unittests_text(self):
        report = parallel.report(parallel.parse("test_maths.py", FAILING, 1, 2.0))
        self.assertEqual(report, "test_maths.py: 2.0s\n.F\n" + "=" * 70 +
                         "\nFAIL: test_sum (test_maths.Maths.test_sum)\n" + "-" * 70 +
                         "\nTraceback (most recent call last):\nAssertionError: 3 != 4")


class RunTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "tests").mkdir()
        (self.root / "tests" / "fixtures").mkdir()
        (self.root / "tests" / "fixtures" / "test_nested.py").write_text("raise AssertionError('not discovered')\n")
        for name, source in FILES.items():
            (self.root / "tests" / name).write_text(textwrap.dedent(source))

    def test_the_files_full_discovery_loads(self):
        (self.root / "tests" / "test-not-a-module.py").write_text("")
        self.assertEqual(parallel.test_files(self.root / "tests"),
                         ["test_broken.py", "test_exits.py", "test_fails.py", "test_passes.py"])

    def test_every_file_counts_and_failures_keep_their_output(self):
        out = io.StringIO()
        status = parallel.run(jobs=2, cwd=self.root, out=out)
        output = out.getvalue()
        self.assertEqual(status, 1)
        # The import failure is unittest's own error, as in the full discovery; the exit is one error of ours.
        self.assertEqual(re.findall(r"^(?:FAIL|ERROR): .*$", output, re.M),
                         ["ERROR: test_broken (unittest.loader._FailedTest.test_broken)",
                          "FAIL: test_fails (test_fails.Fails.test_fails)"])
        self.assertIn("ModuleNotFoundError: No module named 'no_such_module_for_parallel_unittest'", output)
        self.assertIn("AssertionError: 'kept' != 'lost'", output)
        self.assertIn("the failure text stays", output)
        self.assertIn("test_exits.py: ended without unittest's summary (exit status 3), counted as 1 error\n", output)
        self.assertIn("about to leave", output)
        self.assertNotIn("test_passes.py", output)
        lines = output.splitlines()
        self.assertRegex(lines[-3], r"^Ran 4 tests in \d+\.\d{3}s$")
        self.assertEqual(lines[-1], "FAILED (failures=1, errors=2, skipped=1)")

    def test_a_worker_that_raises_is_still_counted(self):
        with mock.patch.object(parallel, "run_file", side_effect=RuntimeError("worker lost")):
            out = io.StringIO()
            status = parallel.run(jobs=2, cwd=self.root, out=out)
        self.assertEqual(status, 1)
        self.assertEqual(out.getvalue().splitlines()[-1], "FAILED (errors=4)")
        self.assertIn("test_passes.py: ended without unittest's summary (exit status None), counted as 1 error\n"
                      "RuntimeError: worker lost", out.getvalue())


class PrReadyTests(unittest.TestCase):
    def test_pr_ready_runs_them_in_parallel_unless_asked_not_to(self):
        for serial, command in ((False, "python3 tools/parallel_unittest.py"),
                                (True, "python3 -m unittest discover -s tests")):
            with self.subTest(serial=serial), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                options = argparse.Namespace(host="local", quick=True, harnesses=False, serial_python=serial)
                steps = pr_ready.plan(options, "origin/main", {"README.md"}, {}, set(), {})
                self.assertEqual({step.name: step for step in steps}["Python tests"].command, command)


if __name__ == "__main__":
    unittest.main()
