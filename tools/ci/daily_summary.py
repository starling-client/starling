#!/usr/bin/env python3
"""The daily run's summary (.github/workflows/daily.yml): the `daily-green` tag and one issue per failing test.

Reads this run's jobs (names, conclusions and step names from the Actions API, never a log) and the legs' verdict
files (tools/ci/ignored_tests.py --report: test identity, kind, panic location, policy file), then:
- every leg green: moves the tag `daily-green` to the commit;
- a failing test: files `Daily: <crate> <test>` (label `bug`) with its panic location, this run as the first that
  saw it and as suspects the pull requests merged since the previous daily run that changed its package's folders
  (saying whether that run passed it, from its job's result there; with no earlier run, those since the last green
  daily), or comments on the open one; when its
  newest issue was closed by a pull request or commit that is not in this run's commit (a fix merged after the commit
  under test), it comments on that closed issue and leaves it closed instead of filing a new one;
- a leg that failed without a test name (build error, timeout, killed, cancelled after it started, skipped, or a
  leg with no sanitised verdict): one issue per leg, `Daily: <leg> did not complete`, naming the failed steps and, for a shard,
  why each run stopped as its verdict file says (signal or exit status, memory peak and limit, wall time, the tests
  still running past 60 s; stopped_text); a leg whose red asks for an action is titled as that action (ACTIONS:
  `Daily: update the machine sample for the mod API change`);
- a failed check of the SDK a leg just built (SDK_CHECKS: the native linkage probe, the API-source check, the source
  stage): one issue per check and platform, `Daily: sdk <check> (<platform>)`, closed only by a run in which that step
  passed; when a Linux SDK leg failed, the missing input of the legs that read it (fn-replace-check, book-quickstart) is
  said on that leg's issue, not filed again;
- a test (or leg) green again: closes its open issue with a comment.
A job cancelled before any of its steps ran (one still waiting for a runner) never ran, so it is neither red nor
green, and nor is a leg whose call was cancelled with only such jobs: nothing is filed or closed for it. A run cancelled
by hand never gets here: daily.yml runs the summary only when the run was not cancelled.
It also outputs `sdk_green`: whether the legs sdk-publish waits for (SDK_LEGS) all ran and passed, by the same rules.
A job it does not know, or an expected leg with no job, is a red leg ("did not report"), never green; so is a
shard of the matrix (tools/ci/ignored_shards.py, which builds it) with no job of its name. Each comment
names this run's URL, and none is posted twice for one run (a rerun of failed jobs, or a retry after a partial
write, completes what is missing). `daily_summary.py --seen SHA` prints a completed real daily run of SHA, if any.
`daily_summary.py --after JOB RESULT` reports a job that runs after the summary (the SDK's publish, `sdk-publish`) the
same way: not `success` files or comments on `Daily: JOB did not complete`, naming its failed steps; `success` closes
it. The parity leg holds global green but not sdk_green. It files one issue per failing harness, with the pull
requests merged since the last green daily as suspects, and closes issues only on complete coverage.
Only a run on refs/heads/main or the release branch with DRY_RUN=false writes; otherwise it prints what it would do to
the job summary. The release branch's issues are titled apart (WRITING_REFS) and it never moves the tag.
Lookups come first and fail the job on any API error (a rate limit included), so a failed lookup never files
duplicates.

Environment: GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT, GITHUB_REF, GITHUB_SERVER_URL, GH_TOKEN, COMMIT, DRY_RUN,
RESULTS (toJSON(needs)), REPORTS (the downloaded daily-report-* artifacts), GITHUB_STEP_SUMMARY, GITHUB_OUTPUT.
"""

import datetime as dt
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci import ignored_shards  # noqa: E402

