#!/usr/bin/env python3
"""`verify-static`: hand-written test stand-ins keep up with the API they mirror, checked on the member PR itself.

Two test crates compile hand-written stand-ins for generated mod-API items, and a contributor's push builds neither
(it runs only `format` and `verify-static`), so a missing stand-in used to surface first in a rollup's `verify`.
This script reads source text only (no build) and fails on:

1. Dispatch-test stand-in files (`dispatch-tests/src/fixtures/stubs/<Name>.rs`, `fixtures/api/<Name>.rs`): a file
   that does not define the one item it is named after (its own impls and `use` lines may sit beside it); a name with
   a file in both folders; and a `shared.rs` that defines stand-ins in `mod stubs` (only `use` lines stay there) or
   has a `mod api` again.
2. A type named in a generated dispatch signature (`crates/sunrise-mod-vocab/src/hooks/*.rs`) that no
   dispatch-test fixture defines or re-exports. Raw render hooks do not generate dispatch, and `#[engine]`
   parameters are not part of the listener signature.

Known false negatives (these still surface only in the rollup's build):
- names the generator introduces rather than the hook signature: `Answered<T>` from `answered_by`, an `out =`
  output type such as `StandaloneModelRegistration`;
- a derive or a method the generated dispatch needs on an existing stub, and a stand-in whose shape
  (fields, variants, generics) no longer fits;
- names are matched by their last segment: a stand-in, trait or impl target is one name across the files read, so
  two different types of the same name in different modules count as one;
- the source scanner does not nest block comments: `/* a /* b */ c */` ends at the first `*/`, and its tail is
  read as code.

Every input path above must exist and the hook catalog must yield at least one hook: a rename never
turns the check off silently.

    python3 tools/hotspots/check_stand_ins.py [--root DIR]
"""

import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures"
FOLDERS = ("stubs", "api")
HOOKS = "crates/sunrise-mod-vocab/src/hooks"
# Names a hook signature may use without a fixture: the prelude, std and generic placeholders the generator keeps.
STD = {"Option", "Some", "None", "Vec", "String", "Box", "Result", "Ok", "Err", "Arc", "Rc", "Self", "HashMap",
       "BTreeMap", "HashSet", "BTreeSet", "Cow", "Duration", "Path", "PhantomData", "Fn", "FnMut", "FnOnce", "Send",
       "Sync"}
NAMED = re.compile(r"\b(struct|enum|trait|type|union|fn|mod|const|static)\s+([A-Za-z_]\w*)")


def strip(source):
    """Source with comments and the contents of string and char literals blanked, so braces and words come only
    from code. Lengths and line breaks are kept: an index into the result is an index into `source`."""
    out, i, n = [], 0, len(source)

    def blank(text):
        return re.sub(r"[^\n]", " ", text)

    while i < n:
        c = source[i]
        word_start = i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")
        raw = re.match(r'b?r(#*)"', source[i:]) if word_start and c in "br" else None
        char = re.match(r"'(?:\\[^']{1,10}|[^\\'\n])'", source[i:]) if c == "'" else None
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = n if end < 0 else end
        elif source.startswith("/*", i):
            end = source.find("*/", i + 2)
            end = n if end < 0 else end + 2
        elif raw:
            close = source.find('"' + raw.group(1), i + raw.end())
            end = n if close < 0 else close + 1 + len(raw.group(1))
            out.append(source[i:i + raw.end()] + blank(source[i + raw.end():end - 1 - len(raw.group(1))])
                       + source[end - 1 - len(raw.group(1)):end])
            i = end
            continue
        elif c == '"':
            end = i + 1
            while end < n and source[end] != '"':
                end += 2 if source[end] == "\\" else 1
            end = min(end + 1, n)
            out.append('"' + blank(source[i + 1:end - 1]) + source[end - 1:end])
            i = end
            continue
        elif char:
            end = i + char.end()
            out.append("'" + " " * (char.end() - 2) + "'")
            i = end
            continue
        else:
            out.append(c)
            i += 1
            continue
        out.append(blank(source[i:end]))
        i = end
    return "".join(out)


