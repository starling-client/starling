#!/usr/bin/env python3
"""A pull request's changed files from GitHub's API, in the form rust_changes.py reads them, so dyna-checks' `changes`
job can select checks without fetching the history.

usage: pr_files.py OUT    (env: GITHUB_API_URL, GITHUB_REPOSITORY, GH_TOKEN, BASE_SHA, HEAD_SHA, CHANGED_FILES)

Writes OUT as `git diff --name-status --no-renames -z BASE_SHA...HEAD_SHA` would, and prints `listed=true` and
`entries=N`, the number of records it wrote, which rust_changes.py --entries checks the listing against. The list
is GitHub's comparison of those two commits (`compare/BASE_SHA...HEAD_SHA`), the same three-dot diff, so it is tied to
this run's pair whatever the pull request's branches do meanwhile. A rename is a deletion and an addition, as with
--no-renames; a copy is an addition. Prints `listed=false` (and the reason on stderr) whenever the list may differ
from git's, so the job falls back to the history:
- an API error, or a file status other than added, removed, modified, renamed or copied;
- a comparison of other commits than the pair asked for;
- the list reaches the comparison's 300 files, or its count differs from the event's changed_files;
- a record with no usable path (empty, a `.` or `..` component, a NUL) or a path listed twice.
Never fails the step.
"""

import json
import os
import re
import sys
import urllib.request

# GET /compare/{base}...{head} lists at most this many files, all on its first page.
API_LIMIT = 300
STATUS = {"added": "A", "removed": "D", "modified": "M", "copied": "A"}


def repo_path(path):
    """Whether `path` is a repository-relative path as git prints one: no empty, `.` or `..` component, no NUL."""
    return isinstance(path, str) and "\0" not in path and not {"", ".", ".."}.intersection(path.split("/"))


def get(path):
    request = urllib.request.Request(
        f"{os.environ.get('GITHUB_API_URL', 'https://api.github.com')}/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
        headers={"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"})
    if os.environ.get("GH_TOKEN"):
        request.add_header("Authorization", f"Bearer {os.environ['GH_TOKEN']}")
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def entries(files):
    """git's (status, path) pairs for the API's file list, or None for an incomplete or unrecognized record or a
    path listed twice."""
    out = []
    for item in files:
        if not isinstance(item, dict) or not repo_path(item.get("filename")):
            return None
        status, path = item.get("status"), item["filename"]
        if status == "renamed" and repo_path(item.get("previous_filename")):
            out += [("D", item["previous_filename"]), ("A", path)]
        elif isinstance(status, str) and status in STATUS:
            out.append((STATUS[status], path))
        else:
            return None
    return out if len({path for _, path in out}) == len(out) else None


def listed():
    """The (status, path) pairs, or the reason they can't stand for git's diff."""
    base, head, expected = os.environ["BASE_SHA"], os.environ["HEAD_SHA"], int(os.environ["CHANGED_FILES"])
    if expected <= 0:
        return "the event did not name a nonempty diff"
    if expected >= API_LIMIT:
        return f"{expected} files: the comparison lists at most {API_LIMIT}"
    # Full commit ids, not names that could move: the comparison is of exactly these two commits.
    if not all(re.fullmatch(r"[0-9a-f]{40}", sha) for sha in (base, head)):
        return "the base and head are not both commit ids"
    # One commit a page: the files come whole on the first page, and the commits are not needed.
    comparison = get(f"compare/{base}...{head}?per_page=1")
    if comparison.get("base_commit", {}).get("sha") != base:
        return "the comparison is not of this run's base"
    files = comparison.get("files")
    if not isinstance(files, list):
        return "the comparison has no file list"
    if len(files) != expected:
        return f"the comparison lists {len(files)} files, the event {expected}"
    return entries(files) or "the comparison has an incomplete or unrecognized file record"


def main(out):
    try:
        changes = listed()
    except Exception as error:  # noqa: BLE001 - any failure only means falling back to git
        changes = f"{type(error).__name__}: {error}"
    if isinstance(changes, str):
        print(f"pr_files: {changes}; selecting from the history instead", file=sys.stderr)
        print("listed=false")
        return
    with open(out, "wb") as stream:
        stream.write(b"".join(f"{status}\0{path}\0".encode("utf-8") for status, path in changes))
    print(f"pr_files: {len(changes)} entries from the API", file=sys.stderr)
    print(f"listed=true\nentries={len(changes)}")


if __name__ == "__main__":
    main(sys.argv[1])