TAG = "daily-green"
LABEL = "bug"
PREFIX = "Daily: "
# The refs whose real runs write, and the title prefix of their issues: the release line's daily runs (a break-glass
# release's follow-up among them) file and close their own issues, apart from main's. Only main moves `daily-green`.
WRITING_REFS = {"refs/heads/main": PREFIX, "refs/heads/release/26.3": "Daily (release/26.3): "}
UNFINISHED = " did not complete"
DISCLOSURE = "> 🤖 Filed by Aria's agent swarm, Dyna, not by Aria."
# The daily's legs are the jobs its summary needs (RESULTS), each a called workflow: daily_legs().
PARITY = "parity"
# The parity steps whose failure is the harness result the verdict already reports: a run that found a red
# harness, and the job that turns the shares' verdicts into the run's one answer. Every other failed step in a
# parity job is infrastructure — a share that wrote its verdict and then died still has to be visible beside a
# harness failure from another share.
PARITY_RESULT_STEPS = ("Run every automatic harness", "Fail unless every share reported and no harness failed")
# The legs sdk-publish waits for (output sdk_green) until a runner with prepared data exists (owner, 2026-11-16).
SDK_LEGS = ("sdk", "machine-sample", "fn-replace-check")
# Legs whose red is an action for someone, not a break: their one issue is titled as the action, and says what to do.
ACTIONS = {
    "machine-sample": ("update the machine sample for the mod API change",
                       "sunrise-org/sunrise-machine-sample, at the pin in `tools/ci/check_machine_sample.py`, no longer "
                       "builds against this commit's mod API. Update the sample for the API change, then move the pin "
                       "to the updated sample in a pull request here."),
}
HELPERS = ("plan", "route", "fresh", "gate")
# Calls that are part of another leg (daily.yml names their jobs for it): the sdk leg is two calls, its Linux legs
# and the others. A leg's result is its worst call's.
PART_OF = {"sdk-windows-macos": "sdk"}
# The one green skip: the SDK build skips when sdk-<sha> already exists, which `sdk / fresh` found (it
# succeeded). Any other skip of an expected job is red.
FRESH = "sdk / fresh"
# Steps whose reports name failing tests. Failures outside those steps still need a job-level issue.
WINDOWS_ROWS_JOB = "windows-runtime-leases / runtime-leases"
WINDOWS_ROWS_STEP = "The Windows-only daily rows"
# The SDK legs' checks of the SDK they just built (prebuilt-game-build.yml), by step: a failed one is its own issue per
# check and platform, `Daily: sdk <check> (<platform>)`, closed only by a run in which that step passed.
SDK_BUILD = "sdk / build "
SDK_CHECKS = {
    "Two mod libraries share the game's exact Rust state": "native linkage probe",
    "The starter mod checks against the API source alone": "API-source check",
    "The source stages with this commit's build data": "source stage",
}
SDK_CHECK = "sdk-check"
# The legs that read the Linux legs' SDK: when one of those failed, a reader whose only failures are the missing input
# is accounted for by the Linux leg's own issue.
LINUX_SDK_JOBS = ("sdk / build linux", "sdk / build linux-server")
# The Linux legs' call's own result, kept apart from the sdk leg's (PART_OF): its `fresh` can fail with no published
# SDK to reuse, and then no Linux build job runs at all.
LINUX_SDK_CALL = "sdk (Linux legs)"
FN_REPLACE_JOB = "fn-replace-check / fn-replace-check"
BOOK_QUICKSTART_JOB = "book-quickstart / quickstart"
LINUX_SDK_READERS = (FN_REPLACE_JOB, BOOK_QUICKSTART_JOB)
LINUX_SDK_INPUT_STEPS = ("Run actions/download-artifact@v4", "The Linux legs are here")
# A pull request merged on its own (`Merge pull request #N from`) or as a rollup's member (`Merge #N at <sha>`).
MERGE = re.compile(r"^Merge (?:pull request #(\d+) from |#(\d+) at )")
MAX_PAGES = 100
# How far back a closed issue's fix is looked for: a fix that merged after the commit under test is hours old.
FIX_LOOKBACK = dt.timedelta(days=14)
# What closed an issue: a pull request (its merge commit) or a commit that named it.
CLOSER = """query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { issue(number: $number) {
    timelineItems(itemTypes: [CLOSED_EVENT], last: 1) { nodes { ... on ClosedEvent {
      closer { ... on PullRequest { mergeCommit { oid } } ... on Commit { oid } } } } } } }
}"""


def send(url, method, body, what):
    """One GitHub HTTP call; any HTTP error (a rate limit included) ends the job."""
    request = urllib.request.Request(
        url, method=method, data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json",
                 "X-GitHub-Api-Version": "2022-11-28"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response) if response.status != 204 else None
    except urllib.error.HTTPError as error:
        remaining = error.headers.get("x-ratelimit-remaining")
        sys.exit(f"::error::GitHub refused {what}: HTTP {error.code}"
                 + (" (rate limit exhausted)" if remaining == "0" else "")
                 + "; nothing more is written, so no duplicate issue can follow")


def api(method, path, body=None):
    """One GitHub REST call on this repository."""
    return send(f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}", method, body,
                f"{method} {path}")


def graphql(query, variables):
    """One GitHub GraphQL query; an error in its answer ends the job like an HTTP error."""
    answer = send("https://api.github.com/graphql", "POST", {"query": query, "variables": variables}, "a GraphQL query")
    if answer.get("errors"):
        sys.exit(f"::error::GitHub refused a GraphQL query: {answer['errors'][0].get('message')}; "
                 "nothing more is written, so no duplicate issue can follow")
    return answer["data"]


def pages(call, path, key=None):
    """Every item of a paginated listing; refuses to act on a listing it could not read whole."""
    items = []
    for page in range(1, MAX_PAGES + 1):
        data = call("GET", f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}")
        batch = data[key] if key else data
        items.extend(batch)
        if len(batch) < 100:
            return items
    sys.exit(f"::error::{path} has more than {MAX_PAGES * 100} items; refusing to act on a partial listing")


def daily_legs(needs):
    """The legs of this run: every call the summary job needs (toJSON(needs)) but `resolve`, a call that is part of
    another leg (PART_OF) counting as that leg. A leg added to daily.yml's summary needs is judged with no other entry;
    a job of a leg missing there is one the summary does not know, and holds green."""
    return tuple(name for name in needs if name != "resolve" and name not in PART_OF)


def leg_results(needs):
    """Each leg's call result from RESULTS (toJSON(needs)), a leg made of several calls (PART_OF) taking the first that
    did not succeed."""
    results = dict(needs)
    results[LINUX_SDK_CALL] = needs.get("sdk", {})
    for part, leg in PART_OF.items():
        if part in results and results.get(leg, {}).get("result") == "success":
            results[leg] = results[part]
    return results


def leg_jobs(jobs, legs):
    """This run's leg jobs (`<leg> / <job>`), without the called workflows' routing jobs."""
    return [job for job in jobs
            if job["name"].split(" / ", 1)[0] in legs and " / " in job["name"]
            and job["name"].split(" / ", 1)[1] not in HELPERS]


def expected_shards():
    """The `shards` leg's matrix jobs the daily run must have, from the matrix's own source
    (tools/ci/ignored_shards.py --matrix --all): every package shard and every daily-rows leg."""
    return [f"shards / ignored-tests-shards ({shard})"
            for shard in [*ignored_shards.expand(ignored_shards.SHARDS), *ignored_shards.DAILY_ROWS_LEGS.values()]]


