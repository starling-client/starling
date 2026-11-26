"""The pull request's merge base, shared by every gate that takes --base.

`pull_request_base` picks the ref a gate compares HEAD with (the first parent of CI's pull-request merge).
check_layers.py, check_addresses.py and tools/hotspots/ use it.
"""

from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, encoding="utf-8", check=True, cwd=ROOT).stdout


def lines(text):
    return text.count("\n") + (1 if text and not text.endswith("\n") else 0)


def text_at(rev, path, root=ROOT):
    """`path`'s text at `rev`, or None when `rev` has no such file. A file that is there but can't be read (a partial
    clone's blob whose download failed) stops the check after a second try: an unread baseline is never taken for a
    missing one, which would skip the comparison."""
    listed = subprocess.run(["git", "ls-tree", "--name-only", rev, "--", path], cwd=root, capture_output=True,
                            encoding="utf-8")
    if listed.returncode != 0:
        raise SystemExit(f"can't list {path} at {rev}: {listed.stderr.strip()}")
    if path not in listed.stdout.splitlines():
        return None
    for _ in range(2):
        shown = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=root, capture_output=True, encoding="utf-8")
        if shown.returncode == 0:
            return shown.stdout
    raise SystemExit(f"{path} is at {rev} but can't be read: {shown.stderr.strip()}")


PULL_REQUEST_MERGE_HELP = ("HEAD is the pull request merged into its base branch (CI's pull_request checkout): "
                           "compare with that merge's first parent, the base branch as it is now")


def pull_request_base(base, merged):
    """(the ref to compare HEAD with, error). With `merged`, HEAD merges the pull request into a descendant of `base`
    (the event's base, where the branch last met main): main has changed since, and the merge holds those changes too,
    so only the merge's own change, against its first parent, is the pull request's. Shared by the gates that take
    --base (check_addresses.py, check_layers.py)."""
    if not merged:
        return base, None
    parents = git("rev-list", "--parents", "-n", "1", "HEAD").split()[1:]
    if len(parents) != 2 or subprocess.run(["git", "merge-base", "--is-ancestor", base, parents[0]],
                                           cwd=ROOT).returncode != 0:
        return base, "--pull-request-merge needs HEAD to merge the pull request into a descendant of --base."
    return parents[0], None
