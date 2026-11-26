#!/usr/bin/env python3
"""Remove the old HookId order pins from tracked hook declarations."""

import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[1]
DECLARATIONS = "crates/sunrise-mod-vocab/src/hooks"
MARKER = re.compile(rb"^// catalog-order: [0-9]+(?:\r?\n|\Z)", re.MULTILINE)


def main():
    tracked = subprocess.check_output(
        ["git", "ls-files", "-z", "--", f":(glob){DECLARATIONS}/*.rs"], cwd=ROOT
    )
    for name in tracked.split(b"\0")[:-1]:
        path = ROOT / os.fsdecode(name)
        if path.name == "mod.rs":
            continue
        source = path.read_bytes()
        updated = MARKER.sub(b"", source)
        if updated != source:
            path.write_bytes(updated)


if __name__ == "__main__":
    main()