def never_started(job):
    """A job cancelled before any step ran: it waited for a runner, or for jobs it needs, until the cancel."""
    return job["conclusion"] == "cancelled" and not any(
        step.get("conclusion") not in (None, "skipped") for step in job.get("steps") or [])


def unstarted_legs(jobs, results, legs):
    """The legs whose call was cancelled and none of whose jobs, routing jobs included, started."""
    return {leg for leg in legs if results.get(leg, {}).get("result") == "cancelled"
            and all(never_started(job) for job in jobs if job["name"].partition(" / ")[0] == leg)}


def job_green(job, fresh_ok):
    return job["conclusion"] == "success" or (
        job["conclusion"] == "skipped" and job["name"].startswith("sdk / ") and fresh_ok)


def failed_helpers(jobs, legs):
    """leg -> its routing or helper jobs that did not succeed (whose failure skips the leg's real jobs)."""
    failed = {}
    for job in jobs:
        leg, _, rest = job["name"].partition(" / ")
        if leg in legs and rest in HELPERS and job["conclusion"] not in ("success", "skipped") and not never_started(job):
            failed.setdefault(leg, []).append(job["name"])
    return failed


def failed_steps(job):
    return [step["name"] for step in job.get("steps") or []
            if step.get("conclusion") in ("failure", "cancelled", "timed_out")]


def report_for(job, reports):
    """Match a verdict to its job by shard or by the Windows rows marker."""
    name = job["name"]
    for report in reports:
        if any(name.endswith(f"({shard})") for shard in report.get("shards", ())):
            return report
        if name == WINDOWS_ROWS_JOB and report.get("windows_rows") is True:
            return report
    return None


def known(name, legs):
    """daily.yml's own jobs and its legs' jobs; any other job is one the summary cannot judge."""
    return (name in ("resolve", "schedule-dispatch", "sdk-publish-report")
            or name.startswith(("summary ", "nightly / ", "sdk-publish / "))
            or name.split(" / ", 1)[0] in legs and " / " in name)


# A run's name and a test's name as ignored_tests.py writes them; anything else in a report is not shown.
SHARD_NAME = re.compile(r"[\w#:.-]{1,200}")
TEST_NAME = re.compile(r"(?=.{1,300}$)(?:r#)?[^\W\d]\w*(?:::(?:r#)?[^\W\d]\w*)*")
STILL_RUNNING_SHOWN = 5


def number(value):
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else None


def gib(size):
    return f"{size / 1024 ** 3:.1f} GiB"


def stopped_text(entry):
    """Why one run of a shard did not complete (ignored_tests.py's report: names and numbers only), as one sentence:
    "`sunrise-client#1of2` killed by signal 9 (SIGKILL), out of memory (1 kill); memory peak 32.0 GiB of 32.0 GiB;
    after 70 min; still running: `a::b`". Unknown fields are left out, never guessed."""
    shard = entry.get("shard")
    name = f"`{shard}`" if isinstance(shard, str) and SHARD_NAME.fullmatch(shard) else "a run"
    if entry.get("in_progress") is True:
        return (f"{name} was still running when the leg ended (its time limit, a cancel, or a lost runner); which "
                "tests were running and the process's end are not known")
    parts = []
    ended = entry.get("ended_by")
    if isinstance(ended, str) and re.fullmatch(r"(?:signal|exit status) \d{1,5}(?: \(SIG[A-Z0-9]{1,10}\))?", ended):
        parts.append(f"{name} killed by {ended}" if ended.startswith("signal") else f"{name} ended with {ended}")
    else:
        reason = entry.get("reason")
        parts.append(f"{name}: {reason}" if isinstance(reason, str) and re.fullmatch(r"[a-z ()]{1,120}", reason)
                     else f"{name} did not complete")
        if (code := number(entry.get("cargo_exit"))) is not None:
            parts[-1] += f" (cargo exit {code})"
    if kills := number(entry.get("oom_kills")):
        parts[-1] += f", out of memory ({kills} kill{'s' if kills > 1 else ''} by the kernel)"
    peak, limit = number(entry.get("memory_peak_bytes")), number(entry.get("memory_max_bytes"))
    if peak is not None:
        parts.append(f"job memory peak {gib(peak)}" + (f" of {gib(limit)}" if limit is not None else ""))
    if (seconds := number(entry.get("wall_seconds"))) is not None:
        parts.append(f"after {seconds // 60} min" if seconds >= 120 else f"after {seconds} s")
    still = [test for test in entry.get("still_running") or () if isinstance(test, str) and TEST_NAME.fullmatch(test)]
    if still:
        more = len(still) - STILL_RUNNING_SHOWN + (number(entry.get("still_running_more")) or 0)
        parts.append("still running past 60 s: " + ", ".join(f"`{test}`" for test in still[:STILL_RUNNING_SHOWN])
                     + (f" and {more} more" if more > 0 else ""))
    if never := number(entry.get("never_reported")):
        parts.append(f"{never} listed test{'s' if never > 1 else ''} never reported")
    return "; ".join(parts)


def named(problem):
    return not problem["test"].startswith("identity withheld")


