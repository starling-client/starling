#!/usr/bin/env python3
"""Puts back the rustdoc the old generated host had where the area files differ.

The old generator took a host method's doc from the game function behind it (`native_<name>`), so a
`#[helper]` entry's own doc was often not the one mods saw, and a helper added to main after the import
ran carried the doc of its declaration. `host-manifest` lists each method's doc; this rewrites the area
files' docs to the base's wherever the two manifests differ.

Usage: restore_docs.py OLD_MANIFEST NEW_MANIFEST   (manifest lines: owner TAB name TAB signature TAB doc)
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def read(path):
    rows = {}
    for line in Path(path).read_text().splitlines():
        owner, name, _, doc, *_ = line.split("\t")
        rows[(owner, name)] = doc
    return rows


def unescape(debug):
    return debug[1:-1].encode().decode("unicode_escape")


def main():
    old, new = read(sys.argv[1]), read(sys.argv[2])
    wanted = {}
    for key, doc in new.items():
        if key in old and old[key] != doc:
            wanted[key[1]] = unescape(old[key])
    for path in sorted((ROOT / "crates/horizon-mod-api/src/host").glob("*.rs")):
        text = path.read_text()
        for name, doc in wanted.items():
            pattern = re.compile(rf"((?:    ///[^\n]*\n)+)(    fn {name}\b)")
            if not pattern.search(text):
                continue
            lines = doc.split("\n")
            if lines[-1] == "":
                lines.pop()
            text = pattern.sub(lambda m: "".join(f"    ///{line}\n" for line in lines) + m.group(2), text, count=1)
        path.write_text(text)
    print(f"restored the docs of {len(wanted)} methods")


main()
