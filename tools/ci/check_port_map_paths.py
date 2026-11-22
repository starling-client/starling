#!/usr/bin/env python3
"""Every tools/port_map/ row that names a Sunrise function still finds it.

`python3 tools/port_ledger.py where Class.method` is where a parity fix starts (AGENTS.md), and it answers
with the rows of `tools/port_map/*.tsv`. A `port` or `inline` row's third column names the Sunrise function
that ports the method, as `path::owner::fn`. Nothing read that column, so a refactor that moved a function
and left its row behind turned the ledger's answer into a file that no longer holds it.

A row resolves when its file declares the function with a body (`fn name`, whatever the visibility), which
is what the ledger indexes; a `use` that re-exports the name from another module is not a definition there,
and neither is a call or a mention in a comment or a string. `tools/port_map_known_stale.txt` lists the rows
that do not resolve today, one `map file<TAB>vanilla name<TAB>path::owner::fn` per line, and it only shrinks
(as `crates/sunrise-mod-api-gen/unwinding-methods.txt` does): a row that does not resolve and is not listed
fails, a listed row that resolves now (or whose row is gone) fails until its line is deleted, and a line added
since `--base` fails. A row's path is part of its key, so repointing one row into another dead path is a new
row, and fails.

    python3 tools/ci/check_port_map_paths.py                 # the check
    python3 tools/ci/check_port_map_paths.py --base SHA      # and nothing added to the list since SHA
    python3 tools/ci/check_port_map_paths.py --refresh       # delete the lines that no longer need one

Fix a row by pointing it at the function's new home: the file that declares `fn name`, or the `impl` block it
moved into. `python3 tools/port_ledger.py map-check FILE` checks the vanilla side and the same Rust side
against a built ledger.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAP = "tools/port_map/*.tsv"
LIST = "tools/ci/port_map_known_stale.txt"
# The verdicts whose third column names a Rust function; `missing`, `home` and `skip:<reason>` name none.
NAMED = ("port", "inline")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
# A declared function's name, in source with comments and strings blanked by `source()`.
DEFINITION = re.compile(r"\bfn\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*)")
# The starts of what `source()` blanks: comments, raw strings, strings, char literals.
SPECIAL = re.compile(r"""//|/\*|(?:b|br)?r(#*)"|b"|"|'(?:\\.|[^\\'])'""")
BLANK = re.compile(r"[^\n]")


def source(text):
    """`text` with comments, strings and char literals blanked, so `fn` in one of them names nothing."""
    out, i, size = [], 0, len(text)
    while True:
        found = SPECIAL.search(text, i)
        if found is None:
            out.append(text[i:])
            return "".join(out)
        out.append(text[i:found.start()])
        token, start = found.group(0), found.start()
        if token == "//":
            end = text.find("\n", start)
            end = size if end < 0 else end
        elif token == "/*":
            depth, end = 1, start + 2
            while depth and end < size:
                opening, closing = text.find("/*", end), text.find("*/", end)
                if closing < 0:
                    end = size
                    break
                if 0 <= opening < closing:
                    depth, end = depth + 1, opening + 2
                else:
                    depth, end = depth - 1, closing + 2
        elif found.group(1) is not None:
            end = text.find('"' + found.group(1), found.end())
            end = size if end < 0 else end + 1 + len(found.group(1))
        elif token.startswith("'") and len(token) > 1:
            end = found.end()
        else:
            end = found.end()
            while end < size:
                if text[end] == "\\":
                    end += 2
                    continue
                if text[end] == '"':
                    end += 1
                    break
                end += 1
        # Keep the newlines so nothing that counts lines can drift; the content itself names nothing.
        out.append(BLANK.sub(" ", text[start:end]))
        i = end


def definitions(text):
    """The functions the file declares with a body: the ledger indexes these, not a bare `fn name;`."""
    names = set()
    for found in DEFINITION.finditer(text):
        i, brackets = found.end(), 0
        while i < len(text):
            char = text[i]
            if char in "([":
                brackets += 1
            elif char in ")]":
                brackets = max(0, brackets - 1)
            elif char == "{" and brackets == 0:
                names.add(found.group(1))
                break
            elif char == ";" and brackets == 0:
                break
            i += 1
    return names


def target_problem(target):
    """Why a named row's third column is not one `path::owner::fn`, or None.

    The ledger reads the whole column as one target, so a second citation separated by a comma belongs in
    the row's note, not here.
    """
    if not target.strip():
        return "the verdict names no target"
    if "," in target:
        return f"{target!r} names more than one target; the ledger reads one, so cite the other in the note"
    file_, separator, rest = target.partition("::")
    name = rest.rpartition("::")[2].strip()
    if not separator:
        return f"{target!r} is not `path::owner::fn`"
    if not file_:
        return f"{target!r} names no file"
    if not name:
        return f"{target!r} names no function"
    if not IDENT.fullmatch(name):
        return f"{target!r} ends in {name!r}, not a function name"
    return None


def parse(root=ROOT):
    """(rows, malformed): every row that names a Rust function, and those that name no usable target."""
    rows, malformed = [], []
    for path in sorted(root.glob(MAP)):
        text = path.read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), 1):
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            parts = [part.strip() for part in line.split("\t")]
            if len(parts) < 2 or parts[1] not in NAMED:
                continue
            target = parts[2] if len(parts) > 2 else ""
            problem = target_problem(target)
            if problem:
                malformed.append((path.relative_to(root).as_posix(), number, parts[0], problem))
            else:
                rows.append((path.relative_to(root).as_posix(), number, parts[0], target))
    return rows, malformed