def verdicts(jobs, reports, results, legs, parity_report, all_legs):
    """(failing tests, unfinished legs, passing tests, green legs) of this run. `legs` are those that must pass,
    `all_legs` every leg the run called (parity too where it may be left out).

    A failing test is a dict of package, test, kind, panic, policy and leg; an unfinished leg is (leg, conclusion,
    failed steps, reason). Green means every leg job and every called workflow concluded success."""
    tests, unfinished, passed, green_legs = [], [], set(), set()
    fresh_ok = any(job["name"] == FRESH and job["conclusion"] == "success" for job in jobs)
    helpers = failed_helpers(jobs, all_legs)
    unstarted = unstarted_legs(jobs, results, all_legs)
    linux_sdk_red = {job["name"] for job in jobs if job["name"] in LINUX_SDK_JOBS and not job_green(job, fresh_ok)}
    if results.get(LINUX_SDK_CALL, {}).get("result", "success") != "success":
        linux_sdk_red.add("sdk")  # the leg's own item (its failed `fresh`, say) is the Linux legs' too
    blocked = []  # the legs that read the Linux SDK and failed only for its missing input
    for job in leg_jobs(jobs, all_legs):
        if never_started(job) or job["name"].startswith(f"{PARITY} / "):
            continue  # parity uses its own verdict, including when the job passed
        report = report_for(job, reports)
        if report:
            passed |= {(item["package"], item["test"]) for item in report["passed"]}
        if job["name"].startswith(SDK_BUILD):
            platform = job["name"][len(SDK_BUILD):]
            passed |= {("sdk", f"{SDK_CHECKS[step['name']]} ({platform})") for step in job.get("steps") or []
                       if step["name"] in SDK_CHECKS and step.get("conclusion") == "success"}
        if job_green(job, fresh_ok):
            green_legs.add(job["name"])
            continue
        if job["conclusion"] == "skipped" and job["name"].split(" / ", 1)[0] in helpers:
            continue  # its leg's failed helper is the one item for it, below
        steps = failed_steps(job)
        problems = [dict(item, leg=job["name"]) for item in (report or {}).get("problems", ())]
        by_name = [item for item in problems if named(item)]
        if job["name"].startswith("shards / "):
            tests += by_name
            if report is None:
                unfinished.append((job["name"], job["conclusion"], steps, "no verdict file: the leg ended before its tests reported"))
            elif report["incomplete"] or len(by_name) < len(problems) or not problems:
                reason = ("a test run did not complete: " + ". ".join(
                    stopped_text(entry if isinstance(entry, dict) else {}) for entry in report["incomplete"])
                          if report["incomplete"] else
                          "failed tests whose names are withheld" if problems else "failed outside its tests")
                unfinished.append((job["name"], job["conclusion"], steps, reason))
        elif job["name"] == WINDOWS_ROWS_JOB and WINDOWS_ROWS_STEP in steps and (
                by_name or (report or {}).get("incomplete")):
            tests += by_name
            # A command that failed without naming a test (a build failure beside a named one) is the leg's item too.
            if report["incomplete"] or len(by_name) < len(problems):
                unfinished.append((job["name"], job["conclusion"], steps, "a test run did not complete: " + ". ".join(
                    stopped_text(entry if isinstance(entry, dict) else {}) for entry in report["incomplete"])
                    if report["incomplete"] else "failed tests whose names are withheld"))
            elif outside := [step for step in steps if step != WINDOWS_ROWS_STEP]:
                unfinished.append((job["name"], job["conclusion"], outside, "failed outside its tests"))
        elif job["name"].startswith(SDK_BUILD) and any(step in SDK_CHECKS for step in steps):
            platform = job["name"][len(SDK_BUILD):]
            tests += [{"package": "sdk", "test": f"{SDK_CHECKS[step]} ({platform})", "kind": SDK_CHECK, "step": step,
                       "panic": None, "policy": None, "leg": job["name"]} for step in steps if step in SDK_CHECKS]
            if outside := [step for step in steps if step not in SDK_CHECKS]:
                unfinished.append((job["name"], job["conclusion"], outside, "failed outside its checks"))
        elif job["name"] in LINUX_SDK_READERS and linux_sdk_red and steps and set(steps) <= set(LINUX_SDK_INPUT_STEPS):
            blocked.append(job["name"].split(" / ", 1)[0])  # not green, and the Linux SDK leg's item says why
        else:
            unfinished.append((job["name"], job["conclusion"], steps, "failed at the steps named"))
    for job in jobs:
        if not known(job["name"], all_legs) and not never_started(job):
            unfinished.append((job["name"], job["conclusion"], [], "did not report: a job the summary does not know"))
    names = {job["name"] for job in jobs}
    if "shards" in all_legs and "shards" not in helpers and "shards" not in unstarted:
        for expected in expected_shards():
            if expected not in names:
                unfinished.append((expected, "missing", [], "did not report: the daily matrix has this shard, but "
                                   "no job of that name ran"))
    seen = {job["name"].split(" / ", 1)[0] for job in leg_jobs(jobs, all_legs)}
    for leg in legs:
        result = results.get(leg, {}).get("result", "missing")
        accounted = any(item["leg"].split(" / ", 1)[0] == leg for item in tests) or any(
            entry[0].split(" / ", 1)[0] == leg for entry in unfinished) or leg in blocked
        if leg in unstarted:
            continue
        if leg == PARITY:
            parity_jobs = [job for job in jobs if job["name"].startswith(f"{PARITY} / ")]
            failures = parity_report["failed"] if parity_report else []
            tests.extend({"package": PARITY, "test": name, "leg": PARITY, "kind": PARITY,
                          "panic": None, "policy": None} for name in failures)
            # A completed harness failure is not an infrastructure failure. The two can also coexist: a share
            # can write a complete verdict, fail while saving its private logs, and still upload, while another
            # share reports a red harness.
            inside = [f"{job['name']}: {step}" for job in parity_jobs
                      for step in failed_steps(job) if step in PARITY_RESULT_STEPS]
            outside = [f"{job['name']}: {step}" for job in parity_jobs
                       for step in failed_steps(job) if step not in PARITY_RESULT_STEPS]
            if not parity_jobs or not parity_report:
                reason = "no verdict from this run attempt"
            elif not parity_report["complete"]:
                reason = "the harness run did not finish"
            elif result != ("failure" if failures else "success"):
                reason = "the parity call did not finish with the reported harness result"
            elif outside:
                reason = "failed outside its harness results"
            else:
                passed |= {(PARITY, name) for name in parity_report["passed"]}
                green_legs.add(leg)  # the harness run completed; close any infrastructure issue
                continue
            unfinished.append((leg, result, outside or inside, reason))
        elif leg in helpers:
            unfinished.append((leg, result, [], "did not complete: " + ", ".join(f"`{name}`" for name in helpers[leg])
                               + " failed, so its jobs did not run"))
        elif leg not in seen:
            unfinished.append((leg, result, [], "did not report: no job of this leg ran, or its jobs are named differently"))
        elif result != "success" and not accounted:
            unfinished.append((leg, result, [], "did not complete, and no job or test of it says why"))
        elif result == "success" and not accounted:
            green_legs.add(leg)  # a whole leg that reported and passed closes its leg-level issue
    if blocked:
        note = (" and ".join(f"`{leg}`" for leg in blocked) + " could not run: "
                + ("it needs" if len(blocked) == 1 else "they need") + " this leg's SDK")
        for item in tests:
            if item["leg"] in linux_sdk_red:
                item["blocks"] = note
        unfinished = [(leg, conclusion, steps, f"{reason}. {note}" if leg in linux_sdk_red else reason)
                      for leg, conclusion, steps, reason in unfinished]
    green = legs_green(legs, jobs, results, tests, unfinished) and not tests and not unfinished
    failing = {(item["package"], item["test"]) for item in tests}
    return tests, unfinished, passed - failing, green_legs, green


