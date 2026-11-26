#!/usr/bin/env python3
"""verify-static's guard of the parity runner's memory (#4896): no parity harness builds the sunrise-client lib test.

Its one rustc outgrew the 16 GiB parity runner, so a harness runs a sunrise-client fixture from
`lab.parity_fixtures()` (crates/sunrise-parity-fixtures) instead. Fails naming file:line of every
`tools/test_*_parity.py` that has:
- a list or tuple of strings naming `sunrise-client` together with `cargo` and `test`, or with `--lib`, whatever its
  other flags and their order (a command line for Cargo);
- a call of a helper whose name contains `test_binary` with `sunrise-client` as an argument.
"""
import ast
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def offenders_in(source):
    """(line, what) for every place in the harness `source` that builds the sunrise-client lib test."""
    found = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, (ast.List, ast.Tuple)):
            words = {item.value for item in node.elts if isinstance(item, ast.Constant)}
            if "sunrise-client" in words and ({"cargo", "test"} <= words or "--lib" in words):
                found.append((node.lineno, "a Cargo command builds the sunrise-client lib test"))
        elif isinstance(node, ast.Call):
            name = getattr(node.func, "id", getattr(node.func, "attr", ""))
            if "test_binary" in name and any(isinstance(arg, ast.Constant) and arg.value == "sunrise-client"
                                             for arg in node.args):
                found.append((node.lineno, f"{name}() builds the sunrise-client lib test"))
    return sorted(found)


def main(root=ROOT):
    failed = False
    for path in sorted((root / "tools").glob("test_*_parity.py")):
        for line, what in offenders_in(path.read_text(encoding="utf-8")):
            failed = True
            relative = path.relative_to(root).as_posix()
            print(f"{relative}:{line}: {what}; run a fixture from lab.parity_fixtures() instead (#4896)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
