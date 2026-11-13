#!/usr/bin/env python3
"""One-shot #1458 move; --verify reconstructs every output from a Git baseline."""
import argparse
from collections import Counter
from pathlib import Path
import subprocess

import modding_concept_pages as book

# Paragraph boundaries inside the former omnibus sections. Heading boundaries are automatic.
CUTS = {
    "The generated executable uses": "Native entry point and diagnostics",
    "A linked trait mod implements": "Hook dispatch and level hosts",
    "`Registrar::block_key(id)`": "Registry keys",
    "`BlockPart::clone_item_stack`": "Block Parts",
    "Declare a block entity type": "Block entity types",
    "For an item-specific Part": "Item and tag Parts",
    "A capability is answered": "Capability providers",
    "An item's own capability": "Item places and components",
    "The game never trusts an answer": "Capability answer limits",
    "A `LivingRef` on the server can apply": "Consumable effects",
    "On a `ServerHost`, import": "Transactions",
    "`Host` exposes the documented": "Host helpers and remaining gaps",
}


def original(revision, path):
    return subprocess.check_output(["git", "show", f"{revision}:docs/modding/src/{path}"], text=True)


def split(revision):
    result, owners, pieces = {}, {}, []
    for group in book.GROUPS:
        text = original(revision, group)
        lines = text.splitlines(keepends=True)
        starts, fenced = [], False
        for i, line in enumerate(lines[2:], 2):
            if line.startswith(("```", "~~~")):
                fenced = not fenced
            if fenced:
                continue
            if line.startswith("#"):
                starts.append((i, line.split(" ", 1)[1].strip()))
            for prefix, title in CUTS.items():
                if line.startswith(prefix):
                    starts.append((i, title))
        assert starts[0][0] == 2, f"{group}: expected first section after title"
        result[group] = "".join(lines[:2]) + book.MARKER + "\n"
        result[group.removesuffix(".md") + "/legacy-anchors.txt"] = "".join(
            anchor + "\n" for _, anchor in book.headings(text))
        for n, (start, title) in enumerate(starts):
            end = starts[n + 1][0] if n + 1 < len(starts) else len(lines)
            body = "".join(lines[start:end])
            slug = next(book.headings("# " + title))[1]
            path = f"{group.removesuffix('.md')}/{n:03}-{slug}.md"
            pieces.append((group, path, title, body))
            for _, anchor in book.headings(body):
                owners[group, anchor] = path
    for group, path, title, body in pieces:
        body = book.rewrite(body, lambda u: book.relocate(u, group, path, owners),
                            lambda u: book.relocate_include(u, group, path))
        separator = len(body) - len(body.rstrip("\n")) - 1
        result[path] = (f"<!-- chapter: {title} -->\n<!-- separator: {separator} -->\n\n"
                        + body.rstrip("\n") + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="write the split from this baseline")
    parser.add_argument("--verify", help="check, without writing, against this baseline")
    parser.add_argument("--html", type=Path, help="also verify published anchors in this build")
    args = parser.parse_args()
    assert bool(args.base) != bool(args.verify), "choose --base or --verify"
    result = split(args.base or args.verify)
    for path, expected in result.items():
        dest = book.SRC / path
        if args.base:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(expected)
        else:
            assert dest.read_text() == expected, f"{path}: differs from scripted move"
    for group in book.GROUPS:
        directory = book.SRC / group.removesuffix(".md")
        assert {p.relative_to(book.SRC).as_posix() for p in directory.iterdir()} == {
            p for p in result if p.startswith(group.removesuffix(".md") + "/")}, "unexpected concept source"
        expected = original(args.base or args.verify, group)
        assembled = book.aggregate(book.SRC, group)
        # Cross-section links now name their canonical concept pages in the source.
        # Assembly restores the old local links; no prose or headings are rewritten.
        assert assembled == expected, f"{group}: aggregate differs from original"
        assert Counter(assembled.splitlines()) == Counter(expected.splitlines())
        if args.html:
            html = book.HtmlLinks((args.html / Path(group).with_suffix(".html")).read_text())
            assert {a for _, a in book.headings(expected)} <= html.ids, f"{group}: published anchor lost"
    count = sum(path.endswith('.md') for path in result) - len(book.GROUPS)
    print(f"Exact reconstruction: {count} concept sources and both aggregates")
    print("Moved payload line multisets and order: equal after relative-link/include rebasing")
    print("Declared edits: chapter metadata, frozen legacy anchors, discovery markers, relative links and include paths")


if __name__ == "__main__":
    main()