def legs_green(legs, jobs, results, tests, unfinished):
    """Whether every one of `legs` ran and passed: its call succeeded, each of its jobs is green, and no failing test
    or unfinished item is its own."""
    fresh_ok = any(job["name"] == FRESH and job["conclusion"] == "success" for job in jobs)
    mine = leg_jobs(jobs, legs)
    return (all(results.get(leg, {}).get("result") == "success" for leg in legs)
            and {job["name"].split(" / ", 1)[0] for job in mine} >= set(legs)
            and all(job_green(job, fresh_ok) for job in mine)
            and not any(item["leg"].split(" / ", 1)[0] in legs for item in tests)
            and not any(entry[0].split(" / ", 1)[0] in legs for entry in unfinished))


def suspects(commit, git):
    """The pull requests merged since the last green daily (`daily-green..commit`), or None with no green daily yet."""
    previous_green = git("rev-parse", "--verify", "--quiet", f"refs/tags/{TAG}^{{commit}}")
    if previous_green is None:
        return None
    log = git("log", "--merges", "--format=%s", f"{previous_green}..{commit}") or ""
    return sorted({int(found[1] or found[2]) for line in log.splitlines() if (found := MERGE.match(line))})


def fixes_after(commit, titles, call, query, git):
    """title -> (number, fix) for each red title with no open issue whose newest closed issue was closed by a pull
    request or commit `fix` that `commit` does not contain: the fix merged after the commit this run tested."""
    if not titles:
        return {}
    since = (dt.datetime.now(dt.timezone.utc) - FIX_LOOKBACK).date().isoformat()
    newest = {}
    for issue in pages(call, f"issues?state=closed&labels={LABEL}&since={since}"):
        if "pull_request" not in issue and issue["title"] in titles:
            newest[issue["title"]] = max(newest.get(issue["title"], 0), issue["number"])
    owner, name = os.environ["GITHUB_REPOSITORY"].split("/")
    fixes = {}
    for title, number in sorted(newest.items()):
        nodes = query(CLOSER, {"owner": owner, "name": name, "number": number})["repository"]["issue"][
            "timelineItems"]["nodes"]
        closer = (nodes[-1].get("closer") if nodes else None) or {}
        fix = (closer.get("mergeCommit") or {}).get("oid") or closer.get("oid")
        # A commit missing from the checkout is not in `commit` either: the checkout holds all of its history.
        if fix and git("merge-base", "--is-ancestor", fix, commit) is None:
            fixes[title] = (number, fix)
    return fixes


def previous_run(commit, call, git):
    """(commit, URL, {job name: conclusion}) of the newest earlier daily run whose summary ran, on an ancestor of
    `commit` other than it (dry runs too: they run every test), or None."""
    current = int(os.environ["GITHUB_RUN_ID"])
    since = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=14)).date().isoformat()
    for run in pages(call, f"actions/workflows/daily.yml/runs?status=completed&created=%3E%3D{since}", "workflow_runs"):
        if run["id"] == current or run["conclusion"] not in ("success", "failure"):
            continue
        jobs = pages(call, f"actions/runs/{run['id']}/jobs?filter=latest", "jobs")
        # A break-glass run's summary ran only some legs: it says nothing of the others.
        tested = [job["name"].split()[1] for job in jobs if job["name"].startswith("summary ")
                  and job["conclusion"] in ("success", "failure") and len(job["name"].split()) > 1
                  and "(break-glass legs)" not in job["name"]]
        if tested and tested[0] != commit and git("merge-base", "--is-ancestor", tested[0], commit) is not None:
            return tested[0], run["html_url"], {job["name"]: job["conclusion"] for job in jobs}
    return None


