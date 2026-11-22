#!/usr/bin/env python3
"""Writes sunrise-release.json, the facts a launcher reads from a game build before it runs anything in it.

    python3 tools/release_file.py write --channel nightly --date 2026-11-16 --commit <sha> sunrise-release.json
    python3 tools/release_file.py write --channel dev --date 2026-11-16 --commit <sha> sunrise-release.json
    python3 tools/release_file.py write --channel stable --build 26.3-0.1.0 --date 2026-11-20 --commit <sha> out.json

The contract is docs/api/release-file.md. Values come from the built commit through `git show`, so nothing of the
commit reaches the disk or runs (nightly-build.yml's `pick` job keeps that rule).
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[1]

# Bump when a launcher must change to install or run a build; docs/api/release-file.md lists what counts.
CONTRACT = 1
# The oldest launcher that may install a build of this contract.
LAUNCHER_MIN = "1.0.0"
CHANNELS = ("nightly", "dev", "stable")
# A stable release's build is its tag, `<target>-<semver>` (tools/release_train.py STABLE_TAG).
STABLE_BUILD = r"\d+\.\d+(?:\.\d+)?-(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)"
VERSION_RS = "crates/sunrise-core/src/version.rs"
RUST_VERSION = "tools/ci/rust-version"


def constant(source: str, name: str) -> str:
    """A `pub const NAME: T = value;` from version.rs, as its literal text without quotes."""
    found = re.search(rf"^pub const {name}: [^=]+= (.+);$", source, re.M)
    if not found:
        raise SystemExit(f"{VERSION_RS} has no {name}")
    return found.group(1).strip('"')


def facts(read: Callable[[str], str], channel: str, date: str, commit: str, build: str = "") -> dict:
    """The release file for the build of `commit` on `channel`, reading the commit's files with `read`. A stable
    release names its tag in `build`; the other channels' builds are `<channel>-<date>-<short commit>`."""
    if channel not in CHANNELS:
        raise SystemExit(f"--channel takes {' or '.join(CHANNELS)}, not {channel}")
    if (channel == "stable") != bool(build) or (build and not re.fullmatch(STABLE_BUILD, build)):
        raise SystemExit(f"--build takes a release's tag, like 26.3-0.1.0, for --channel stable only, not {build!r}")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise SystemExit(f"--commit takes a full commit id, not {commit}")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
        raise SystemExit(f"--date takes YYYY-MM-DD, not {date}")
    version = read(VERSION_RS)
    return {
        "contract": CONTRACT,
        "launcher_min": LAUNCHER_MIN,
        "channel": channel,
        "build": build or f"{channel}-{date}-{commit[:7]}",
        "commit": commit,
        "version_id": constant(version, "VERSION_ID"),
        "vanilla": constant(version, "VANILLA_VERSION"),
        "protocol": int(constant(version, "PROTOCOL_VERSION")),
        "java_major": int(constant(version, "JAVA_VERSION")),
        "rust": read(RUST_VERSION).strip(),
    }


def git_reader(commit: str) -> Callable[[str], str]:
    def read(path: str) -> str:
        return subprocess.run(["git", "show", f"{commit}:{path}"], cwd=ROOT, check=True, capture_output=True,
                              text=True).stdout
    return read


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    write = commands.add_parser("write", help="write the release file of one build")
    write.add_argument("--channel", required=True, choices=CHANNELS)
    write.add_argument("--build", default="", help="--channel stable: the release's tag (26.3-0.1.0)")
    write.add_argument("--date", required=True, help="the build's date, YYYY-MM-DD (UTC)")
    write.add_argument("--commit", required=True, help="the full id of the commit built")
    write.add_argument("out", type=Path)
    args = parser.parse_args(argv)
    document = facts(git_reader(args.commit), args.channel, args.date, args.commit, args.build)
    args.out.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"{args.out}: contract {CONTRACT}, {document['build']}, Minecraft {document['version_id']}", file=sys.stderr)


if __name__ == "__main__":
    main()
