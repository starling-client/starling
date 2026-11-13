#!/usr/bin/env python3
"""Adds `use mod_api::prelude::*;` to each file the compiler says calls an area-trait method
without the trait in scope (cargo's JSON messages on stdin).

The host methods moved from `Host` to one trait per area, and a method call needs its trait in
scope. A file's first top-level `use` line gets the import; a file where the error is inside a
`mod` block that does not glob its parent gets it inside that block (second pass: --inner).
"""
import json
import re
import sys

top, inner = {}, {}
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("reason") != "compiler-message":
        continue
    m = message["message"]
    if m.get("code") and m["code"]["code"] == "E0599" and any(
        "not in scope" in c["message"] or "trait" in c["message"] and "in scope" in c["message"] for c in m["children"]
    ):
        span = next((s for s in m["spans"] if s["is_primary"]), None)
        if span:
            top.setdefault(span["file_name"], []).append(span["line_start"])
for path, lines in top.items():
    text = open(path).read().split("\n")
    # the innermost `mod ... {` before the first error line, if the file's own imports don't reach it
    first = min(lines)
    block = None
    for i in range(first - 1, -1, -1):
        if re.match(r"^\s*(pub(\(\w+\))? )?mod \w+ \{$", text[i]):
            block = i
            break
    if "--inner" in sys.argv and block is not None:
        indent = re.match(r"^\s*", text[block]).group(0) + "    "
        text.insert(block + 1, f"{indent}use mod_api::prelude::*;")
    else:
        if any(l.strip() == "use mod_api::prelude::*;" for l in text[:80]):
            continue
        at = next((i for i, l in enumerate(text) if l.startswith("use ")), 0)
        text.insert(at, "use mod_api::prelude::*;")
    open(path, "w").write("\n".join(text))
    print("import", path)