def rows(root=ROOT):
    """(map file, line number, vanilla name, `path::owner::fn`) of every row that names a Rust function."""
    return parse(root)[0]


def unresolved(refs, root=ROOT):
    """Identity -> why, for each row whose file is missing or does not declare its function.

    An identity is the row's `(map file, vanilla name, path::owner::fn)`: the line number is left out because
    rows move down their file, and the path is in because repointing a row is writing a new one.
    """
    sources, out = {}, {}
    for map_file, _, name, where in refs:
        file_, _, rest = where.partition("::")
        fn = rest.rpartition("::")[2].strip()
        target = root / file_
        if not target.is_file():
            out[(map_file, name, where)] = f"no file {file_}"
            continue
        if file_ not in sources:
            sources[file_] = definitions(source(target.read_text(encoding="utf-8", errors="replace")))
        if fn not in sources[file_]:
            out[(map_file, name, where)] = f"{file_} does not declare fn {fn}"
    return out


def listed(text):
    """The identities of a list file's lines, comments and blank lines left out."""
    return {tuple(line.split("\t")) for line in text.splitlines()
            if line.strip() and not line.lstrip().startswith("#")}


def numbered(text):
    """Identity -> its line number in the list file."""
    return {tuple(line.split("\t")): number for number, line in enumerate(text.splitlines(), 1)
            if line.strip() and not line.lstrip().startswith("#")}


def base_listed(base, root=ROOT):
    """The list as it was at `base`, or None when the file is new there. An unreadable list is fatal."""
    known = subprocess.run(["git", "-C", str(root), "cat-file", "-e", f"{base}^{{commit}}"],
                           capture_output=True, text=True)
    if known.returncode != 0:
        sys.exit(f"git cannot read {base}, so the list at the base cannot be read:\n{known.stderr}")
    listed_at = subprocess.run(["git", "-C", str(root), "ls-tree", "--name-only", base, "--", LIST],
                               capture_output=True, text=True)
    if listed_at.returncode != 0:
        sys.exit(f"git cannot list {LIST} at {base}, so it cannot tell a new list from an unreadable one:\n"
                 f"{listed_at.stderr}")
    if not listed_at.stdout.strip():
        return None
    was = subprocess.run(["git", "-C", str(root), "show", f"{base}:{LIST}"], capture_output=True, text=True)
    if was.returncode != 0:
        sys.exit(f"git lists {LIST} at {base} but cannot read it:\n{was.stderr}")
    return listed(was.stdout)


def report(unresolved_rows, malformed=(), obsolete=(), lines=None, list_lines=None):
    """The failure text: each row at its map file and line, and what to do about it."""
    lines, list_lines = lines or {}, list_lines or {}
    problems = [f"{map_file}:{line}: {name}: target is malformed: {why}"
                for map_file, line, name, why in malformed]
    for identity, why in sorted(unresolved_rows.items()):
        map_file, name, where = identity
        problems.append(f"{map_file}:{lines.get(identity, '?')}: {name} -> {where}: {why}")
    for identity in sorted(obsolete):
        _, name, where = identity
        detail = (f"listed at {identity[0]}:{lines[identity]}, which resolves now" if identity in lines
                  else "its row is gone from tools/port_map")
        problems.append(f"{LIST}:{list_lines.get(identity, '?')}: {name} -> {where}: {detail}; "
                        f"delete its line (or run --refresh)")
    return problems


def main(argv=None, root=ROOT):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="a commit whose list this one may not grow past")
    parser.add_argument("--refresh", action="store_true", help="delete the list lines that no longer need one")
    args = parser.parse_args(argv)

    refs, malformed = parse(root)
    stale = unresolved(refs, root)
    path = root / LIST
    text = ""
    if path.exists():
        # newline="" keeps each retained line's own ending when --refresh writes the file back.
        with path.open(encoding="utf-8", newline="") as list_file:
            text = list_file.read()
    known, list_lines = listed(text), numbered(text)
    lines = {(map_file, name, where): number for map_file, number, name, where in refs}
    unexplained = {identity: why for identity, why in stale.items() if identity not in known}
    obsolete = known - set(stale)

    if args.refresh and obsolete:
        # Keep every line that still needs to be there, comments and order included: --refresh only deletes.
        kept = [line for line in text.splitlines(keepends=True)
                if not (line.strip() and not line.lstrip().startswith("#")
                        and tuple(line.rstrip("\r\n").split("\t")) in obsolete)]
        with path.open("w", encoding="utf-8", newline="") as list_file:
            list_file.write("".join(kept))
        print(f"port map: {len(obsolete)} line(s) removed from {LIST}.")
        known -= obsolete
        obsolete = set()

    added = set()
    if args.base:
        was = base_listed(args.base, root)
        added = known - was if was is not None else set()

    problems = report(unexplained, malformed, obsolete, lines, list_lines)
    for identity in sorted(added):
        _, name, where = identity
        problems.append(f"{LIST}:{list_lines.get(identity, '?')}: {name} -> {where}: added since "
                        f"{args.base}; the list only shrinks")
    if problems:
        counted = []
        if malformed:
            counted.append(f"{len(malformed)} malformed row(s)")
        if unexplained:
            counted.append(f"{len(unexplained)} row(s) name a function that is not there")
        if obsolete:
            counted.append(f"{len(obsolete)} listed row(s) need no line")
        if added:
            counted.append(f"{len(added)} line(s) added since the base")
        print(f"port map: {', '.join(counted)}:", file=sys.stderr)
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"port map: {len(refs)} row(s) name a Rust function, {len(known)} listed as not resolving yet.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
