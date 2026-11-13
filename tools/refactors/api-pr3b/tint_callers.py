#!/usr/bin/env python3
"""Declared edit: append the explicit tint context to four-argument MeshTables calls."""
import json
from pathlib import Path

root = Path(__file__).resolve().parents[3]
report = []
for path in sorted((root / "crates").rglob("*.rs")):
    source = path.read_text()
    token = "MeshTables::new("
    offset = 0
    edits = []
    while (start := source.find(token, offset)) != -1:
        opening = start + len(token) - 1
        stack = ["("]
        commas = []
        end = opening + 1
        while stack:
            char = source[end]
            if char in "([{":
                stack.append(char)
            elif char in ")]}":
                stack.pop()
            elif char == "," and len(stack) == 1:
                commas.append(end)
            end += 1
        closing = end - 1
        inner = source[opening + 1:closing]
        count = len(commas) + (0 if inner.rstrip().endswith(",") else 1)
        if count == 4:
            argument = "api_values::mod_tint_values(assets.registry)?" if path == root / "crates/horizon-client/src/game.rs" else "None"
            if "\n" in inner:
                indent = source[source.rfind("\n", 0, closing) + 1:closing]
                replacement = inner.rstrip() + ("" if inner.rstrip().endswith(",") else ",") + "\n" + indent + "    " + argument + ",\n" + indent
            else:
                replacement = inner.rstrip().rstrip(",") + ", " + argument
            edits.append((opening + 1, closing, replacement))
            report.append({"path": str(path.relative_to(root)), "line": source.count("\n", 0, start) + 1, "argument": argument})
        elif count != 5:
            raise ValueError(f"{path}: unexpected argument count {count}")
        offset = end
    for start, stop, replacement in reversed(edits):
        source = source[:start] + replacement + source[stop:]
    if edits:
        path.write_text(source)
print(json.dumps(report, indent=2))
