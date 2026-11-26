#!/usr/bin/env python3
"""The repository's formatting check, the one CI's `format` and `lint` jobs and `tools/pr_ready.py` run.

    python3 tools/ci/format_scan.py

`cargo fmt --all` reaches only the modules a `mod` line names, so it misses the gameplay fragments an `include!`
reads, the API and vocabulary files a build script includes, and the generator's
fixtures. Each of those is checked by path as well. Every check runs, so one run names every unformatted file; the
exit status is 1 if any of them failed. The launcher is a separate workspace with its own check.
"""

import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Every `.rs` file under these folders, checked one file at a time (no child modules followed).
TREES = ("crates/sunrise-mod-api/src", "crates/sunrise-mod-vocab/src", "crates/sunrise-client/src/game",
         "crates/sunrise-client/src/health",
         "crates/sunrise-client/src/mobs/goals",
         "crates/sunrise-client/src/save/carrier")
# Files no `mod` line reaches.
DISCOVERED = ("crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/*.rs",
              "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/*/*.rs")
RUSTFMT = ["rustfmt", "--edition", "2024", "--check", "--config", "skip_children=true"]
# Windows limits a command line to 32,767 characters (CHUNK=200).
# On Linux and macOS, ARG_MAX is at least 2 MiB (MAX_ARG_STRLEN is 128 KiB); 500 relative
# paths averaging ~60 characters total ~30 KiB, staying well within Unix limits while reducing process spawn overhead.
CHUNK = 200 if os.name == "nt" else 500


def commands(root: Path) -> list[list[str]]:
    files = sorted(path.relative_to(root).as_posix() for tree in TREES for path in (root / tree).rglob("*.rs"))
    files += sorted(path.relative_to(root).as_posix() for pattern in DISCOVERED for path in root.glob(pattern))
    return [["cargo", "fmt", "--all", "--", "--check"]] + [
        RUSTFMT + files[start:start + CHUNK] for start in range(0, len(files), CHUNK)]


def main() -> int:
    all_commands = commands(ROOT)
    # Run formatting checks in parallel across available cores, overlapping cargo fmt and rustfmt chunks.
    max_workers = min(len(all_commands), os.cpu_count() or 4)
    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        results = list(pool.map(lambda cmd: subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True), all_commands))
    failed = [cmd for cmd, result in zip(all_commands, results) if result.returncode != 0]
    # Replay differences in deterministic command order to avoid interleaved outputs across threads.
    for result in results:
        if result.stdout:
            sys.stdout.write(result.stdout)
        if result.stderr:
            sys.stderr.write(result.stderr)
    sys.stdout.flush()
    sys.stderr.flush()
    if failed:
        print(f"format_scan: {len(failed)} formatting check(s) failed; the differences are above.", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