def package_folders(package):
    """The source folders a change to `package` lands in: its own, and a folder whose sources it also builds
    (sunrise-client-lib builds crates/sunrise-client's: ignored_shards.SHARED_SOURCES)."""
    members = ignored_shards.workspace_members()
    folders = {members[package][0]} if package in members else set()
    folders |= {members[crate][0] for crate, packages in ignored_shards.SHARED_SOURCES.items()
                if package in packages and crate in members}
    return sorted(folders)


def pull_requests_touching(base, commit, folders, git):
    """The pull requests that brought a commit changing `folders` into `commit` since `base`: for each such commit, the
    oldest merge naming a pull request on its way into `commit` (its own merge, or a rollup's member merge)."""
    numbers = set()
    for change in (git("log", "--no-merges", "--format=%H", f"{base}..{commit}", "--", *folders) or "").split():
        merges = (git("log", "--merges", "--ancestry-path", "--format=%s", f"{change}..{commit}") or "").splitlines()
        named = [found for line in merges if (found := MERGE.match(line))]
        if named:
            numbers.add(int(named[-1][1] or named[-1][2]))
    return sorted(numbers)


def test_suspect_line(item, previous, commit, git):
    """A failing test's suspects: the pull requests since the previous daily run that changed its package's folders,
    saying whether that run is known to have passed it (its leg's job succeeded there), or None with no such run."""
    if previous is None or not (folders := package_folders(item["package"])):
        return None
    base, url, legs = previous
    numbers = pull_requests_touching(base, commit, folders, git)
    where = ", ".join(f"`{folder}`" for folder in folders)
    listed = ", ".join(f"#{n}" for n in numbers) if numbers else "none identified in this interval"
    if legs.get(item["leg"]) == "success":
        return (f"Suspects: it passed in the previous daily run ({url}, `{base[:9]}`). Pull requests merged since then "
                f"that changed {where}: {listed}.")
    seen = legs.get(item["leg"], "no such job")
    return (f"Suspects: the previous daily run ({url}, `{base[:9]}`) does not show this test passing (its job "
            f"`{item['leg']}` ended `{seen}`), so earlier merges are suspects too. Pull requests merged since then "
            f"that changed {where}: {listed}.")


def suspect_line(numbers, commit):
    if numbers is None:
        return f"Suspects: no daily run has been green yet, so every merge before `{commit[:9]}` is a suspect."
    if not numbers:
        return f"Suspects: no pull request merged between `{TAG}` and `{commit[:9]}` (a direct push or an older break)."
    return f"Suspects, merged since the last green daily (`{TAG}`..`{commit[:9]}`): " + ", ".join(f"#{n}" for n in numbers)


def test_title(item, prefix=PREFIX):
    return f"{prefix}{item['package']} {item['test']}"


def leg_title(leg, prefix=PREFIX):
    action = ACTIONS.get(leg.split(" / ", 1)[0])
    return f"{prefix}{action[0]}" if action else f"{prefix}{leg}{UNFINISHED}"


def detail(item):
    where = f"`{item['panic']}`" if item["panic"] else "not reported"
    kinds = {
        "failed": f"fails in the daily run. Panic location: {where}.",
        "gpu": f"fails with a Vulkan driver present. Panic location: {where}.",
        "passes-now": f"passes now but is listed as a known failure in `{item['policy']}`: take it off that list.",
        "no-longer-runs": f"is listed in `{item['policy']}` but no longer runs: remove the row or restore the test.",
        "parity": "failed in the daily run. Its logs hold vanilla data, so they stay on the runner.",
        SDK_CHECK: f"fails at the step `{item.get('step')}`, so this SDK is not published.",
    }
    blocks = f" {item['blocks']}." if item.get("blocks") else ""
    return f"`{item['test']}` ({item['package']}, leg `{item['leg']}`) {kinds[item['kind']]}{blocks}"


def leg_detail(leg, conclusion, steps, reason):
    named_steps = ", ".join(f"`{step}`" for step in steps) or "none recorded"
    action = ACTIONS.get(leg.split(" / ", 1)[0])
    return ((f"{action[1]}\n\n" if action else "")
            + f"Leg `{leg}` ended `{conclusion}`: {reason}. Failed steps: {named_steps}.")


def plan(commit, run_url, tests, unfinished, passed, green_legs, green, open_issues, numbers, fixes=None, lines=None,
         prefix=PREFIX):
    """Every write this run makes, in order: ("tag", sha) / ("create", title, body) / ("comment", number, title, body)
    / ("close", number, title, body). `fixes` (fixes_after) holds the closed issues whose fix this commit predates;
    `lines` a failing test's own suspects line by title (test_suspect_line), else every red gets `numbers`' line.
    `prefix` titles the issues (WRITING_REFS); only main's (PREFIX) green moves the tag."""
    fixes = fixes or {}
    lines = lines or {}
    actions = []
    if green and prefix == PREFIX:
        actions.append(("tag", commit))
    reds = [(test_title(item, prefix), detail(item)) for item in tests]
    reds += [(leg_title(leg, prefix), leg_detail(leg, conclusion, steps, reason))
             for leg, conclusion, steps, reason in unfinished]
    filed = set()
    for title, text in reds:
        if title in filed:
            continue
        filed.add(title)
        line = lines.get(title) or suspect_line(numbers, commit)
        if title in open_issues:
            actions.append(("comment", open_issues[title], title,
                            f"🤖 Agent: {red_marks(run_url)[1]} on `{commit[:9]}`.\n\n{text}\n\n{line}\n"))
        elif title in fixes:
            number, fix = fixes[title]
            actions.append(("comment", number, title,
                            f"🤖 Agent: {red_marks(run_url)[1]} on `{commit[:9]}`, which predates the fix `{fix[:9]}`"
                            f" that closed this issue; it stays closed.\n\n{text}\n"))
        else:
            actions.append(("create", title,
                            f"{DISCLOSURE}\n\n{text}\n\n{red_marks(run_url)[0]} on `{commit[:9]}`.\n\n"
                            f"{line}\n\nThis issue gets a comment from each later daily run that still sees it and is "
                            "closed by the first that does not.\n"))
    for title, number in sorted(open_issues.items()):
        if title in filed:
            continue
        rest = title[len(prefix):]
        acted = [leg for leg, (action, _) in ACTIONS.items() if action == rest]
        if acted:
            back = acted[0] in green_legs or any(name.startswith(f"{acted[0]} / ") for name in green_legs)
        elif rest.endswith(UNFINISHED):
            back = rest[:-len(UNFINISHED)] in green_legs
        else:
            package, _, test = rest.partition(" ")
            back = (package, test) in passed
        if back:
            actions.append(("close", number, title,
                            f"🤖 Agent: {green_mark(run_url)} on `{commit[:9]}`; closing.\n"))
    return actions


