"""The Python tests as Starling's CI runs them (ci.yml's lint job): Sunrise's suite, `unittest discover -s tests`, less
the modules that test Sunrise's own release and CI pipeline. Starling doesn't run that pipeline: its workflows
(daily.yml, nightly-build.yml, release.yml, sdk-publish.yml, game-build.yml, dyna-ci.yml) aren't in this tree, and
ci.yml is Starling's. Every other module runs as Sunrise has it, and the skipped ones stay in the tree for the sync.

    python tools/starling/py_tests.py [-v]
"""
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Sunrise's release and CI pipeline, which Starling doesn't run.
SUNRISE_PIPELINE = {
    "test_break_glass",
    "test_check_pe_arch",
    "test_ci_memory_report",
    "test_ci_pr_filters",
    "test_ci_runners",
    "test_ci_rust_changes",
    "test_daily_fuzz",
    "test_daily_rows_coverage",
    "test_mod_tool",
    "test_nightly",
    "test_nightly_set",
    "test_release_branches",
    "test_release_job_graph",
    "test_release_train",
}


def module_of(test):
    """The test module a case comes from; a module that failed to import is named by its placeholder's id."""
    module = type(test).__module__
    if module == "unittest.loader":
        return test.id().rsplit(".", 1)[-1]
    return module.split(".")[0]


def without(suite, skipped, dropped):
    kept = unittest.TestSuite()
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            kept.addTest(without(test, skipped, dropped))
        elif module_of(test) in skipped:
            dropped.add(module_of(test))
        else:
            kept.addTest(test)
    return kept


def main(argv):
    # As `python -m unittest discover -s tests` from the root: the root importable (`from tools.ci import ...`),
    # and discovery puts tests/ first.
    sys.path[0] = str(ROOT)
    suite = unittest.TestLoader().discover(str(ROOT / "tests"))
    dropped = set()
    suite = without(suite, SUNRISE_PIPELINE, dropped)
    print(f"Sunrise's pipeline, not run here: {', '.join(sorted(dropped)) or 'none found'}", flush=True)
    result = unittest.TextTestRunner(verbosity=2 if "-v" in argv else 1).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
