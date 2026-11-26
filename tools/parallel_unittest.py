#!/usr/bin/env python3
"""`python3 -m unittest discover -s tests`, one test file per worker: the same tests and outcomes in a fraction of the
time (#5043: 45 s instead of 6 min on 20 threads).

    python3 tools/parallel_unittest.py             # every tests/test*.py, os.cpu_count() files at a time
    python3 tools/parallel_unittest.py --jobs 4

Each file runs in its own process as `python -m unittest discover -s tests -p <file>`, from the same working
directory: the same top-level directory and import rules as the full discovery, one file at a time. tests/ holds no
packages, so its top-level files are every file the full discovery loads. A file that fails to import is an error,
as in the full discovery (unittest reports it as a failed test of its own); a worker that ends without unittest's
summary (a crash, an exit from a test) counts as one error too, with its output shown.

On stderr, as unittest writes: the failing files' output as unittest printed it, then one summary in unittest's own
format ("Ran N tests in Xs", then "OK" or "FAILED (failures=..., errors=..., skipped=...)"), so tools/pr_ready.py
reads it as it reads the sequential run's. Like unittest, it exits 0 when everything passed and 1 otherwise.

What it can't match is a test that depends on another file running first in the same process: run alone, such a
file fails here (and when run by itself), and is fixed by making the file load what it needs itself.
"""

import argparse
import concurrent.futures
import fnmatch
import os
from pathlib import Path
import re
import subprocess
import sys
import time

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# unittest's result counts, in the order its summary line lists them.
COUNTS = ("failures", "errors", "skipped", "expected failures", "unexpected successes")
SEPARATOR = "-" * 70
# unittest's discovery only imports files whose names are module names.
MODULE_NAME = re.compile(r"[_a-z]\w*\.py$", re.IGNORECASE)


class FileResult:
    """One test file's run: tests run, unittest's counts, its output and exit status, and whether unittest finished."""

    def __init__(self, name, ran=0, counts=None, output="", finished=True, seconds=0.0, status=0):
        self.name, self.ran, self.output, self.finished, self.seconds = name, ran, output, finished, seconds
        self.status = status
        self.counts = dict.fromkeys(COUNTS, 0) if counts is None else counts

    @property
    def failed(self):
        return bool(self.counts["failures"] or self.counts["errors"] or self.counts["unexpected successes"])


def parse(name, output, returncode=0, seconds=0.0):
    """A file's result from unittest's output: the last "Ran N tests" and the outcome line after it.

    Without them, or with an exit status that disagrees with them, the worker did not finish: one error.
    """
    counts = dict.fromkeys(COUNTS, 0)
    ran = list(re.finditer(r"^Ran (\d+) tests? in ", output, re.M))
    outcome = re.search(r"^(OK|FAILED|NO TESTS RAN)(?: \((.*)\))?$", output[ran[-1].end():], re.M) if ran else None
    if outcome and outcome.group(2):
        for part in outcome.group(2).split(", "):
            key, _, value = part.partition("=")
            if key in counts and value.isdigit():
                counts[key] = int(value)
    # unittest exits 0 on OK, 1 on FAILED and 5 when a file holds no tests.
    expected = {"OK": 0, "FAILED": 1, "NO TESTS RAN": 5}.get(outcome.group(1)) if outcome else None
    finished = outcome is not None and expected == returncode
    if not finished:
        counts["errors"] += 1
    return FileResult(name, int(ran[-1].group(1)) if ran else 0, counts, output, finished, seconds, returncode)


def test_files(start, pattern="test*.py"):
    """The files the full discovery loads from start's top level, in its (sorted) order."""
    return sorted(path.name for path in Path(start).iterdir()
                  if path.is_file() and fnmatch.fnmatch(path.name, pattern) and MODULE_NAME.match(path.name))


def run_file(name, start="tests", cwd=None):
    """Runs one test file in its own process, as the full discovery would load it."""
    command = [sys.executable, "-m", "unittest", "discover", "-s", start, "-p", name]
    started = time.monotonic()
    try:
        process = subprocess.run(command, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, text=True, errors="replace")
    except OSError as error:
        return parse(name, f"could not start {name}: {error}\n", None, time.monotonic() - started)
    return parse(name, process.stdout, process.returncode, time.monotonic() - started)


def report(result):
    """What to show of a failing file: unittest's own failure text, everything before its summary."""
    if not result.finished:
        return (f"{result.name}: ended without unittest's summary (exit status {result.status}), counted as 1 error\n"
                f"{result.output.rstrip()}")
    body = result.output.rsplit(f"\n{SEPARATOR}\nRan ", 1)[0]
    return f"{result.name}: {result.seconds:.1f}s\n{body.rstrip()}"


def summary(results, seconds):
    """unittest's summary for all the files together, and its exit status."""
    ran = sum(result.ran for result in results)
    totals = {key: sum(result.counts[key] for result in results) for key in COUNTS}
    failed = any(result.failed for result in results)
    details = [f"{key}={totals[key]}" for key in COUNTS if totals[key]]
    word = "FAILED" if failed else "NO TESTS RAN" if not ran and not totals["skipped"] else "OK"
    tests = "test" if ran == 1 else "tests"
    lines = [SEPARATOR, f"Ran {ran} {tests} in {seconds:.3f}s", "",
             word + (f" ({', '.join(details)})" if details else "")]
    return "\n".join(lines), 1 if failed else 5 if word == "NO TESTS RAN" else 0


def run(start="tests", pattern="test*.py", jobs=None, cwd=None, verbose=False, out=None):
    """Runs every test file, jobs at a time, prints the failing ones' output and the summary, returns the status."""
    out = out or sys.stderr
    started = time.monotonic()
    names = test_files(Path(cwd or ".") / start, pattern)
    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs or os.cpu_count() or 1) as pool:
        futures = [pool.submit(run_file, name, start, cwd) for name in names]
        # In file order, each as soon as it and the ones before it are done.
        for name, future in zip(names, futures):
            try:
                result = future.result()
            except Exception as error:  # a worker's own failure is still the file's error, never a lost file
                result = parse(name, f"{type(error).__name__}: {error}\n", None)
            results.append(result)
            if result.failed or verbose:
                print(report(result), file=out, flush=True)
    text, status = summary(results, time.monotonic() - started)
    print(text, file=out, flush=True)
    return status


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1,
                        help="test files run at once (default: os.cpu_count(), here %(default)s)")
    parser.add_argument("-s", "--start", default="tests", help="the directory to discover in (default tests)")
    parser.add_argument("-p", "--pattern", default="test*.py", help="the files to run (default test*.py)")
    parser.add_argument("--verbose", action="store_true", help="also show the output of files that passed")
    args = parser.parse_args(argv)
    if args.jobs < 1:
        parser.error("--jobs must be at least 1")
    return run(args.start, args.pattern, args.jobs, verbose=args.verbose)


if __name__ == "__main__":
    sys.exit(main())
