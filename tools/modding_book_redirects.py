#!/usr/bin/env python3
"""Check docs/modding/redirects.json, the book's old addresses and the pages that replaced them (docs/modding-book.md)."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
BOOK = "docs/modding/src"
REDIRECTS = "docs/modding/redirects.json"
# The site's own rule for a page's address: lowercase words joined by hyphens, folders by slashes.
ADDRESS = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*(?:/[a-z0-9]+(?:-[a-z0-9]+)*)*")


def address(page):
    """A page's address under /docs/modding: its path without `.md` or numeric order prefixes; introduction.md is ""."""
    if page == "introduction.md":
        return ""
    return re.sub(r"(^|/)\d+-", r"\1", page.removesuffix(".md"))


def addresses(pages):
    return {address(p) for p in pages if p.endswith(".md") and p != "SUMMARY.md"}


def pages_on_disk(root=ROOT):
    src = root / BOOK
    return [p.relative_to(src).as_posix() for p in src.rglob("*.md")]


def pages_at(commit, root=ROOT):
    out = subprocess.run(["git", "-C", str(root), "ls-tree", "-r", "-z", "--name-only", commit, "--", BOOK],
                         check=True, capture_output=True, text=True).stdout
    return [p.removeprefix(f"{BOOK}/") for p in out.split("\0") if p]


def problems(text, pages):
    """What is wrong with the redirects file's text, given the book's pages."""
    try:
        redirects = json.loads(text)
    except json.JSONDecodeError as error:
        return [f"{REDIRECTS} is not JSON: {error}"]
    if not isinstance(redirects, dict):
        return [f"{REDIRECTS} must be one object of old address to new address"]
    found = []
    if text != json.dumps(dict(sorted(redirects.items())), indent=2) + "\n":
        found.append(f"{REDIRECTS} must list each address once, sorted, with two-space indents and a final newline")
    here = addresses(pages)
    for old, new in redirects.items():
        if not isinstance(new, str) or not ADDRESS.fullmatch(old) or not (new == "" or ADDRESS.fullmatch(new)):
            found.append(f"{old!r}: {new!r} is not an old address and a new one (lowercase words, hyphens, slashes)")
            continue
        if old in here:
            found.append(f"{old} is a page of the book; its redirect would hide it")
        if new not in here:
            found.append(f"{old} redirects to {new or 'the introduction'}, which is not a page of the book")
    return found


def unredirected(before, after, redirects):
    """The addresses the book had before and has no longer, with no redirect."""
    return sorted(addresses(before) - addresses(after) - set(redirects))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="the commit to compare with: each address it had and the book lost needs an entry")
    args = parser.parse_args(argv)
    text = (ROOT / REDIRECTS).read_text(encoding="utf-8")
    pages = pages_on_disk()
    found = problems(text, pages)
    if args.base and not found:
        lost = unredirected(pages_at(args.base), pages, json.loads(text))
        if lost:
            found.append(f"{len(lost)} address(es) of the book at {args.base[:10]} have no page and no redirect; add "
                         f"to {REDIRECTS} (the page that replaces each, or \"\" for the introduction):\n"
                         + "\n".join(f'  "{a}": "",' for a in lost))
    if found:
        print("\n".join(found), file=sys.stderr)
        return 1
    print(f"{REDIRECTS}: every old address is gone from the book and every new one is a page"
          + (f"; every address the book lost since {args.base[:10]} has one" if args.base else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
