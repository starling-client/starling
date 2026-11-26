"""One-off conversion for #4907: makes sunrise-client lib tests runnable by the `sunrise-parity-fixtures` binary.

For each named test it moves the body into a function beside it, `run_<test>`, that a build without `test` compiles,
and leaves the `#[test]` (its attributes, name and ignore reason untouched, so test rows and discovery still see it)
calling that function: crates/sunrise-parity-fixtures/build.rs's table finds exactly that shape. The inline module that
holds it becomes visible to `game`, where the table is; a test at the top of a file module is reported, for the
module's declaration to be made visible by hand.

Usage: parity_fixture_split.py TESTS.json NAME...   (TESTS.json: ignored_inventory.tests() rows as dicts)
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def skip_code(text, i):
    """The index after a comment, string or char literal starting at i, or None if none starts there."""
    if text.startswith("//", i):
        end = text.find("\n", i)
        return len(text) if end < 0 else end
    if text.startswith("/*", i):
        depth, j = 1, i + 2
        while depth:
            if text.startswith("/*", j):
                depth, j = depth + 1, j + 2
            elif text.startswith("*/", j):
                depth, j = depth - 1, j + 2
            else:
                j += 1
        return j
    raw = re.match(r'b?r(#*)"', text[i:i + 260])
    if raw and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
        close = '"' + raw.group(1)
        return text.index(close, i + raw.end()) + len(close)
    if text[i] == '"' or text.startswith('b"', i) and not (text[i - 1].isalnum() or text[i - 1] == "_"):
        j = i + (2 if text[i] == "b" else 1)
        while text[j] != '"':
            j += 2 if text[j] == "\\" else 1
        return j + 1
    if text[i] == "'":
        if text[i + 1] == "\\":
            return text.index("'", i + 3) + 1
        if text[i + 2] == "'":
            return i + 3
    return None


def matching_brace(text, open_index):
    depth, i = 0, open_index
    while i < len(text):
        skipped = skip_code(text, i)
        if skipped is not None:
            i = skipped
            continue
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("unbalanced braces")


def split(path, line, name):
    text = path.read_text()
    starts = [0] + [m.end() for m in re.finditer("\n", text)]
    # The definition nearest the inventory's line: earlier edits in this file may have moved it a little.
    found = [m.start() for m in re.finditer(rf"\bfn {re.escape(name)}\(\)\s*\{{", text)]
    if not found:
        raise SystemExit(f"{path}:{line}: {name} is not `fn {name}() {{`")
    fn_at = min(found, key=lambda at: abs(at - starts[line - 1]))
    line = text.count("\n", 0, fn_at) + 1
    open_index = text.index("{", fn_at)
    close_index = matching_brace(text, open_index)
    indent = text[starts[line - 1]:fn_at]
    body = text[open_index:close_index + 1]
    wrapper = f"{{\n{indent}    run_{name}();\n{indent}}}\n\n{indent}pub(in crate::game) fn run_{name}() {body}"
    text = text[:open_index] + wrapper + text[close_index + 1:]
    # The innermost inline module around the test.
    depth, module = 0, None
    for match in re.finditer(r"\bmod\s+(\w+)\s*\{", text[:fn_at]):
        open_mod = match.end() - 1
        if matching_brace(text, open_mod) > fn_at:
            module = match
    if module is None:
        print(f"{path}: {name} is at the top of a file module: make its declaration visible by hand", file=sys.stderr)
        path.write_text(text)
        return
    line_start = text.rfind("\n", 0, module.start()) + 1
    declaration = text[line_start:module.end()]
    visible = re.sub(r"^(\s*)(pub(\([^)]*\))?\s+)?mod ", r"\1pub(in crate::game) mod ", declaration)
    text = text[:line_start] + visible + text[module.end():]
    path.write_text(text)


def main():
    tests = {row["name"]: row for row in json.load(open(sys.argv[1]))}
    for name in sys.argv[2:]:
        row = tests[name]
        split(ROOT / row["path"], row["line"], name.rsplit("::", 1)[1])


if __name__ == "__main__":
    main()
