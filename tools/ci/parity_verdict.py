#!/usr/bin/env python3
"""Reduce this attempt's private parity log to harness names and outcomes for the daily summary.

One share: `parity_verdict.py FOLDER REPORT --bucket I/N`. Every share:
`parity_verdict.py --merge REPORT VERDICT... --expect N --plan SELECTION`, which is complete only when the
shares reported exactly once each for GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT and COMMIT, and cover the planned selection.
"""
import argparse
import json
import os
import re
import shutil
import sys
from pathlib import Path

NAME = r"[a-z0-9_]+"
LABEL = rf"{NAME}(?:#\d+)?"
RECORD = re.compile(rf"^(PASS|FAIL) ({LABEL}) \d+s(?: |$)")
FINISHED = re.compile(r"^(\d+) passed, (\d+) failed in [\d.]+ min\.$")


def verdict(log, expected, status):
    """Latest outcome per selected shard. A footer alone does not prove the selected set ran."""
    labels, counts = {}, None
    for line in log.splitlines():
        if (found := RECORD.match(line)) and found[2] in expected:
            labels[found[2]] = found[1] == "PASS"
        if finished := FINISHED.fullmatch(line):
            counts = (int(finished[1]), int(finished[2]))
    passes = sum(labels.values())
    failures = len(labels) - passes
    complete = (bool(expected) and labels.keys() == expected and counts == (passes, failures)
                and status == str(int(failures != 0)))
    harnesses = {}
    for label, passed in labels.items():
        name = label.split("#", 1)[0]
        harnesses[name] = harnesses.get(name, True) and passed
    # An incomplete run proves no harness passed: its passes stay out of the uploaded verdict.
    return ({"passed": sorted(name for name, passed in harnesses.items() if passed) if complete else [],
             "failed": sorted(name for name, passed in harnesses.items() if not passed),
             "complete": complete}, labels)


def unique_names(value, pattern):
    if isinstance(value, list) and all(isinstance(name, str) and re.fullmatch(pattern, name) for name in value):
        names = set(value)
        if len(names) == len(value):
            return names
    return None


def merge(reports, expected, plan, scope):
    """Require one finished verdict per share of this exact run and disjoint coverage of its plan.

    An unfinished run proves no passes, but keeps the failures from valid reports of this run.
    """
    planned = unique_names(plan, LABEL)
    problems = [] if planned else ["the planned selection is missing, empty or malformed"]
    shares, selected, passed, failed = set(), set(), set(), set()
    for report in reports:
        if not isinstance(report, dict):
            problems.append("a share verdict is unreadable or not an object")
            continue
        if report.get("scope") != scope:
            problems.append("a share verdict does not match this run's scope")
            continue
        share, count = report.get("bucket"), report.get("of")
        if type(share) is not int or not 0 <= share < expected or type(count) is not int or count != expected:
            problems.append("a share verdict has an invalid bucket or share count")
            continue
        complete = report.get("complete")
        labels = unique_names(report.get("selected"), LABEL)
        share_passed = unique_names(report.get("passed"), NAME)
        share_failed = unique_names(report.get("failed"), NAME)
        if type(complete) is not bool or labels is None or share_passed is None or share_failed is None:
            problems.append(f"share {share} has a malformed selection or verdict")
            continue
        names = {label.split("#", 1)[0] for label in labels}
        outcomes = share_passed | share_failed
        if not outcomes <= names or share_passed & share_failed or (
                complete and (not labels or outcomes != names)) or (not complete and share_passed):
            problems.append(f"share {share} has outcomes inconsistent with its selection or completeness")
            continue
        passed.update(share_passed)
        failed.update(share_failed)
        if share in shares:
            problems.append(f"share {share} reported twice")
            continue
        shares.add(share)
        if selected & labels:
            problems.append("two shares ran the same harness")
        selected.update(labels)
        if not complete:
            problems.append(f"share {share} did not finish")
    if shares != set(range(expected)):
        problems.append(f"shares {sorted(shares) or 'none'} reported, not 0..{expected - 1}")
    if planned and selected != planned:
        problems.append(f"{len(planned - selected)} planned harnesses ran in no share, "
                        f"{len(selected - planned)} ran outside the plan")
    return {
        "passed": sorted(passed - failed) if not problems else [],
        "failed": sorted(failed),
        "complete": not problems,
        "problems": problems,
        "shares": {"reported": len(shares), "expected": expected},
        "scope": scope,
    }


def main(*arguments):
    if arguments and arguments[0] == "--merge":
        return merge_main(arguments[1:])
    parser = argparse.ArgumentParser(description="Reduce one share's private parity log to a verdict.")
    parser.add_argument("folder")
    parser.add_argument("report")
    parser.add_argument("--bucket", metavar="I/N", help="which share of the run this log is")
    args = parser.parse_args(arguments)
    share = None
    if args.bucket:
        try:
            index, count = (int(part) for part in args.bucket.split("/", 1))
        except ValueError:
            parser.error(f"--bucket is I/N, not {args.bucket}")
        if count < 1 or not 0 <= index < count:
            parser.error(f"--bucket is I/N with 0 <= I < N, not {args.bucket}")
        share = {"index": index, "of": count}
    folder = Path(args.folder)
    run_id, run_attempt, commit = (folder / "scope").read_text().splitlines()
    log, selection, exit_status = (folder / name for name in ("parity.log", "selection.json", "exit"))
    expected = set(json.loads(selection.read_text())) if selection.is_file() else set()
    result, labels = verdict(log.read_text(errors="replace") if log.is_file() else "", expected,
                             exit_status.read_text().strip() if exit_status.is_file() else "")
    result["selected"] = sorted(expected)
    result["bucket"] = None if share is None else share["index"]
    result["of"] = 1 if share is None else share["of"]
    result["scope"] = {"run_id": run_id, "run_attempt": run_attempt, "commit": commit}
    Path(args.report).write_text(json.dumps(result, indent=2) + "\n")
    print_verdict(result)

    # Preserve failed-shard and first-retry logs across reuse of the slot's scratch directory.
    logs = Path(".local/parity-runner/logs")
    for path in [*logs.glob("*.first-run.log"), *(logs / f"{label}.log" for label, passed in labels.items() if not passed)]:
        if path.is_file():
            shutil.copyfile(path, folder / path.name)


def read_evidence(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        return None


def merge_main(arguments):
    parser = argparse.ArgumentParser(description="Merge the daily run's share verdicts into one parity.json.")
    parser.add_argument("report", help="the merged verdict to write")
    parser.add_argument("sources", nargs="*", type=Path, help="each share's verdict")
    parser.add_argument("--expect", type=int, required=True, help="how many shares the run has")
    parser.add_argument("--plan", type=Path, required=True, help="the planned selection every share was carved from")
    args = parser.parse_args(arguments)
    if args.expect < 1:
        parser.error("--expect must be positive")
    scope = {"run_id": os.environ["GITHUB_RUN_ID"], "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"],
             "commit": os.environ["COMMIT"]}
    result = merge((read_evidence(path) for path in args.sources), args.expect, read_evidence(args.plan), scope)
    Path(args.report).write_text(json.dumps(result, indent=2) + "\n")
    print_verdict(result)
    for problem in result["problems"]:
        print(f"not complete: {problem}", file=sys.stderr)
    return 0


def print_verdict(result):
    for name in result["passed"] if result["complete"] else []:
        print(f"PASS {name}")
    for name in result["failed"]:
        print(f"FAIL {name}")


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:]) or 0)
