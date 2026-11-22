#!/usr/bin/env python3
"""Run the modding book's start pages as a reader would, with the mod tool alone.

Every ```sh block of the pages in docs/modding/src/start/ is run, in the page's order, in a fresh home folder and a
fresh working folder: the install page first, then each other page from its own empty folder. A block that cannot run
here (another computer's, or one that needs the reader's own game and Minecraft files) says so in the page itself, in
a comment on the line before it:

    <!-- quickstart-test: skip, <why> -->

so a new block is run unless its author says why not. A block that runs in the project an earlier page made (the
pages run in file name order) names its folder the same way, under the run's scratch folder, where each page's working
folder is named after the page:

    <!-- quickstart-test: in first-mod/greeter -->

The run fails at the first block that exits non-zero and names its page and line.

    python3 tools/book_quickstart.py                 # the pages as written: the tool the install page downloads
    python3 tools/book_quickstart.py --tool <path>   # the same, then with this cargo-sunrise-mod for the mod pages

`--tool` stands for the install page's "run the block again" with a newer tool: the page's own blocks still run first.
The pages download the tool, the SDK and a Rust toolchain, so this needs the network; it is a daily check
(book-quickstart.yml), never a pull request's. It refuses to start when a launcher is on PATH: the mod tools must
work without one.
"""
import argparse
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
START = ROOT / "docs/modding/src/start"
INSTALL = "install-tool.md"
MARK = re.compile(r"<!--\s*quickstart-test:\s*(?:skip,\s*(?P<skip>.+?)|in\s+(?P<folder>\S+))\s*-->")
LAUNCHER = "sunrise-launcher"


def blocks(text):
    """The page's ```sh blocks in order, each as (line of its fence, code, why it is skipped or None, the folder
    under the run's scratch folder it runs in or None)."""
    found, lines, at = [], text.splitlines(), 0
    while at < len(lines):
        if lines[at].strip() != "```sh":
            at += 1
            continue
        end = next((i for i in range(at + 1, len(lines)) if lines[i].strip() == "```"), None)
        if end is None:
            raise SystemExit(f"line {at + 1}: a ```sh block that never closes")
        before = next((line for line in reversed(lines[:at]) if line.strip()), "")
        mark = MARK.fullmatch(before.strip())
        found.append((at + 1, "\n".join(lines[at + 1:end]), mark and mark["skip"], mark and mark["folder"]))
        at = end + 1
    return found


def run_page(page, folder, env, log=sys.stdout):
    """Runs the page's blocks in one shell started in `folder`, a folder of the run's scratch folder. Returns what went
    wrong, or None."""
    found = blocks(page.read_text())
    read, write = os.pipe()
    script, runnable = ["set -e"], []
    for line, code, skip, where in found:
        if skip:
            print(f"{page.name}:{line}: not run ({skip})", file=log, flush=True)
            continue
        runnable.append(line)
        # The marker goes to the descriptor Python reads, so the failing block is known whatever the block prints.
        script.append(f"echo {line} >&{write}")
        if where:
            script.append(f"cd -- {shlex.quote(str(Path(folder).parent / where))}")
        script.append(code)
    process = subprocess.Popen(["bash", "-c", "\n".join(script)], cwd=folder, env=env, pass_fds=(write,),
                               stdout=log, stderr=subprocess.STDOUT)
    os.close(write)
    with os.fdopen(read) as markers:
        started = [int(line) for line in markers.read().split()]
    if process.wait() != 0:
        return f"{page}:{started[-1] if started else 0}: this block failed; its output is above"
    # A block that ends the shell with success (`exit 0`) must not pass for the blocks after it.
    missed = [line for line in runnable if line not in started]
    if missed:
        return f"{page}:{missed[0]}: this block never started: the shell ended before it, with success"
    if runnable:
        print(f"{page.name}: every block ran", file=log, flush=True)
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tool", type=Path, help="a cargo-sunrise-mod that replaces the installed one for the mod pages")
    parser.add_argument("--pages", type=Path, default=START, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if shutil.which(LAUNCHER):
        raise SystemExit(f"{LAUNCHER} is on PATH: this check runs only where PATH holds no launcher")
    if not shutil.which("cargo"):
        raise SystemExit("cargo is not on PATH: the pages ask for Rust first")
    if not (args.pages / INSTALL).is_file():
        raise SystemExit(f"{args.pages / INSTALL}: the install page is missing")
    pages = sorted(args.pages.glob("*.md"), key=lambda page: (page.name != INSTALL, page.name))
    with tempfile.TemporaryDirectory(prefix="book-quickstart-") as scratch:
        home = Path(scratch, "home")
        Path(scratch, "tmp").mkdir()
        home.mkdir()
        # A fresh home, Cargo home and temporary folder. Only the reader's Rust is kept: rustup's toolchains
        # (RUSTUP_HOME) and the `cargo` on PATH. An inherited Cargo home could hold an older `cargo-sunrise-mod`,
        # which cargo prefers to PATH's.
        env = {name: value for name, value in os.environ.items()
               if not name.startswith(("SUNRISE_LAUNCHER", "XDG_")) and name not in ("SUNRISE_MOD_CACHE", "BASH_ENV")}
        env["RUSTUP_HOME"] = os.environ.get("RUSTUP_HOME") or str(Path.home() / ".rustup")
        env["CARGO_HOME"] = str(Path(scratch, "cargo"))
        env["TMPDIR"] = str(Path(scratch, "tmp"))
        env["HOME"] = str(home)
        # Where the install page puts the tool; its `export` lasts for that page's shell only.
        env["PATH"] = f"{home / '.local/bin'}{os.pathsep}{env['PATH']}"
        for page in pages:
            folder = Path(scratch, page.stem)
            folder.mkdir()
            problem = run_page(page, folder, env)
            if problem:
                raise SystemExit(problem)
            if page.name == INSTALL and args.tool:
                shutil.copy2(args.tool, home / ".local/bin/cargo-sunrise-mod")
                print(f"the mod pages run with {args.tool}", flush=True)


if __name__ == "__main__":
    main()