def render(actions, writes):
    lead = "## Daily summary" + ("" if writes else " (dry run: nothing written)")
    lines = [lead, ""]
    if any(action[0] == "tag" for action in actions):
        lines.append("Every applicable leg is green: then publish the nightly of this commit (release.yml, kind nightly).")
    if not actions:
        lines.append("Nothing to do: no leg is red and no open daily issue went green.")
    for action in actions:
        verb = {"tag": "Move", "create": "File", "comment": "Comment on", "close": "Close"}[action[0]]
        if action[0] == "tag":
            lines.append(f"- {verb} `{TAG}` to `{action[1]}`")
        elif action[0] == "create":
            lines += [f"- {verb} **{action[1]}**", "", "  " + action[2].strip().replace("\n", "\n  "), ""]
        else:
            lines += [f"- {verb} #{action[1]} **{action[2]}**", "", "  " + action[3].strip().replace("\n", "\n  "), ""]
    return "\n".join(lines) + "\n"


def red_marks(run_url):
    """What a red report of this run says on an issue: its opening body, or its comment."""
    return (f"First daily run that saw it: {run_url}", f"Still red in the daily run {run_url}")


def green_mark(run_url):
    return f"Green again in the daily run {run_url}"


def already_said(number, marks, call):
    """Whether issue `number` already carries one of `marks` (its body, or one of its comments): one comment per
    transition and run, so a rerun that turns a test green still posts its close comment."""
    texts = [call("GET", f"issues/{number}")["body"] or ""]
    texts += [comment["body"] or "" for comment in pages(call, f"issues/{number}/comments")]
    return any(mark in text for mark in marks for text in texts)


def apply(actions, call, run_url):
    for action in actions:
        if action[0] == "tag":
            # Force-update the one tag; create it the first time.
            if any(ref["ref"] == f"refs/tags/{TAG}" for ref in call("GET", f"git/matching-refs/tags/{TAG}")):
                call("PATCH", f"git/refs/tags/{TAG}", {"sha": action[1], "force": True})
            else:
                call("POST", "git/refs", {"ref": f"refs/tags/{TAG}", "sha": action[1]})
        elif action[0] == "create":
            call("POST", "issues", {"title": action[1], "body": action[2], "labels": [LABEL]})
        elif action[0] == "comment":
            if not already_said(action[1], red_marks(run_url), call):
                call("POST", f"issues/{action[1]}/comments", {"body": action[3]})
        else:
            if not already_said(action[1], (green_mark(run_url),), call):
                call("POST", f"issues/{action[1]}/comments", {"body": action[3]})
            call("PATCH", f"issues/{action[1]}", {"state": "closed", "state_reason": "completed"})


