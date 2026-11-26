#!/usr/bin/env python3
"""Step 2 of Starling's plan with Sunrise (docs: memory "starling-sunrise-only"): Starling's whole items out of a
Sunrise file into a module of Starling's own beside it, so a sync with Sunrise meets a `mod` line instead of Starling's
code in Sunrise's.

    python3 tools/starling/move_out.py FILE [--base REV] [--name starling] [--dry-run]

FILE is a Sunrise source file (not a mod.rs) that Starling's commits changed since REV (the sync's base). Each hunk
of `git diff REV -- FILE` (the file on disk) that only adds lines and holds whole items (functions, types, constants, impl
blocks, with their docs and attributes) moves, as long as it sits at the file's top level, directly in an inherent
`impl Type {` block (its methods go into an `impl Type {` of the new module) or directly in `mod tests {` (into a
child module of the tests). What moves becomes `pub(super)` where it was private (FILE and its other modules
call it, as before), and its `super::` paths get one more level. FILE gets `mod NAME;` and `use self::NAME::*;`
after its first `mod` line, and `mod tests` the same; the new files are FILE's folder's NAME.rs and tests/NAME.rs.

Run rustfmt over the files after, and check that the crate compiles: what moved may name a private item of FILE's
other modules, which the move can't see.
"""
import argparse
import pathlib
import re
import subprocess
import sys

ITEM = re.compile(r"^\s*(?:#\[|#!\[|///|//!|//|/\*|pub(?:\([\w:]+\))?\s|(?:async\s+|const\s+|unsafe\s+|extern\s+\"\w+\"\s+)*fn\s|"
                  r"struct\s|enum\s|impl[\s<]|const\s|static\s|type\s|trait\s|mod\s|macro_rules!|use\s)")
PRIVATE = re.compile(r"^(\s*)((?:async\s+|const\s+|unsafe\s+)*(?:fn|struct|enum|const|static|type|trait)\s)")
INHERENT = re.compile(r"^impl(?:<[^{]*>)?\s+([A-Za-z_][\w:]*)(?:<[^{]*>)?\s*\{\s*$")
TESTS = re.compile(r"^(?:pub(?:\([\w:]+\))?\s+)?mod tests\s*\{\s*$")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, check=True, encoding="utf-8").stdout


def depths(lines):
    """Each line's brace depth at its start, with strings, chars, raw strings and comments skipped."""
    out, depth, block, raw = [], 0, 0, None
    for line in lines:
        out.append(depth)
        i, n = 0, len(line)
        in_str = False
        while i < n:
            c = line[i]
            if raw is not None:
                end = '"' + "#" * raw
                j = line.find(end, i)
                if j < 0:
                    break
                i, raw = j + len(end), None
                continue
            if block:
                if line.startswith("*/", i):
                    block -= 1
                    i += 2
                elif line.startswith("/*", i):
                    block += 1
                    i += 2
                else:
                    i += 1
                continue
            if in_str:
                if c == "\\":
                    i += 2
                    continue
                if c == '"':
                    in_str = False
                i += 1
                continue
            if line.startswith("//", i):
                break
            if line.startswith("/*", i):
                block += 1
                i += 2
                continue
            m = re.match(r'b?r(#*)"', line[i:])
            if m and (i == 0 or not (line[i - 1].isalnum() or line[i - 1] == "_")):
                raw = len(m.group(1))
                i += m.end()
                continue
            if c == '"':
                in_str = True
                i += 1
                continue
            if c == "'":
                m = re.match(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'])'", line[i:])
                i += m.end() if m else 1
                continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            i += 1
        if in_str:
            # A string that runs on to the next line: Rust strings may; keep it open.
            pass
    return out


def hunks(path, base):
    """(start, count) of each hunk's added lines in the file, 0-based: in a -U0 diff a hunk's new side is all added
    lines, whether or not it also removes some of Sunrise's."""
    # Against the file as it is on disk (its lines are the ones read).
    diff = git("diff", "-U0", base, "--", str(path))
    found = []
    for m in re.finditer(r"(?m)^@@ -\d+(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", diff):
        removed = int(m.group(1)) if m.group(1) is not None else 1
        added = int(m.group(3)) if m.group(3) is not None else 1
        if added > 0 and m.group(3) != "0":
            found.append((int(m.group(2)) - 1, added))
    return found


def whole_items(chunk):
    """Whether these lines are whole items: they open with an item (or its docs/attributes), their braces close,
    and they end where an item ends."""
    text = [line for line in chunk if line.strip()]
    if not text or not ITEM.match(text[0]):
        return False
    d = depths(chunk + [""])
    if d[-1] != 0 or min(d) < 0:
        return False
    last = text[-1].rstrip()
    return last.endswith(("}", ";", "*/")) or last.lstrip().startswith("//")


NAMED = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?(?:(?:async|const|unsafe)\s+)*"
                   r"(fn|struct|enum|const|static|type|trait|union)\s+([A-Za-z_]\w*)")


