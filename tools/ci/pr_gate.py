"""Whether this run gets the heavy checks (everything that compiles), as `heavy=true|false` for $GITHUB_OUTPUT.

Ordinary pull requests get only the cheap checks (verify-static and format). The heavy checks run on main's
pushes, dispatches, and same-repository rollup or sync pull requests, identified by labels or branch names.
Fork pull requests remain cheap even when they use those labels or names.

A reviewer runs the heavy gate on one contributor's pull request with `gh workflow run dyna-ci.yml -f pr=<number>`:
main's workflow checks out that pull request's test merge with its base (what would merge, as a pull_request run
checks), runs the heavy jobs on GitHub's runners with a read-only token, and a job that runs no pull-request code
posts their results on the pull request's head commit. A rollup or sync pull request of
this repository is refused there: each push to it already runs the heavy gate as a pull_request run, and a dispatch
only ran the same gate a second time.
"""

import json
import os
import re
import subprocess
import sys


def integration(labels, head_ref: str) -> bool:
    """A rollup or sync pull request: its own pushes run the heavy gate.

    A rollup builder session pushes only branches named after itself (`rollup-76`, `sunrise-review-rollup-3`), and a
    label added after the pull request opens misses its first event, so the branch name alone counts too.
    """
    return bool({"rollup", "sync"}.intersection(labels)) or head_ref.startswith(
        ("fork-review-rollup-", "sunrise-review-rollup-", "rollup-", "sync-", "sync/upstream-"))


def outputs(event: str, same_repository: bool, pr: str, read_pr, *, labels: tuple[str, ...] | list[str] = (), head_ref: str = "") -> dict[str, str]:
    if event == "workflow_dispatch" and pr:
        if not re.fullmatch(r"[1-9][0-9]{0,6}", pr):
            raise SystemExit(f"pr must be a pull request number, not {pr!r}")
        head = read_pr(pr)
        if head["same_repository"] and integration(head["labels"], head["ref"]):
            raise SystemExit(f"::error::pull request #{pr} ({head['ref']}) is a rollup or sync: every push to it already "
                             "runs the heavy gate as its pull_request run, so a dispatch would only repeat it. Read that "
                             "run's checks on the pull request, or push again to run them anew.")
        if head["mergeable"] is None:
            raise SystemExit(f"::error::GitHub has not finished computing pull request #{pr}'s test merge with its base: "
                             "dispatch the gate again in a minute.")
        if not head["mergeable"]:
            raise SystemExit(f"::error::pull request #{pr} conflicts with its base, so there is no merge to test: merge "
                             "the base into it first.")
        return {"heavy": "true", "pr_sha": head["sha"], "pr_merge": head["merge"],
                "trusted": "true" if head["same_repository"] else "false"}
    # ci.yml's full checks run only for a fork's pull request (and scheduled runs on main): this repository's other
    # pull requests get dyna-checks' cheap checks, and these integration pull requests the heavy gate.
    heavy = event != "pull_request" or (same_repository and integration(labels, head_ref))
    return {"heavy": "true" if heavy else "false", "pr_sha": "", "pr_merge": "", "trusted": ""}


def read_pr(number: str) -> dict:
    """The pull request's head commit, its test merge with its base, its branch and labels, and whether its branch lives
    in this repository.

    GitHub computes the test merge in the background after the base or head moves: until then `mergeable` is null,
    and outputs() refuses the dispatch so it is sent again.
    """
    repository = os.environ["GITHUB_REPOSITORY"]
    answer = json.loads(subprocess.run(
        ["gh", "api", f"repos/{repository}/pulls/{number}",
         "--jq", "{sha: .head.sha, ref: .head.ref, repo: .head.repo.full_name, labels: [.labels[].name], "
                 "state: .state, mergeable: .mergeable, merge: .merge_commit_sha}"],
        check=True, capture_output=True, text=True).stdout)
    if answer["state"] != "open":
        raise SystemExit(f"pull request #{number} is {answer['state']}")
    if not re.fullmatch(r"[0-9a-f]{40}", answer["sha"]):
        raise SystemExit(f"pull request #{number} has no head commit")
    if answer["mergeable"] and not re.fullmatch(r"[0-9a-f]{40}", answer["merge"] or ""):
        raise SystemExit(f"pull request #{number} has no test merge commit")
    return {"sha": answer["sha"], "merge": answer["merge"] if answer["mergeable"] else "", "mergeable": answer["mergeable"],
            "ref": answer["ref"], "labels": answer["labels"], "same_repository": answer["repo"] == repository}


def main() -> None:
    values = outputs(os.environ.get("EVENT", ""),
                     os.environ.get("HEAD_REPO", "") == os.environ.get("GITHUB_REPOSITORY"),
                     os.environ.get("PR", ""), read_pr,
                     labels=json.loads(os.environ.get("PR_LABELS", "[]")) or [],
                     head_ref=os.environ.get("HEAD_REF", ""))
    for key, value in values.items():
        print(f"{key}={value}")
    print(f"heavy checks: {values['heavy']}", file=sys.stderr)


if __name__ == "__main__":
    main()
