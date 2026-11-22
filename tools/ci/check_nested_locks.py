#!/usr/bin/env python3
"""Every nested Cargo workspace's own Cargo.lock still matches its manifests.

The telemetry server, the registry and the probes are their own workspaces with their own lock,
and many take game crates by path. A game crate that gains a dependency changes the root lock, which the gate's
--locked builds check, but not these: their --locked builds then refuse after merge (the telemetry image, the
daily's fixtures). `cargo update --workspace --locked` resolves each one and fails, changing nothing, when its lock
would have to change. A Cargo.lock beside a manifest without `[workspace]` belongs to a package that is copied into
another workspace before it builds, and is not checked here.
"""
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def nested_workspaces(root=ROOT):
    listed = subprocess.run(["git", "ls-files", "-z", "*/Cargo.lock"], cwd=root, capture_output=True, check=True)
    for lock in sorted(filter(None, listed.stdout.decode().split("\0"))):
        manifest = root / Path(lock).parent / "Cargo.toml"
        if manifest.is_file() and "workspace" in tomllib.loads(manifest.read_text()):
            yield manifest


def main(root=ROOT):
    failed = False
    for manifest in nested_workspaces(root):
        result = subprocess.run(["cargo", "update", "--workspace", "--locked", "--quiet", "--manifest-path", str(manifest)],
                                cwd=root, capture_output=True, text=True)
        if result.returncode:
            failed = True
            folder = manifest.parent.relative_to(root)
            # Cargo's own words say why: a stale lock, or a manifest or network error.
            print(result.stderr.strip(), file=sys.stderr)
            print(f"::error file={folder / 'Cargo.lock'}::{folder}/Cargo.lock could not be validated (cargo exited"
                  f" {result.returncode}). If the lock needs updating, run: cargo update --workspace --manifest-path"
                  f" {folder / 'Cargo.toml'}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