def names(text_lines):
    """The (kind, name) of each item declared in these lines, at any depth."""
    found = set()
    for line in text_lines:
        m = NAMED.match(line)
        if m:
            found.add((m.group(1).replace("const", "const"), m.group(2)))
    return found


def items_in(lines, d, start, end, theirs=frozenset()):
    """The whole items among added lines [start, end): (a, b, where) for each run of lines that opens with an item
    (its docs and attributes first) at a level that can move (`context`) and closes it before `end`."""
    found, i = [], start
    while i < end:
        line = lines[i]
        where = context(lines, d, i) if line.strip() else None
        if where is None or not ITEM.match(line) or line.lstrip().startswith("use "):
            i += 1
            continue
        level = d[i]
        j, opened = i, False
        while j < end:
            text = lines[j].strip()
            is_note = text.startswith(("//", "#[", "#![", "/*", "*"))
            if not is_note and text:
                opened = True
            nxt = d[j + 1] if j + 1 < len(d) else 0
            if opened and nxt == level and text.endswith(("}", ";", "},")) and not is_note:
                break
            if nxt < level:
                j = end
                break
            j += 1
        if j >= end:
            i += 1
            continue
        b = j + 1
        first = next((lines[k].strip() for k in range(i, b)
                      if lines[k].strip() and not lines[k].strip().startswith(("//", "#[", "#![", "/*", "*"))), "")
        # Sunrise's file has an item of this name: it is Sunrise's, edited, and stays where a sync meets it.
        declared = NAMED.match(first)
        if declared and (declared.group(1), declared.group(2)) in theirs:
            i = b
            continue
        if re.match(r"^(?:pub(?:\([\w:]+\))?\s+)?(?:mod|use|extern\s+crate)\s", first) or first.startswith("macro_rules!"):
            i = b
            continue
        # The blank lines after it go with it.
        while b < end and not lines[b].strip():
            b += 1
        found.append((i, b, where))
        i = b
    return found


def context(lines, d, start):
    """Where line `start` sits: ("top", None), ("impl", Type) or ("tests", None), else None."""
    if d[start] == 0:
        return ("top", None)
    if d[start] != 1:
        return None
    for i in range(start - 1, -1, -1):
        if d[i] == 0 and lines[i].strip():
            header = lines[i].strip()
            m = INHERENT.match(header)
            if m:
                return ("impl", header[:-1].rstrip())
            if TESTS.match(header):
                return ("tests", None)
            return None
    return None


INCLUDE = re.compile(r'((?:include_bytes|include_str|include)!\(\s*)"(?![/\\])([^"]+)"')
RESTRICTED = re.compile(r"^(\s*)pub\((super|self)\)")


def lifted(line):
    """A restricted visibility as it reads one module deeper: `pub(super)` names FILE's parent, which is
    `super::super` from FILE's child; `pub(self)` was FILE, which is `super`."""
    return RESTRICTED.sub(lambda m: m.group(1) + ("pub(in super::super)" if m.group(2) == "super" else "pub(super)"),
                          line)


