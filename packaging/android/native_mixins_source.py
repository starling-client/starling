"""A build's copy of the game, made readable to horizon-mixins where the fork's source is not.

    native_mixins_source.py SOURCE

Run on SOURCE, the copy that native_build.sh and tools/starling/hnmod_compat.sh prepare (never the checkout), after
horizon-materialize. Two things horizon-mixins refuses in the game since the fork of 2026-11-14:

1. A module named through a parent folder, `#[path = "../x.rs"]` (E_INVALID_PATH: no '.' or '..'):
   `game/mobs/creatures/mod.rs` names five files of `game/creatures/` that way. Each such file moves into the folder
   its `#[path]` is read from, and the attribute names it there. The module's Rust path does not change, so mods
   target it as before. Refused, leaving SOURCE as it was: a file named twice, a moved file that names modules of its
   own (they would be looked for in the new folder), or a name already taken there. Modules under `#[cfg(test)]` are
   left alone: horizon-mixins skips them.
2. `#[horizon_mod_api_macros::replaceable]` on a function a mod targets (E_UNSUPPORTED_MACRO: an unexpanded
   attribute macro). The macro returns the function as written: it marks the functions the fork's prebuilt game
   stages for replacement (`horizon-mod-api-gen` replace.rs), which a phone's build does not use. The marker line goes.
"""

import pathlib
import re
import sys

ATTR = re.compile(r'^(\s*)#\[path\s*=\s*"([^"]*\.\.[^"]*)"\]\s*$')
MOD = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*;")
CHILD = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*;", re.M)
REPLACEABLE = "#[horizon_mod_api_macros::replaceable]"


def parent_paths(source: pathlib.Path) -> None:
    src = source / "crates/horizon-client/src"
    moves = {}
    edits = []
    for file in sorted(src.rglob("*.rs")):
        lines = file.read_text(encoding="utf-8").split("\n")
        for i, line in enumerate(lines):
            m = ATTR.match(line)
            if not m:
                continue
            # The item it is on: the next line that is not an attribute or a comment.
            j = i + 1
            attrs = [line]
            while j < len(lines) and lines[j].strip().startswith(("#[", "//")):
                attrs.append(lines[j])
                j += 1
            k = i - 1
            while k >= 0 and lines[k].strip().startswith(("#[", "///")):
                attrs.append(lines[k])
                k -= 1
            if j >= len(lines) or not MOD.match(lines[j]) or any("cfg(test)" in a for a in attrs):
                continue
            target = (file.parent / m.group(2)).resolve()
            # A test file's own fixtures outside the game (horizon-mod-wasm's, the book's): the test module above
            # them is under #[cfg(test)], which horizon-mixins does not read.
            if not target.is_file() or src not in target.parents:
                continue
            if target in moves:
                sys.exit(f"native_mixins_source: {target.relative_to(source)} is named twice")
            if CHILD.search(target.read_text(encoding="utf-8")):
                sys.exit(f"native_mixins_source: {target.relative_to(source)} names modules of its own")
            new = file.parent / target.name
            if new.exists():
                sys.exit(f"native_mixins_source: {new.relative_to(source)} is taken")
            moves[target] = new
            edits.append((file, i, f'{m.group(1)}#[path = "{target.name}"]'))
    for target, new in moves.items():
        target.rename(new)
    by_file = {}
    for file, i, text in edits:
        by_file.setdefault(file, []).append((i, text))
    for file, changes in by_file.items():
        lines = file.read_text(encoding="utf-8").split("\n")
        for i, text in changes:
            lines[i] = text
        file.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    for target, new in moves.items():
        print(f"native_mixins_source: {target.relative_to(source)} -> {new.relative_to(source)}")


def replaceable_markers(source: pathlib.Path) -> None:
    count = 0
    for crate in sorted((source / "crates").iterdir()):
        if not (crate / "src").is_dir():
            continue
        for file in sorted((crate / "src").rglob("*.rs")):
            text = file.read_text(encoding="utf-8")
            if REPLACEABLE not in text:
                continue
            lines = text.split("\n")
            kept = [line for line in lines if line.strip() != REPLACEABLE]
            if len(kept) != len(lines):
                count += len(lines) - len(kept)
                file.write_text("\n".join(kept), encoding="utf-8", newline="\n")
    print(f"native_mixins_source: {count} #[replaceable] markers off")


def main() -> None:
    source = pathlib.Path(sys.argv[1]).resolve()
    parent_paths(source)
    replaceable_markers(source)


if __name__ == "__main__":
    main()
