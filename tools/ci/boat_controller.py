#!/usr/bin/env python3
"""Trusted API-only recovery of fork PR boat startup outages.

This script comes exclusively from the controller's default-branch workflow SHA.
It neither downloads nor executes code from the triggering PR.
"""
from datetime import datetime, timedelta, timezone
from urllib.parse import urlencode
import io
import json
import re
import zipfile
import os
import subprocess
import time

from boat_health import api, genuine_failure, recovery_reason


def eligible(run, repo, workflow):
    return (run["workflow_id"] == workflow and run["event"] == "pull_request"
            and run["run_attempt"] == 1
            and run.get("conclusion") not in ("success", "cancelled", "skipped")
            and run.get("head_repository") is not None
            and run["head_repository"]["full_name"] != repo)


def log(message):
    print(message, flush=True)
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
        summary.write(message + "\n\n")


def post(path):
    subprocess.run(["gh", "api", "--method", "POST", path], check=True)


def recover(repo, run):
    path = f"repos/{repo}/actions/runs/{run['id']}"
    jobs = api(path + "/attempts/1/jobs?per_page=100")["jobs"]
    reason = recovery_reason(jobs, datetime.now(timezone.utc))
    if not reason:
        return
    log(f"Run {run['id']}: {reason}.")
    if run["status"] != "completed":
        log(f"Cancelling run {run['id']} to release its stranded boat jobs.")
        post(path + "/cancel")
        for _ in range(20):
            if api(path)["status"] == "completed":
                break
            time.sleep(3)
        else:
            raise RuntimeError("Cancellation has not completed; refusing an overlapping rerun")
    # The preceding attempt is now immutable. Recheck before rerunning so a real
    # failure arriving between inspection and cancellation is never retried.
    jobs = api(path + "/attempts/1/jobs?per_page=100")["jobs"]
    if genuine_failure(jobs):
        log(f"Run {run['id']}: a genuine failure appeared; no rerun requested.")
        return
    log(f"Rerunning run {run['id']}; changes reads the previous attempt's outage and forces ubuntu-latest.")
    post(path + "/rerun")


def recorded_base(repo, run):
    """Read a bounded metadata artifact, never extract or execute PR content."""
    artifacts = api(f"repos/{repo}/actions/runs/{run['id']}/artifacts?per_page=100")["artifacts"]
    matches = [item for item in artifacts if item["name"] == "boat-base-1" and not item["expired"]]
    if len(matches) != 1 or matches[0]["size_in_bytes"] > 65536:
        return ""
    archive = subprocess.run(["gh", "api", f"repos/{repo}/actions/artifacts/{matches[0]['id']}/zip"],
                             check=True, capture_output=True).stdout
    try:
        with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
            entries = zipped.infolist()
            if len(entries) != 1 or entries[0].filename != "boat-base.json" or entries[0].file_size > 4096:
                return ""
            metadata = json.loads(zipped.read(entries[0]))
    except Exception:
        # PR-controlled codecs can raise zlib, bz2, lzma and JSON parser errors.
        # No malformed metadata is allowed to stop inspection of other runs.
        return ""
    if (not isinstance(metadata, dict) or metadata.get("run_id") != str(run["id"])
            or metadata.get("attempt") != "1" or metadata.get("head") != run["head_sha"]):
        return ""
    base = metadata.get("base")
    return base if isinstance(base, str) and re.fullmatch(r"[0-9a-f]{40}", base) else ""


def unsupported_reason(repo, run, trusted_ref):
    """Require the original merge's trusted base to contain our exact routing gate.

    A rerun preserves its original workflow revision. Never assume a new controller
    can retrofit old runs, or execute PR content to find out whether it can.
    """
    pulls = run.get("pull_requests", [])
    if len(pulls) == 1:
        base = pulls[0]["base"]["sha"]
    elif not pulls:
        base = recorded_base(repo, run)
        if not base:
            return "original PR base metadata is unavailable or invalid"
    else:
        return "original PR base SHA is ambiguous"
    paths = ("tools/ci/boat_health.py", ".github/workflows/dyna-ci.yml")
    for path in paths:
        try:
            original = api(f"repos/{repo}/contents/{path}?ref={base}")
        except subprocess.CalledProcessError:
            return "original fallback gate is missing or unreadable"
        trusted = api(f"repos/{repo}/contents/{path}?ref={trusted_ref}")
        if original["sha"] != trusted["sha"]:
            return "original base does not contain this controller's exact fallback gate"
    comparison = api(f"repos/{repo}/compare/{base}...{run['head_sha']}")
    # Artifact metadata is PR-controlled, not an authenticated base assertion.
    # Require independent ancestry: the claimed trusted gate must already be in
    # the run head. Otherwise a three-dot diff can hide old routing files at head.
    if comparison["merge_base_commit"]["sha"] != base:
        return "recorded base is not an ancestor of the original run head"
    files = comparison["files"]
    # GitHub truncates compare file lists at 300. An incomplete diff cannot prove
    # the PR left the trusted routing code alone.
    if len(files) >= 300:
        return "PR changed-file list may be truncated"
    if any(file["filename"] in paths or file.get("previous_filename") in paths for file in files):
        return "PR modifies fallback routing code"
    return ""


def candidates(repo, workflow):
    """Active runs cannot be displaced by a stream of newer completed runs."""
    for status in ("queued", "in_progress", "failure"):
        page = 1
        while True:
            query = dict(event="pull_request", status=status, per_page=100, page=page)
            if status == "failure":
                query["created"] = ">=" + (datetime.now(timezone.utc) - timedelta(days=1)).isoformat()
            runs = api(f"repos/{repo}/actions/workflows/{workflow}/runs?{urlencode(query)}")["workflow_runs"]
            for run in runs:
                if status == "failure" and datetime.fromisoformat(run["updated_at"].replace("Z", "+00:00")) < datetime.now(timezone.utc) - timedelta(hours=1):
                    continue
                yield run
            if len(runs) < 100:
                break
            page += 1


def consider(repo, run, workflow, trusted_ref):
    if not eligible(run, repo, workflow):
        return
    jobs = api(f"repos/{repo}/actions/runs/{run['id']}/attempts/1/jobs?per_page=100")["jobs"]
    if not recovery_reason(jobs, datetime.now(timezone.utc)):
        return
    unsupported = unsupported_reason(repo, run, trusted_ref)
    if unsupported:
        log(f"Run {run['id']}: skipped recovery: {unsupported}.")
        return
    # Read again after support inspection: run state may have changed meanwhile.
    run = api(f"repos/{repo}/actions/runs/{run['id']}")
    if eligible(run, repo, workflow):
        recover(repo, run)


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    workflow = api(f"repos/{repo}/actions/workflows/dyna-ci.yml")["id"]
    with open(os.environ["GITHUB_EVENT_PATH"]) as event_file:
        event = json.load(event_file)
    if os.environ["GITHUB_EVENT_NAME"] == "workflow_run":
        run_id = event["workflow_run"]["id"]
        run = api(f"repos/{repo}/actions/runs/{run_id}")
        consider(repo, run, workflow, os.environ["GITHUB_WORKFLOW_SHA"])
    else:
        for run in candidates(repo, workflow):
            try:
                consider(repo, run, workflow, os.environ["GITHUB_WORKFLOW_SHA"])
            except Exception as failure:
                # A candidate contains untrusted data; its failure cannot starve
                # every later candidate, regardless of the parser/API exception.
                log(f"Run {run['id']}: recovery inspection failed ({type(failure).__name__}); continuing sweep.")


if __name__ == "__main__":
    main()
