#!/usr/bin/env python3
"""Splits hand-written host doubles by area, for the host split (PR 1).

A test double used to write `impl Host for Double { fn level(..) {..} fn player(..) {..} }`.
Host methods now live in one trait per area, so each overridden method moves to the impl of its
area trait, and every area the double leaves alone is implemented by the generated
`mod_api::impl_<union>_areas!(Double, except(<areas it overrides>))` macro.

Usage: split_doubles.py FILE...   (rewrites the files in place; reads crates/horizon-mod-api/src/host/)
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HOST_DIR = ROOT / "crates/horizon-mod-api/src/host"
UNIONS = {
    "Host": ("Host",),
    "ServerHost": ("ServerHost",),
    "ClientHost": ("ClientHost",),
    "Backend": ("Backend", "ClientBackend"),
    "BackendMut": ("BackendMut",),
    "ClientBackendMut": ("ClientBackendMut",),
}
MACRO = {
    "Host": "impl_host_areas",
    "ServerHost": "impl_server_host_areas",
    "ClientHost": "impl_client_host_areas",
    "Backend": "impl_backend_areas",
    "BackendMut": "impl_backend_mut_areas",
    "ClientBackendMut": "impl_client_backend_mut_areas",
}


def camel(stem):
    return "".join(part.capitalize() for part in stem.split("_"))


def load_methods():
    """(union, method name) -> (area stem, area trait name)."""
    table = {}
    for path in sorted(HOST_DIR.glob("*.rs")):
        stem = path.stem
        text = path.read_text()
        for union, suffixes in UNIONS.items():
            for suffix in suffixes:
                name = camel(stem) + suffix
                match = re.search(rf"^pub trait {name}\b[^{{]*\{{\n(.*?)^\}}\n", text, re.S | re.M)
                if not match:
                    continue
                for fn in re.finditer(r"^    (?:pub )?fn (\w+)", match.group(1), re.M):
                    table[(union, fn.group(1))] = (stem, name)
    return table


def depth_delta(line):
    stripped = re.sub(r'"(?:\\.|[^"\\])*"', '""', line)
    stripped = re.sub(r"'(?:\\.|[^'\\])'", "''", stripped)
    stripped = stripped.split("//")[0]
    return sum(stripped.count(c) for c in "{([") - sum(stripped.count(c) for c in "})]")


def split_items(lines, indent):
    """The items of an impl body: attribute and doc lines stay with the item that follows."""
    items, current, depth, pending = [], [], 0, []
    for line in lines:
        current.append(line)
        depth += depth_delta(line)
        stripped = line.strip()
        if depth == 0 and stripped and not stripped.startswith(("#[", "///", "//")):
            items.append(current)
            current = []
    if current:
        items.append(current)
    return items


EMPTY = re.compile(
    r"^(?P<attrs>(?:[ \t]*#\[[^\n]*\]\n)*)(?P<indent>[ \t]*)impl (?:crate::|mod_api::)?(?P<union>Host|ServerHost|ClientHost|Backend|BackendMut|ClientBackendMut) for (?P<ty>[^\n{]+?) \{\}\n(?!(?:[ \t]*#\[[^\n]*\]\n)*[ \t]*mod_api::impl_)",
    re.M,
)


def rewrite(text, table):
    text = EMPTY.sub(
        lambda m: f"{m.group(0)}{m.group('attrs')}{m.group('indent')}mod_api::{MACRO[m.group('union')]}!({m.group('ty')});\n",
        text,
    )
    pattern = re.compile(
        r"^(?P<attrs>(?:[ \t]*#\[[^\n]*\]\n)*)(?P<indent>[ \t]*)impl (?P<path>(?:crate::|mod_api::)?)(?P<union>Host|ServerHost|ClientHost|Backend|BackendMut|ClientBackendMut) for (?P<ty>[^\n{]+?) \{\n",
        re.M,
    )
    out, pos = [], 0
    for match in pattern.finditer(text):
        if match.start() < pos:
            continue
        indent = match.group("indent")
        close = re.compile(rf"^{re.escape(indent)}\}}\n", re.M).search(text, match.end())
        if not close:
            continue
        body = text[match.end() : close.start()].splitlines(keepends=True)
        union = match.group("union")
        ty = match.group("ty")
        by_trait, rest, overridden = {}, [], []
        for item in split_items(body, indent):
            fn = next((re.match(r"\s*(?:pub )?fn (\w+)", l) for l in item if re.match(r"\s*(?:pub )?fn \w+", l)), None)
            if fn and (union, fn.group(1)) in table:
                stem, trait = table[(union, fn.group(1))]
                by_trait.setdefault((stem, trait), []).extend(item)
                overridden.append(stem)
            else:
                rest.extend(item)
        if not by_trait and not match.group("path") and not rest:
            continue
        out.append(text[pos : match.start()])
        attrs = match.group("attrs")
        if not by_trait:
            out.append(text[match.start() : close.end()])
            out.append(f"{attrs}{indent}mod_api::{MACRO[union]}!({ty});\n")
            pos = close.end()
            continue
        for (stem, trait), items in by_trait.items():
            out.append(f"{attrs}{indent}impl mod_api::prelude::{trait} for {ty} {{\n{''.join(items)}{indent}}}\n")
        out.append(f"{attrs}{indent}impl {match.group('path')}{union} for {ty} {{\n{''.join(rest)}{indent}}}\n")
        skipped = ", ".join(dict.fromkeys(overridden))
        out.append(f"{attrs}{indent}mod_api::{MACRO[union]}!({ty}, except({skipped}));\n")
        # A sibling tier of an overridden area (`ClientBackend` beside `Backend`) is not overridden.
        for suffix in UNIONS[union][1:]:
            for stem in dict.fromkeys(overridden):
                name = camel(stem) + suffix
                if re.search(rf"^pub trait {name}\b", (HOST_DIR / f"{stem}.rs").read_text(), re.M):
                    out.append(f"{attrs}{indent}impl mod_api::prelude::{name} for {ty} {{}}\n")
        pos = close.end()
    out.append(text[pos:])
    return "".join(out)


def main():
    table = load_methods()
    for name in sys.argv[1:]:
        path = Path(name)
        text = path.read_text()
        new = rewrite(text, table)
        if new != text:
            path.write_text(new)
            print(f"split {name}")


if __name__ == "__main__":
    main()