def git(*args):
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def main(call=api, run_git=git, query=graphql):
    env = os.environ
    commit = env["COMMIT"]
    needs = json.loads(env["RESULTS"])
    selected = needs.get("resolve", {}).get("outputs", {}).get("legs")
    # Only main and the release branch write, whatever the input says: another branch's run is always a dry run. A
    # break-glass run (daily.yml `break_glass`, its legs selected) files and closes no issue either.
    prefix = WRITING_REFS.get(env.get("GITHUB_REF"), PREFIX)
    writes = env.get("GITHUB_REF") in WRITING_REFS and env.get("DRY_RUN") == "false" and not selected
    run_url = f"{env.get('GITHUB_SERVER_URL', 'https://github.com')}/{env['GITHUB_REPOSITORY']}/actions/runs/{env['GITHUB_RUN_ID']}"
    reports_dir = Path(env["REPORTS"])
    reports = [json.loads(path.read_text()) for path in sorted(reports_dir.glob("*/*.json")) if path.name != "parity.json"]
    attempt = env["GITHUB_RUN_ATTEMPT"]
    path = reports_dir / f"daily-report-parity-{attempt}" / "parity.json"
    parity_report = json.loads(path.read_text()) if path.is_file() else None
    if parity_report and parity_report.get("scope") != {
            "run_id": env["GITHUB_RUN_ID"], "run_attempt": attempt, "commit": commit}:
        parity_report = None
    jobs = pages(call, f"actions/runs/{env['GITHUB_RUN_ID']}/jobs?filter=latest", "jobs")
    all_legs, results = daily_legs(needs), leg_results(needs)
    # Only a run that writes nothing may leave parity out: a writing run's skipped parity holds green and the tag.
    parity_not_applicable = (not writes and results.get("resolve", {}).get("outputs", {}).get("parity_required") == "false"
                             and results.get(PARITY, {}).get("result") == "skipped")
    legs = tuple(leg for leg in all_legs if leg != PARITY or not parity_not_applicable)
    if selected:
        # A break-glass run (daily.yml `break_glass`) ran only these legs: the others, and their skipped jobs, are no
        # part of its verdict.
        chosen = set(json.loads(selected))
        all_legs = legs = tuple(leg for leg in legs if leg in chosen)
        jobs = [job for job in jobs if job["name"].partition(" / ")[0] not in set(daily_legs(needs)) - chosen]
    tests, unfinished, passed, green_legs, green = verdicts(jobs, reports, results, legs, parity_report, all_legs)
    open_issues = {issue["title"]: issue["number"]
                   for issue in pages(call, f"issues?state=open&labels={LABEL}")
                   if "pull_request" not in issue and issue["title"].startswith(prefix)}
    # Resolve the prior green before planning or applying any writes, including the tag move.
    numbers = suspects(commit, run_git) if tests or unfinished else []
    reds = {test_title(item, prefix) for item in tests} | {leg_title(leg, prefix) for leg, *_ in unfinished}
    fixes = fixes_after(commit, reds - open_issues.keys(), call, query, run_git)
    other_tests = [item for item in tests if item["kind"] not in (PARITY, SDK_CHECK)]
    previous = previous_run(commit, call, run_git) if other_tests else None
    lines = {test_title(item, prefix): line for item in other_tests
             if (line := test_suspect_line(item, previous, commit, run_git))}
    actions = plan(commit, run_url, tests, unfinished, passed, green_legs, green, open_issues, numbers, fixes, lines,
                   prefix)
    sdk_green = legs_green(SDK_LEGS, jobs, results, tests, unfinished)
    text = render(actions, writes) + (
        f"\nSDK legs ({', '.join(SDK_LEGS)}): {'green' if sdk_green else 'not green'}; "
        f"sdk-publish {'publishes' if sdk_green else 'does not publish'} sdk-<sha> on a writing run.\n")
    print(text)
    if env.get("GITHUB_STEP_SUMMARY"):
        with open(env["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(text)
    if env.get("GITHUB_OUTPUT"):
        with open(env["GITHUB_OUTPUT"], "a") as out:
            out.write(f"green={str(green).lower()}\n")
            out.write(f"sdk_green={str(sdk_green).lower()}\n")
    if writes:
        apply(actions, call, run_url)
    return 0


def after(name, result, call=api, run_git=git, query=graphql):
    """Reports the job `name` that ran after the summary, which concluded `result` (`needs.<name>.result`): one
    issue for it while it fails, closed by its next success. Writes only where the summary would."""
    env = os.environ
    prefix = WRITING_REFS.get(env.get("GITHUB_REF"), PREFIX)
    writes = env.get("GITHUB_REF") in WRITING_REFS and env.get("DRY_RUN") == "false" and env.get("PARTIAL") != "true"
    run_url = f"{env.get('GITHUB_SERVER_URL', 'https://github.com')}/{env['GITHUB_REPOSITORY']}/actions/runs/{env['GITHUB_RUN_ID']}"
    commit = env["COMMIT"]
    open_issues = {issue["title"]: issue["number"]
                   for issue in pages(call, f"issues?state=open&labels={LABEL}")
                   if "pull_request" not in issue and issue["title"] == leg_title(name, prefix)}
    if result == "success":
        actions = plan(commit, run_url, [], [], set(), {name}, False, open_issues, [], prefix=prefix)
    else:
        jobs = pages(call, f"actions/runs/{env['GITHUB_RUN_ID']}/jobs?filter=latest", "jobs")
        steps = [f"{job['name']}: {step}" for job in jobs if job["name"].startswith(f"{name} / ")
                 for step in failed_steps(job)]
        unfinished = [(name, result, steps, "nothing was published, so no Nightly follows")]
        fixes = fixes_after(commit, {leg_title(name, prefix)} - open_issues.keys(), call, query, run_git)
        actions = plan(commit, run_url, [], unfinished, set(), set(), False, open_issues,
                       suspects(commit, run_git), fixes, prefix=prefix)
    text = render(actions, writes)
    print(text)
    if env.get("GITHUB_STEP_SUMMARY"):
        with open(env["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(text)
    if writes:
        apply(actions, call, run_url)
    return 0


def seen(sha, call=api):
    """The URL of a completed real daily run of sha (its summary job, named `summary <sha>`, finished), or ""."""
    current = int(os.environ["GITHUB_RUN_ID"])
    # The schedule asks about main's head, so a run of it is recent: look back two weeks, not at all history.
    since = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=14)).date().isoformat()
    for run in pages(call, f"actions/workflows/daily.yml/runs?status=completed&created=%3E%3D{since}", "workflow_runs"):
        if run["id"] == current or run["conclusion"] not in ("success", "failure"):
            continue
        if any(job["name"] == f"summary {sha}" and job["conclusion"] in ("success", "failure")
               for job in pages(call, f"actions/runs/{run['id']}/jobs?filter=latest", "jobs")):
            return run["html_url"]
    return ""


if __name__ == "__main__":
    if sys.argv[1:2] == ["--seen"]:
        print(seen(sys.argv[2]))
        sys.exit(0)
    if sys.argv[1:2] == ["--after"]:
        sys.exit(after(sys.argv[2], sys.argv[3]))
    sys.exit(main())
