#!/usr/bin/env python3
"""Print the ignored-tests shard matrix: every shard, or the ones a pull request needs.

Usage:
  ignored_shards.py --matrix --all      `matrix=`, every shard (main's pushes)
  ignored_shards.py --matrix BASE HEAD  `matrix=`, the shards this pull request must run

The ignored-tests check is a matrix with one package per shard; this file is the single
place that names them. A shard is selected when a changed crate is its package or a
transitive dependency of it. Workspace-wide files, and any path no crate owns, select
every shard; pushes to main always run everything (the `changes` job passes --all).
crates/horizon-client holds horizon-client-lib's sources too (its [lib] path reaches
into that tree), so that directory counts for both packages; the launcher is its own
workspace (launcher/Cargo.lock) and selects nothing.
"""

import json
import sys
import tomllib
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci import rust_changes  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]

# One package per shard, balanced on the measured test times of the single-job run this
# plan replaced (dyna-ci run #952, e8752e146f: horizon-client 380s, horizon-client-render
# 279s, horizon-client-lib 189s, 16m53s wall); horizon-world and horizon-items joined when every library
# test started running with the data, not only the #[ignore]d ones. Rebalance by editing this tuple only.
SHARDS = ("horizon-client", "horizon-client-lib", "horizon-client-render", "horizon-world", "horizon-items")
# A package too big for one test process's memory runs as several legs, `PACKAGE#NofM` (tools/ci/ignored_tests.py
# gives each test to exactly one leg). horizon-client's single process was killed (signal 9) on three main runs.
LEGS = {"horizon-client": 2}

# Files at the repository root that every package's build reads.
ROOT_WIDE_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
# The crates directory whose sources build more than one of the shard packages.
SHARED_SOURCES = {"horizon-client": ("horizon-client", "horizon-client-lib")}
# Workspaces of their own: nothing under them builds the shard packages.
SEPARATE_WORKSPACES = ("launcher/",)


def expand(packages):
    """The matrix entries for these shard packages: a package with legs becomes one entry per leg."""
    return [
        f"{package}#{number}of{LEGS[package]}" if package in LEGS else package
        for package in packages
        for number in range(1, LEGS.get(package, 1) + 1)
    ]


def workspace_dependencies():
    """Each workspace member's dependency names (its manifest's dependencies and dev-dependencies)."""
    dependencies = {}
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        table = tomllib.loads(manifest.read_text())
        names = set()
        for section in ("dependencies", "dev-dependencies"):
            names.update(table.get(section, {}))
        dependencies[table["package"]["name"]] = names
    return dependencies


def depends_on(start, target, dependencies):
    """Whether `target` is `start` itself or a transitive dependency of `start`."""
    seen, todo = {start}, [start]
    while todo:
        current = todo.pop()
        if current == target:
            return True
        for name in dependencies.get(current, ()):
            if name not in seen:
                seen.add(name)
                todo.append(name)
    return False


def select(paths, dependencies):
    """The shard packages whose tests a changed path set can affect."""
    if not rust_changes.needs_rust(paths):
        return []
    owners = set()
    for path in paths:
        if not (path.startswith("docs/") or path.endswith(".md")):
            if any(path.startswith(prefix) for prefix in SEPARATE_WORKSPACES):
                continue
            if ("/" not in path and path in ROOT_WIDE_FILES) or path.startswith(".cargo/"):
                return list(SHARDS)
            if path.startswith("crates/") and path.count("/") >= 2:
                crate = path.split("/")[1]
                owners.update(SHARED_SOURCES.get(crate, (crate,)))
            else:
                return list(SHARDS)  # no crate owns it: run everything, as the single job did
    return [
        package for package in SHARDS
        if any(depends_on(package, owner, dependencies) for owner in owners)
    ]


if __name__ == "__main__":
    arguments = sys.argv[1:]
    every = arguments[:2] == ["--matrix", "--all"]
    selected = len(arguments) == 3 and arguments[0] == "--matrix"
    if every:
        shards = expand(SHARDS)
    elif selected:
        shards = expand(select(rust_changes.changed_paths(arguments[1], arguments[2]), workspace_dependencies()))
    else:
        sys.exit(__doc__)
    print("matrix=" + json.dumps({"shard": shards}))
