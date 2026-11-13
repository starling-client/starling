#!/usr/bin/env python3
"""mdBook preprocessor: render declared NeoForge counterparts without a source snapshot."""
import json
from pathlib import Path
import re
import sys

from modding_hook_notes import expand

PAGE = "reference/neoforge-counterparts.md"


def hooks(root):
    rows = []
    for path in sorted((root / "crates/horizon-mod-vocab/src/hooks").glob("*.rs")):
        if path.name == "mod.rs":
            continue
        text = path.read_text(encoding="utf8")
        for m in re.finditer(r"#\[hook\((.*?)\)\]\s*(?:fn\s+(\w+))", text, re.S):
            attr, name = m.group(1), m.group(2)

            def key(k):
                v = re.search(k + r'\s*=\s*"([^"]*)"', attr)
                return v.group(1) if v else None

            neo = key("neoforge")
            if neo is None:
                lst = re.search(r"neoforge\s*=\s*\[(.*?)\]", attr, re.S)
                if lst:
                    neo = ", ".join(re.findall(r'"([^"]*)"', lst.group(1)))
            rows.append((name, key("side") or "", neo))
    return sorted(rows)


def table(rows, head):
    out = [f"| Hook | Side | {head} |", "| --- | --- | --- |"]
    out += [f"| `{n}` | {s} | {c} |" for n, s, c in rows]
    return "\n".join(out)


def page(root):
    rows = hooks(root)
    mapped = [(n, s, f"`{c}`") for n, s, c in rows if c and not c.lower().startswith("none")]
    none = [
        (n, s, re.sub(r"^none\s*\(?(.*?)\)?$", r"\1", c, flags=re.S).strip() or "none")
        for n, s, c in rows
        if c and c.lower().startswith("none")
    ]
    return f"""# NeoForge counterparts

Each hook's `neoforge = ...` key in `crates/horizon-mod-vocab/src/hooks/` names the NeoForge 26.3 event or method it
matches, since a counterpart's name and contract are the rule for a hook. This page lists the keys as declared.
The book build renders it directly from those declarations; there is no generated Markdown snapshot to update.
A key names the event class (`PlayerEvent.Clone`) or runtime callback (`IPayloadHandler.handle`);
notes about cancelling or the event bus belong in the hook's `///` doc, not in the key.

{len(rows)} hooks: {len(mapped)} name a counterpart, {len(none)} say they have none.
This is a declaration reference, not an independent verification of every mapping. Registration events and the
callbacks they register are different moments: packet receipt names `IPayloadHandler.handle`, and per-draw item
decoration names `IItemDecorator.render`, not their registration events. A `none` explains why no event or method
matches that hook's runtime contract.

## Hooks with a counterpart

{table(mapped, "NeoForge")}

## Hooks that declare no counterpart

A hook with no NeoForge counterpart is judged on its merits like any other. The note is the key's own reason.

{table(none, "Reason")}
"""


def preprocess(context, book):
    root = Path(context["root"]).resolve().parents[1]
    return expand(book, page(root), "NeoForge counterparts", PAGE)


if __name__ == "__main__" and sys.argv[1:2] != ["supports"]:
    print(json.dumps(preprocess(*json.load(sys.stdin))))
