#!/usr/bin/env python3
"""Rewrite the no-op child-stderr callbacks for debt item P3-119."""
import re
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / "crates/sunrise-native-build/src"
# Every no-op closure in these files is the stderr callback of run or run_checked.
CALLS = {
    "discover.rs": 1,
    "lib.rs": 6,
    "mixins_patch.rs": 1,
    "patch.rs": 2,
    "prebuilt.rs": 2,
    "registry_compile.rs": 1,
    "registry_generate.rs": 1,
    "registry_plan.rs": 2,
}
CALLBACK = re.compile(r", \|_\| \{\}(?=[,)])|^[ \t]*\|_\| \{\},\n", re.MULTILINE)
LIB = {
    "its output appended to the build log; `line` sees each line of its standard error.":
        "its output appended to the build log.",
    ", line: impl FnMut(&str)": "",
    "run_checked(cmd, what, line, None, None)": "run_checked(cmd, what, None, None)",
    "    mut line: impl FnMut(&str),\n": "",
    "                line(&raw);\n": "",
}

rewritten = {}
for name, expected in CALLS.items():
    path = SRC / name
    text, count = CALLBACK.subn("", path.read_text())
    if count != expected:
        raise SystemExit(f"{name}: expected {expected} callbacks, found {count}")
    if name == "lib.rs":
        for old, new in LIB.items():
            if text.count(old) != 1:
                raise SystemExit(f"{name}: expected one occurrence of {old!r}")
            text = text.replace(old, new)
    rewritten[path] = text
for path, text in rewritten.items():
    path.write_text(text)
