#!/usr/bin/env python3
"""Start the day's real daily run from a push to main when GitHub's schedule did not
(.github/workflows/daily-catch-up.yml).

GitHub has delivered almost none of this repository's `schedule` events: daily.yml had no scheduled run from the day
it landed (2026-11-14) to 2026-11-16, ci.yml's every-six-hours one none since 2026-11-07. So each push to main asks:
is it past the daily's hour (09:00 UTC) with no real daily run started since then, other than a cancelled one? Then
it dispatches daily.yml on main as the schedule would (`dry_run=false`, `scheduled=true`), and waits until the run is
listed, so the next push's check sees it. A day without a push needs no run: a scheduled daily stops on a commit that
already has one. A run cancelled since 09:00 does not count, so the push after a main-break fix starts the daily again.

Environment: GITHUB_REPOSITORY, GH_TOKEN. Any API error fails the job: a failed lookup never dispatches.
"""

import datetime as dt
import json
import os
import sys
import time
import urllib.parse
import urllib.request

WORKFLOW = "daily.yml"
HOUR = 9  # daily.yml's cron, '0 9 * * *'


def api(path, body=None):
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
        data=None if body is None else json.dumps(body).encode(), method="GET" if body is None else "POST",
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        text = response.read()
    return json.loads(text) if text else None


def slot(now):
    """The start of the newest daily slot at or before now, or None before today's."""
    start = now.replace(hour=HOUR, minute=0, second=0, microsecond=0)
    return start if now >= start else None


def real_runs(runs):
    """The real (writing) daily runs among runs: dispatched on main, not a dry run (its title says so), not cancelled.
    A `schedule` run only dispatches the real one (daily.yml's schedule-dispatch), so it never counts itself."""
    return [run for run in runs if run["event"] == "workflow_dispatch" and run["head_branch"] == "main"
            and not run["display_title"].endswith("(dry run)") and run.get("conclusion") != "cancelled"]


def runs_since(since):
    query = urllib.parse.urlencode({"created": f">={since:%Y-%m-%dT%H:%M:%SZ}", "per_page": 100})
    return api(f"actions/workflows/{WORKFLOW}/runs?{query}")["workflow_runs"]


def main(now=None):
    now = now or dt.datetime.now(dt.timezone.utc)
    since = slot(now)
    if since is None:
        print(f"before {HOUR:02}:00 UTC: today's daily is not due")
        return 0
    started = real_runs(runs_since(since))
    if started:
        print(f"today's daily started: {started[0]['html_url']}")
        return 0
    api(f"actions/workflows/{WORKFLOW}/dispatches",
        {"ref": "main", "inputs": {"dry_run": "false", "scheduled": "true"}})
    for _ in range(12):
        time.sleep(5)
        started = real_runs(runs_since(since))
        if started:
            print(f"no daily since {since:%H:%M} UTC: started {started[0]['html_url']}")
            return 0
    print(f"::error::dispatched {WORKFLOW}, but no run of it was listed within a minute")
    return 1


if __name__ == "__main__":
    sys.exit(main())
