"""Whether a main push may reuse a gate that already passed on its exact tree: `reused=true|false` and `run=<id>`.

Every push and pull_request gate's lint job records the tree it checked out as an artifact named `gate-tree-<tree>`:
a pull_request run's tree is GitHub's test merge, a push's the commit's, and either way the dyna-ci.yml that ran is
that tree's own. A `pr` dispatch runs main's definition on another tree, so it records nothing and is never reused. A rollup merged into main
usually leaves main with the very tree its pull_request run built, linted and tested, and main's push run then ran
that whole gate again: two large runners for about 25 minutes and a medium one for 6, at the moment the next
rollup's gate wants them. When this repository's dyna-checks run on the same tree passed its gate (lint and verify
with their Rust steps run), the push takes its verdict: lint, verify and verify-static skip the Rust and tooling work
on the small runner, as for a change selection finds no Rust or tooling in.

Anything else (no such run, a dispatched run, a run from a fork or another workflow, an artifact without its run,
an API error or timeout) reuses nothing and the push runs the whole gate.
"""

import json
import os
import subprocess
import sys

WORKFLOW = ".github/workflows/dyna-ci.yml"
GATE = ("lint", "verify", "verify-static")
# The events whose run uses the checked tree's own workflow file.
OWN_DEFINITION = ("push", "pull_request")
# The job, and the step in it that shows the job did the Rust work rather than the documentation-only path.
RUST_STEPS = {"lint": "Lint native targets and server build", "verify": "Test Rust core and voxel kernel correctness"}


def api(path: str) -> dict:
    return json.loads(subprocess.run(["gh", "api", path], check=True, capture_output=True, text=True, timeout=30).stdout)


def gate_passed(jobs: list[dict]) -> bool:
    named = {job["name"]: job for job in jobs}
    return all(name in named and named[name]["conclusion"] == "success" for name in GATE) and all(
        any(step["name"] == want and step["conclusion"] == "success" for step in named[name]["steps"])
        for name, want in RUST_STEPS.items())


def reuse(repository: str, sha: str, read=api) -> tuple[int | None, str]:
    """The run whose verdict the push at sha may take, and why (or why not)."""
    tree = read(f"repos/{repository}/commits/{sha}")["commit"]["tree"]["sha"]
    artifacts = read(f"repos/{repository}/actions/artifacts?name=gate-tree-{tree}&per_page=30")["artifacts"]
    for artifact in sorted(artifacts, key=lambda a: a["created_at"], reverse=True):
        run_id = (artifact.get("workflow_run") or {}).get("id")
        if not run_id:
            continue
        run = read(f"repos/{repository}/actions/runs/{run_id}")
        # Only this repository's own branches run our workflow file as written, and only a push or pull_request runs
        # the checked tree's own definition; a fork's run or a dispatch proves nothing.
        if (run["path"] != WORKFLOW or run["event"] not in OWN_DEFINITION
                or (run.get("head_repository") or {}).get("full_name") != repository):
            continue
        if gate_passed(read(f"repos/{repository}/actions/runs/{run_id}/jobs?per_page=100")["jobs"]):
            return run_id, f"tree {tree[:10]} of {sha[:10]} passed the gate in run {run_id} ({run['event']})"
    return None, f"no passed gate of this repository's dyna-checks on tree {tree[:10]} of {sha[:10]}"


def main() -> None:
    # The one boundary where anything may go wrong (the API, its answers' shape, a timeout): it only costs the saving.
    try:
        run, why = reuse(os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_SHA"])
    except Exception as error:  # noqa: BLE001
        run, why = None, f"lookup failed, running the whole gate: {getattr(error, 'stderr', None) or error!r}"
    print(f"reused={'true' if run else 'false'}")
    print(f"run={run or ''}")
    print(f"gate reuse: {why}", file=sys.stderr)
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
        summary.write(f"Gate reuse: {why}.\n")


if __name__ == "__main__":
    main()
