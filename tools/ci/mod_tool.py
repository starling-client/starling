#!/usr/bin/env python3
"""The rolling release of the mod tool (docs/release-flow.md, "Rolling mod tool"): the one tag `mod-tool` always holds
the newest `cargo-sunrise-mod` built from main, for the three hosts, with a signed `tool.json` naming its commit.

    mod_tool.py crates                       the folders whose change changes the tool, one per line (from cargo metadata)
    mod_tool.py decide [--force]             what mod-tool.yml's `decide` job writes to GITHUB_OUTPUT (reads COMMIT)
    mod_tool.py tool-json COMMIT OUT         OUT = tool.json for COMMIT
    mod_tool.py newer COMMIT                 fails when the published tool is a commit that COMMIT does not contain

The folders are the workspace packages `cargo-sunrise-mod` depends on (normal and build dependencies, not dev), found
by `cargo metadata`, so a crate the tool starts to use is watched without anyone editing a list here.
"""
import argparse
import importlib.util
import datetime as dt
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

TAG = "mod-tool"
TOOL = "cargo-sunrise-mod"
INFO = "tool.json"
# What builds the tool besides its crates: the lock and the manifest every crate resolves through, the pinned compiler,
# and the files that build, pack, sign and publish it.
BUILD_INPUTS = ("Cargo.toml", "Cargo.lock", "tools/ci/rust-version", "tools/prebuilt_game.py", "tools/ci/mod_tool.py",
                "tools/ci/sign_release.sh", ".github/workflows/mod-tool.yml")
CONTRACT_SOURCE = "crates/sunrise-native-build/src/contract.rs"


def run(*command: str, check: bool = True) -> str:
    done = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if check and done.returncode != 0:
        sys.exit(f"{' '.join(command)}: {done.stderr.strip() or done.stdout.strip()}")
    return done.stdout


def crates(metadata: dict | None = None) -> list[str]:
    """The repository-relative folders of the tool's own crate and of every workspace crate it needs to build."""
    metadata = metadata or json.loads(run("cargo", "metadata", "--locked", "--format-version", "1"))
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    members = set(metadata["workspace_members"])
    start = next(id for id in members if packages[id]["name"] == TOOL)
    seen, todo = set(), [start]
    while todo:
        id = todo.pop()
        if id in seen:
            continue
        seen.add(id)
        for dependency in nodes[id]["deps"]:
            # A dependency only the tests use does not change the tool.
            if dependency["pkg"] in members and any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                todo.append(dependency["pkg"])
    root = Path(metadata["workspace_root"])
    return sorted(Path(packages[id]["manifest_path"]).parent.relative_to(root).as_posix() for id in seen)


def previous() -> str:
    """The commit the published `mod-tool` holds (its tool.json), or "" when there is none yet. Only decides whether to
    build; what a user trusts is the signature, which the publish job checks."""
    done = subprocess.run(["gh", "release", "download", TAG, "--pattern", INFO, "--output", "-"], cwd=ROOT,
                          capture_output=True, text=True)
    if done.returncode != 0:
        if "release not found" in done.stderr or "no assets match" in done.stderr:
            return ""
        sys.exit(f"could not read the published {TAG}: {done.stderr.strip()}")
    commit = json.loads(done.stdout)["commit"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        sys.exit(f"the published {TAG}'s {INFO} names no commit")
    return commit


def is_ancestor(older: str, newer: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", older, newer], cwd=ROOT).returncode == 0


def changed(since: str, head: str) -> list[str]:
    """The files that changed the tool between `since` and `head`; everything when `since` is not in head's history."""
    folders = crates()
    if not since or not is_ancestor(since, head):
        return folders
    paths = [*(folder + "/" for folder in folders), *BUILD_INPUTS]
    return [line for line in run("git", "diff", "--name-only", since, head, "--", *paths).splitlines() if line]


def decide(force: bool) -> None:
    # By its path: putting tools/ on sys.path would shadow the tests' modules of the same names.
    spec = importlib.util.spec_from_file_location("release_train", ROOT / "tools/release_train.py")
    train = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(train)
    head = run("git", "rev-parse", "--verify", os.environ["COMMIT"] + "^{commit}").strip()
    if not is_ancestor(head, "refs/remotes/origin/main"):
        sys.exit(f"::error::refused: {head} is not on main; {TAG} holds only a commit of main")
    held = previous()
    if held and not is_ancestor(held, head):
        sys.exit(f"::error::refused: {TAG} holds {held}, which {head} does not contain: it is newer or from elsewhere")
    touched = changed(held, head)
    build = bool(touched) or force
    reason = (f"{len(touched)} file(s) of the tool changed since {held[:9] or 'nothing'}" if touched else
              f"the tool is unchanged since {held[:9]}" + (", rebuilt by force" if force else ""))
    if build and not train.fork_gate_passed(head):
        sys.exit(f"::error::refused: dyna-checks' gate jobs ({', '.join(sorted(train.GATE))}) did not all pass on a "
                 f"main push of {head}; {TAG} follows a green gate")
    print(f"{'build' if build else 'skip'}: {reason}")
    with open(os.environ["GITHUB_OUTPUT"], "a") as out:
        out.write(f"build={str(build).lower()}\ncommit={head}\n")


def tool_json(commit: str, out: Path) -> None:
    """tool.json: what the signed sums bind to the tool beside each archive's hash."""
    contract = re.search(r"pub const CONTRACT: u32 = (\d+);", (ROOT / CONTRACT_SOURCE).read_text())
    date = run("git", "log", "-1", "--format=%cd", "--date=format-local:%Y-%m-%d", commit).strip()
    out.write_text(json.dumps({"schema": 1, "tag": TAG, "commit": commit, "date": date,
                               "native_contract": int(contract.group(1)),
                               "rust": (ROOT / "tools/ci/rust-version").read_text().strip()}, indent=2) + "\n")


def newer(commit: str) -> None:
    held = previous()
    if held and not is_ancestor(held, commit):
        sys.exit(f"::error::not publishing: {TAG} holds {held}, which {commit} does not contain")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("crates")
    commands.add_parser("decide").add_argument("--force", action="store_true")
    made = commands.add_parser("tool-json")
    made.add_argument("commit")
    made.add_argument("out", type=Path)
    commands.add_parser("newer").add_argument("commit")
    args = parser.parse_args()
    if args.command == "crates":
        print("\n".join(crates()))
    elif args.command == "decide":
        decide(args.force)
    elif args.command == "tool-json":
        tool_json(args.commit, args.out)
    else:
        newer(args.commit)


if __name__ == "__main__":
    main()
