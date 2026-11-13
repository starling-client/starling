#!/usr/bin/env python3
"""Ends a run stranded behind a dead self-hosted pool.

Runs on a GitHub-hosted runner beside jobs bound for the pool. GitHub lets such a
job wait 24 hours for a runner, so a pool that dies after a run was routed to it
would hold that run's checks for a day.

Waiting is not enough to call a job stuck, because a run queued behind a busy pool
waits too. So once this run has work queued and no progress for STALL_MINUTES, it
looks at the pool's jobs across the repository, and cancels only when twice in a row
none of them is running or has just finished. That is a pool that is down, or one
that cannot start a runner. The run ends red, and a rerun can use GitHub-hosted
runners.
"""
from datetime import datetime
import json
import os
import subprocess
import sys
import time

LABEL = "dyna-"
# Every queued run has a watcher, and they share GITHUB_TOKEN's 1,000 requests an hour.
POLL = 120
# A pool that lost its last job can be between jobs, so one quiet look proves nothing.
QUIET_RECHECK = 180
BUSY_RECHECK = 600
# Pool jobs start only after the hosted changes job.
FIRST_JOB_WAIT = 300


def seconds(stamp):
    """A GitHub timestamp as a Unix time, or 0 when it is empty."""
    return datetime.fromisoformat(stamp).timestamp() if stamp else 0


def on_pool(job, label):
    """Whether the job asks for a Dyna runner: any label that starts with the prefix (dyna-small, dyna-medium, ...)."""
    return any(name.startswith(label) for name in job.get("labels", []))


def queued_on_pool(jobs, label):
    return sum(job["status"] == "queued" and on_pool(job, label) for job in jobs)


def running_on_pool(jobs, label):
    return sum(job["status"] == "in_progress" and on_pool(job, label) for job in jobs)


def last_progress(jobs):
    """The newest time any job started or finished."""
    return max((seconds(job.get(key)) for job in jobs for key in ("started_at", "completed_at")), default=0)


def pool_worked(jobs, label, since):
    """Whether one of these jobs shows a runner serving the pool now or since then.

    A job that never left the queue has a start time and may have a finish time, so
    only a job that held a runner counts.
    """
    for job in jobs:
        if not on_pool(job, label):
            continue
        if job["status"] == "in_progress" or (job.get("runner_name") and seconds(job.get("completed_at")) >= since):
            return True
    return False


def pool_is_working(api, repo, label, since):
    """Whether the pool is serving any run. None means GitHub did not say."""
    runs = api(f"repos/{repo}/actions/runs?status=in_progress&per_page=100")
    if runs is None:
        return None
    # The oldest runs reached the pool first, so a busy pool usually answers at once.
    for run in sorted(runs["workflow_runs"], key=lambda run: run["id"]):
        jobs = api(f"repos/{repo}/actions/runs/{run['id']}/jobs?filter=latest&per_page=100")
        if jobs is None:
            return None
        if pool_worked(jobs["jobs"], label, since):
            return True
    return False


def watch(api, cancel, clock, sleep, repo, run, attempt, me, stall, label=LABEL):
    """Returns the exit code once the run needs no more watching."""
    url = f"repos/{repo}/actions/runs/{run}/attempts/{attempt}/jobs?per_page=100"
    started = clock()
    quiet = 0
    while True:
        answer = api(url)
        wait = POLL
        if answer is not None:
            jobs = [job for job in answer["jobs"] if job["name"] != me]
            queued = queued_on_pool(jobs, label)
            now = clock()
            if not queued and not running_on_pool(jobs, label):
                if now - started > FIRST_JOB_WAIT:
                    return 0
            elif queued and now - max(last_progress(jobs), started) >= stall:
                working = pool_is_working(api, repo, label, now - stall)
                if working is False and quiet >= 1:
                    print(f"::error::{queued} job(s) waited {stall // 60} minutes and no job on the self-hosted pool is running. "
                          "Cancelling this run. A rerun uses GitHub-hosted runners once the pool's lease expires.")
                    cancel()
                    return 1
                quiet = quiet + 1 if working is False else 0
                wait = QUIET_RECHECK if working is False else BUSY_RECHECK
            else:
                quiet = 0
        sleep(wait)


def gh(*args):
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def api(path):
    """A failed read is a reason to look again, never to cancel a run."""
    try:
        return json.loads(gh("api", path))
    except (subprocess.CalledProcessError, json.JSONDecodeError) as failure:
        print(f"::warning::{path}: {failure}")
        return None


def main():
    repo, run = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
    return watch(api, lambda: gh("run", "cancel", run), time.time, time.sleep, repo, run,
                 os.environ.get("GITHUB_RUN_ATTEMPT", "1"), os.environ["GITHUB_JOB"],
                 int(os.environ.get("STALL_MINUTES", "20")) * 60, os.environ.get("POOL_LABEL", LABEL))


if __name__ == "__main__":
    sys.exit(main())