def block_end(text, open_index):
    """Index just past the brace that closes the one at `open_index`."""
    depth = 0
    for i in range(open_index, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(text)


def spans(text, start=0, end=None):
    """(start, end) of each item at brace depth 0 of `text[start:end]` (stripped text), split at `;` or a closing
    brace; leading blanks are not part of an item."""
    end = len(text) if end is None else end
    items, begin, depth = [], start, 0
    for i in range(start, end):
        c = text[i]
        if c in "{([":
            depth += 1
        elif c in "})]":
            depth -= 1
            # A block item ends at its brace; `static X: T = T { .. };` ends at the `;` after it.
            if depth == 0 and c == "}" and not re.match(r"\s*;", text[i + 1:end]):
                items.append((begin, i + 1))
                begin = i + 1
        elif c == ";" and depth == 0:
            items.append((begin, i + 1))
            begin = i + 1
    result = []
    for a, b in items:
        while a < b and text[a].isspace():
            a += 1
        if a < b:
            result.append((a, b))
    return result


def top_level(text):
    return [text[a:b] for a, b in spans(text)]


def without_attributes(item):
    return re.sub(r"^\s*(#!?\[(?:[^\[\]]|\[[^\]]*\])*\]\s*)*", "", item)


def item_name(item):
    """The name an item defines, or None for `impl`, `use` and macro calls."""
    body = without_attributes(item)
    body = re.sub(r"^pub(\([^)]*\))?\s+", "", body)
    match = re.match(r"(?:unsafe\s+)?(struct|enum|trait|type|union|fn|mod|const|static)\s+([A-Za-z_]\w*)", body)
    return match.group(2) if match else None


def defined_names(text):
    names = {m.group(2) for m in NAMED.finditer(text)}
    for use in re.findall(r"\buse\s+([^;]*);", text):
        names |= set(re.findall(r"\bas\s+([A-Za-z_]\w*)", use))
        names |= set(re.findall(r"\b([A-Za-z_]\w*)\s*(?=[,}]|$)", use.strip()))
    return names


def module_body(text, name):
    match = re.search(rf"(?<![\w:])mod\s+{name}\s*\{{", text)
    if not match:
        return None
    return text[match.end() - 1 + 1:block_end(text, match.end() - 1) - 1]


def missing_inputs(root):
    """The input paths that do not exist: each would otherwise make its check find nothing."""
    paths = [FIXTURES, *(f"{FIXTURES}/{folder}" for folder in FOLDERS), f"{FIXTURES}/shared.rs", HOOKS]
    return [f"{path}: missing; tools/hotspots/check_stand_ins.py reads it. If it moved, update the script's path."
            for path in paths if not (root / path).exists()]


def check_fixture_files(root):
    problems, owners = [], {}
    fixtures = root / FIXTURES
    for folder in FOLDERS:
        for path in sorted((fixtures / folder).glob("*.rs")):
            rel = path.relative_to(root)
            owners.setdefault(path.stem, []).append(rel)
            named = [n for n in map(item_name, top_level(strip(path.read_text()))) if n]
            if named != [path.stem]:
                found = ", ".join(f"`{n}`" for n in named) or "nothing"
                problems.append(f"{rel}: a stand-in file defines exactly the item it is named after, `{path.stem}`; "
                                f"it defines {found}. Give each other item its own file in {FIXTURES}/{folder}/.")
    for name, paths in sorted(owners.items()):
        if len(paths) > 1:
            problems.append(f"`{name}` has a stand-in in both {paths[0]} and {paths[1]}: two definitions are two "
                            "different types to the generated dispatch. Keep one and re-export it with `use` "
                            "from the other scope.")
    shared = fixtures / "shared.rs"
    text = strip(shared.read_text())
    stubs = module_body(text, "stubs")
    if stubs is not None:
        for item in top_level(stubs):
            name = item_name(item)
            if name:
                problems.append(f"{shared.relative_to(root)}: `mod stubs` defines `{name}`; only `use` lines "
                                f"stay there. Move it to {FIXTURES}/stubs/{name}.rs.")
    if module_body(text, "api") is not None:
        problems.append(f"{shared.relative_to(root)}: `mod api` is back; each API stand-in has its own file, "
                        f"{FIXTURES}/api/<Name>.rs.")
    return problems


HOOK = re.compile(r"#\[hook\((?:[^()]|\([^()]*\))*\)\]\s*fn\s+(\w+)\s*(<[^>]*>)?\s*\(")
TARGET = re.compile(r'\btarget\s*=\s*(?:r#*)?"')
ENGINE = re.compile(r"\s*#\[\s*engine\s*\]")


def signature(text, start):
    """The listener parameters and return type of the fn whose `(` is at `start - 1`."""
    parts = []
    in_parameters = True
    depth = 1
    for i in range(start, len(text)):
        char = text[i]
        if char in ";{" and depth == 0:
            break
        if in_parameters and char in ",)" and depth == 1:
            if not ENGINE.match(text, start):
                parts.append(text[start:i])
            start = i + 1
            in_parameters = char == ","
        if char in "([<{":
            depth += 1
        elif char in ")]>}" and not (char == ">" and text[i - 1] == "-"):
            depth -= 1
    else:
        i = len(text)
    return " ".join([*parts, text[start:i]])


def check_hook_types(root):
    fixtures = root / FIXTURES
    known = set(STD)
    for path in fixtures.rglob("*.rs"):
        known |= defined_names(strip(path.read_text()))
    hook_files = sorted((root / HOOKS).glob("*.rs"))
    texts = {}
    for path in hook_files:
        source = path.read_text()
        text = strip(source)
        texts[path] = source, text
        known |= {m.group(2) for m in NAMED.finditer(text)}
    if not any(HOOK.search(text) for _, text in texts.values()):
        return [f"{HOOKS}: no `#[hook(...)] fn` found; the hook-signature check would check nothing. If the "
                "declaration form changed, update tools/hotspots/check_stand_ins.py."]
    missing = {}
    for path, (source, text) in texts.items():
        for match in HOOK.finditer(text):
            target = TARGET.search(text, match.start(), match.end())
            if target and source.startswith("mod_render::render::", target.end()):
                continue
            generics = set(re.findall(r"\b([A-Z]\w*)", match.group(2) or ""))
            for name in re.findall(r"\b([A-Z]\w*)", signature(text, match.end())):
                if name not in known and name not in generics:
                    missing.setdefault(name, []).append((match.group(1), path.relative_to(root)))
    return [f"{hooks[0][1]}: hook `{hooks[0][0]}` names type `{name}`, which has no dispatch-test stand-in"
            + (f" (also hooks {', '.join(f'`{h}`' for h, _ in hooks[1:])})" if len(hooks) > 1 else "")
            + f". Add {FIXTURES}/api/{name}.rs (a mod_api type; stubs/{name}.rs if the generated code names an "
            "engine stub), or a `use` re-export of an existing one."
            for name, hooks in sorted(missing.items())]


def problems(root):
    missing = missing_inputs(root)
    if missing:
        return missing
    return check_fixture_files(root) + check_hook_types(root)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--root", type=Path, default=ROOT)
    args = ap.parse_args(argv)
    found = problems(args.root)
    for problem in found:
        print(problem, file=sys.stderr)
    if found:
        print(f"{len(found)} stand-in problem(s); see tools/hotspots/check_stand_ins.py", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