def widen(chunk, private=True, deeper=1):
    """The chunk's private items (its own level, depth 0 within it) made pub(super), with the fields of its structs
    and the methods of its inherent impl blocks: what was private to FILE was FILE's and its modules', and pub(super)
    in FILE's child is that again (a wider one would outlive the private types it names). `super::` one level up.
    With `private` false (tests), only the paths."""
    out = []
    d = depths(chunk + [""])
    inside = None  # "struct" or "impl" while in such a block opened at depth 0
    for line, depth in zip(chunk, d):
        # Only a visibility the line had: the pub(super) widening below already names FILE.
        lift = (depth == 0 or (depth == 1 and inside in ("struct", "impl"))) and bool(RESTRICTED.match(line))
        if private and depth == 0:
            m = PRIVATE.match(line)
            if m:
                line = f"{m.group(1)}pub(super) {line[len(m.group(1)):]}"
            if re.match(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?struct\s+\w+[^;(]*\{\s*$", line):
                inside = "struct"
            elif INHERENT.match(line.strip()):
                inside = "impl"
            elif line.strip():
                inside = None
        elif private and depth == 1 and inside == "struct":
            m = re.match(r"^(\s*)([a-z_][a-z0-9_]*)\s*:(?!:)", line)
            if m:
                line = f"{m.group(1)}pub(super) {line[len(m.group(1)):]}"
        elif private and depth == 1 and inside == "impl":
            m = PRIVATE.match(line)
            if m:
                line = f"{m.group(1)}pub(super) {line[len(m.group(1)):]}"
        line = INCLUDE.sub(lambda m: f'{m.group(1)}"{"../" * deeper}{m.group(2)}"', line)
        line = re.sub(r"(?<![\w:])super::", "super::super::", line)
        line = re.sub(r"(?<![\w:])self::(?=[a-z_]+::)", "super::", line)
        # After the paths: the visibility it gets names its own path.
        if lift:
            line = lifted(line)
        out.append(line)
    return out


def dedent(chunk, by=4):
    return [line[by:] if line.startswith(" " * by) else line.lstrip() if not line.strip() else line for line in chunk]


def path_loaded(path):
    """Whether a `#[path = "..."]` in the crate loads FILE (it then resolves its modules as a mod.rs does)."""
    src = next((p for p in path.resolve().parents if (p / "Cargo.toml").is_file()), None)
    if src is None:
        return False
    found = subprocess.run(["git", "grep", "-l", "-F", f'{path.name}"]', "--", str(src)], capture_output=True,
                           encoding="utf-8").stdout.split()
    for other in found:
        text = pathlib.Path(other).read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r'#\[path\s*=\s*"([^"]+)"\]', text):
            if (pathlib.Path(other).parent / m.group(1)).resolve() == path.resolve():
                return True
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("file", type=pathlib.Path)
    parser.add_argument("--base", default="46c00106def")
    parser.add_argument("--name", default="starling")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    path = args.file
    if path.name == "mod.rs" or path.name == "lib.rs":
        sys.exit("move_out: a mod.rs or lib.rs keeps its children beside it; not handled")
    lines = path.read_text(encoding="utf-8").split("\n")
    d = depths(lines)
    moved = {"top": [], "impl": {}, "tests": []}
    taken = set()
    skipped = []
    sunrise = subprocess.run(["git", "show", f"{args.base}:{path.as_posix()}"], capture_output=True,
                             encoding="utf-8", errors="replace").stdout.split("\n")
    theirs = names(sunrise)
    for start, count in hunks(path, args.base):
        pieces = items_in(lines, d, start, start + count, theirs)
        if not pieces:
            skipped.append((start + 1, count, None))
            continue
        for a, b, (kind, ty) in pieces:
            chunk = lines[a:b]
            if kind == "top":
                moved["top"].append(widen(chunk))
            elif kind == "impl":
                moved["impl"].setdefault(ty, []).append(widen(chunk))
            else:
                moved["tests"].append(dedent(widen(chunk, private=False, deeper=2)))
            taken.update(range(a, b))
        left = count - sum(b - a for a, b, _ in pieces)
        if left and any(lines[i].strip() for i in range(start, start + count) if i not in taken):
            skipped.append((start + 1, count, f"{left} of its lines stay"))
    total = len(taken)
    print(f"move_out: {path}: {total} lines move ({len(moved['top'])} top-level hunks, "
          f"{sum(len(v) for v in moved['impl'].values())} in impl blocks, {len(moved['tests'])} in tests); "
          f"{len(skipped)} pure-add hunks stay")
    for line, count, where in skipped:
        print(f"  stays: line {line} (+{count}) {where}")
    if args.dry_run or not total:
        return
    folder = path.with_suffix("")
    stem = path.stem
    header = [f"//! Starling's part of {stem}.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling",
              "//! adds there lives here, so a sync with Sunrise meets only the `mod` line.",
              "#![allow(clippy::wildcard_imports)]", "", "#[allow(unused_imports)]", "use super::*;", ""]
    body = list(header)
    for chunk in moved["top"]:
        body += chunk + [""]
    for impl_header, chunks in moved["impl"].items():
        body.append(f"{impl_header} {{")
        for chunk in chunks:
            body += chunk + [""]
        if body[-1] == "":
            body.pop()
        body += ["}", ""]
    folder.mkdir(exist_ok=True)
    module = folder / f"{args.name}.rs"
    if (moved["top"] or moved["impl"]) and module.is_file():
        # A pass before this one made it: what moves now goes after what is there.
        body = module.read_text(encoding="utf-8").rstrip("\n").split("\n") + [""] + body[len(header):]
    if moved["top"] or moved["impl"]:
        module.write_text("\n".join(body).rstrip("\n") + "\n", encoding="utf-8", newline="\n")
    if moved["tests"]:
        tests_header = ["//! Starling's tests of " + stem + ".rs, out of Sunrise's file (tools/starling/move_out.py).",
                        "#![allow(clippy::wildcard_imports)]", "", "#[allow(unused_imports)]", "use super::*;", ""]
        tests = list(tests_header)
        for chunk in moved["tests"]:
            tests += chunk + [""]
        (folder / "tests").mkdir(exist_ok=True)
        test_module = folder / "tests" / f"{args.name}.rs"
        if test_module.is_file():
            tests = test_module.read_text(encoding="utf-8").rstrip("\n").split("\n") + [""] + tests[len(tests_header):]
        test_module.write_text("\n".join(tests).rstrip("\n") + "\n", encoding="utf-8", newline="\n")
    kept = [line for i, line in enumerate(lines) if i not in taken]
    # Explicit paths: a file loaded with `#[path]` resolves its `mod x;` beside itself, not in a folder of its
    # name, and its inline modules' paths from its own folder.
    loaded = path_loaded(path)
    top_mod = [f'#[path = "{stem}/{args.name}.rs"]', f"mod {args.name};", "#[allow(unused_imports)]",
               f"pub use self::{args.name}::*;"]
    inner = f"../{stem}/tests/{args.name}.rs" if loaded else f"{args.name}.rs"
    tests_mod = [f'    #[path = "{inner}"]', f"    mod {args.name};", "    #[allow(unused_imports)]",
                 f"    use self::{args.name}::*;"]
    kd = depths(kept)
    out, in_tests = [], False
    # `mod NAME;` and its glob after the file's first `mod x;` line, else after its last leading `use`, else after
    # its inner docs and attributes; the same at the top of `mod tests {`.
    at = None
    has_top = any(kd[i] == 0 and line.strip() == f"mod {args.name};" for i, line in enumerate(kept))
    has_tests = any(kd[i] == 1 and line.strip() == f"mod {args.name};" for i, line in enumerate(kept))
    if (moved["top"] or moved["impl"]) and not has_top:
        at = next((i for i, line in enumerate(kept)
                   if kd[i] == 0 and re.match(r"^(?:pub(?:\([\w:]+\))?\s+)?mod \w+;\s*$", line)), None)
        if at is None:
            in_use, last = False, None
            for i, line in enumerate(kept):
                text = line.strip()
                if kd[i] != 0:
                    continue
                if re.match(r"^(?:pub(?:\([\w:]+\))?\s+)?use\s", text):
                    in_use = True
                if in_use and text.endswith(";"):
                    last, in_use = i, False
                    continue
                if in_use or not text or text.startswith(("//", "#![", "#[")):
                    continue
                break
            at = last
        if at is None:
            at = max([i for i, line in enumerate(kept[:200]) if line.startswith(("//!", "#!["))], default=-1)
    for i, line in enumerate(kept):
        if at == -1 and i == 0:
            out += top_mod
        out.append(line)
        if i == at:
            out += top_mod
        if moved["tests"] and not has_tests and not in_tests and kd[i] == 0 and TESTS.match(line.strip()):
            out += tests_mod
            in_tests = True
    path.write_text("\n".join(out), encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
