#!/usr/bin/env python3
"""Migrate native test bool assertions to explicit Result expectations; no production unwrapping."""
import json
import re
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[3]
paths = subprocess.check_output(
    ["rg", "-l", r"assert!\(!?game\.level_set_block\(", "crates/horizon-client/src", "--glob", "*.rs"],
    cwd=root, text=True,
).splitlines()
report = []
for relative in paths:
    path = root / relative
    source = path.read_text()
    edits = []
    for match in re.finditer(r"assert!\((!?)(game\.level_set_block\()", source):
        depth = 1
        end = match.end()
        while depth:
            char = source[end]
            depth += (char == "(") - (char == ")")
            end += 1
        # These arguments are native typed expressions, not strings; refuse any unexpected
        # syntax rather than expanding this deliberately narrow mechanical rewrite.
        call = source[match.start(2):end]
        if '"' in call or source[end] != ")":
            raise ValueError(f"unexpected assertion syntax in {relative}")
        expected = "false" if match.group(1) else "true"
        replacement = f"assert_eq!({call}, Ok({expected}))"
        edits.append((match.start(), end + 1, replacement))
        report.append({"path": relative, "line": source.count("\n", 0, match.start()) + 1,
                       "expected": expected})
    for start, end, replacement in reversed(edits):
        source = source[:start] + replacement + source[end:]
    path.write_text(source)
print(json.dumps(report, indent=2))
