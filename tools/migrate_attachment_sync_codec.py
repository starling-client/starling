#!/usr/bin/env python3
"""Mechanically migrate `sync: false` to `sync_codec: None` inside AttachmentType literals."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

LITERAL_START = re.compile(r"\bAttachmentType\s*\{")
SYNC_FALSE = re.compile(r"\bsync:\s*false\b")


def find_block_end(text: str, open_brace: int) -> int:
    depth = 0
    i = open_brace
    n = len(text)
    in_string = False
    escape = False
    while i < n:
        ch = text[i]
        if in_string:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_string = False
        else:
            if ch == '"':
                in_string = True
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    return i + 1
        i += 1
    raise ValueError(f"unmatched '{{' at offset {open_brace}")


def migrate_text(text: str) -> tuple[str, int]:
    parts: list[str] = []
    pos = 0
    replacements = 0
    for match in LITERAL_START.finditer(text):
        if match.start() < pos:
            continue
        open_brace = match.end() - 1
        end = find_block_end(text, open_brace)
        block = text[match.start() : end]
        new_block, count = SYNC_FALSE.subn("sync_codec: None", block)
        if count:
            parts.append(text[pos : match.start()])
            parts.append(new_block)
            pos = end
            replacements += count
    if replacements == 0:
        return text, 0
    parts.append(text[pos:])
    return "".join(parts), replacements


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Report files without editing them")
    args = parser.parse_args()

    root = pathlib.Path(".")
    paths = sorted(
        list(root.glob("crates/**/*.rs"))
        + list(root.glob("docs/modding/**/*.rs"))
        + list(root.glob("docs/modding/**/*.md"))
    )
    changed_files = 0
    total_sites = 0
    for path in paths:
        original = path.read_text(encoding="utf-8")
        updated, count = migrate_text(original)
        if count:
            changed_files += 1
            total_sites += count
            if not args.check:
                path.write_text(updated, encoding="utf-8")
            print(f"{path}: {count}")
    print(f"Total: {total_sites} sites in {changed_files} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
