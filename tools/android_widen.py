#!/usr/bin/env python3
"""Starling: the staged game's crates fully public, for mixin mods built for Horizon (tools/android_native_sdk.py).

    python3 tools/android_widen.py ROOT     # ROOT: the staged copy (tools/prebuilt_game.py `stage`), never a work tree

A mixin's patch crate copies game functions with the imports of their module, and Horizon's mods also name game
items directly. Sunrise split Horizon's game crate into several (sunrise-gameplay, sunrise-client-gameplay...), whose
modules are private, and the staging pass widens only the game crate and the lower items `internals.txt` declares.
Rather than a list that grows with every mod, this makes every module and every restricted item of the game's crates
public in the staged copy: `mod x` becomes `pub mod x`, `pub(crate)`, `pub(super)`, `pub(self)` and `pub(in path)`
become `pub`. The APK is built from the same copy (android.yml, store_base.sh), so a mod built against it loads.
Item names don't change, only who may name them; a private item made public may warn, never fail.

A re-export (`pub(crate) use`) that names a `macro_rules!` macro of its crate stays as it is: a local macro can't be
re-exported publicly (E0364).
"""
import re
import sys
from pathlib import Path

# The crates the game is made of, below the mod API: what a copied function or a Horizon mod's helper names.
CRATES = ("sunrise-client", "sunrise-client-gameplay", "sunrise-gameplay", "sunrise-client-lib", "sunrise-client-render",
          "sunrise-client-input", "sunrise-world", "sunrise-core", "sunrise-protocol", "sunrise-items")
RESTRICTED = re.compile(r"\bpub\s*\(\s*(?:crate|super|self|in\s+[\w:]+)\s*\)")
MACRO = re.compile(r"\bmacro_rules!\s*(\w+)")
RESTRICTED_USE = re.compile(r"\bpub\s*\([^)]*\)\s*use\s+[^;]+;")
# A module declaration with no visibility, at the start of a line (attributes before it stay as they are).
PRIVATE_MOD = re.compile(r"(?m)^(\s*)mod\s+(\w+)\s*([;{])")


def widen(text: str, macros: frozenset = frozenset()) -> str:
    macros = macros | set(MACRO.findall(text))
    kept = []

    def keep(m: re.Match) -> str:
        if set(re.findall(r"\w+", m.group(0))) & macros:
            kept.append(m.group(0))
            return f"\0{len(kept) - 1}\0"
        return m.group(0)

    text = RESTRICTED_USE.sub(keep, text)
    text = RESTRICTED.sub("pub", text)
    for i, original in enumerate(kept):
        text = text.replace(f"\0{i}\0", original)
    return PRIVATE_MOD.sub(lambda m: f"{m.group(1)}pub mod {m.group(2)}{m.group(3)}", text)


def widen_tree(root: Path) -> int:
    """Every file of the game's crates under `root` widened in place; how many changed. Only on a copy."""
    changed = 0
    for crate in CRATES:
        files = [p for p in (root / "crates" / crate).rglob("*.rs") if "target" not in p.parts]
        texts = {p: p.read_text(encoding="utf-8") for p in files}
        macros = frozenset(name for text in texts.values() for name in MACRO.findall(text))
        for path, text in texts.items():
            wide = widen(text, macros)
            if wide != text:
                path.write_bytes(wide.encode("utf-8"))
                changed += 1
    return changed


def main() -> None:
    root = Path(sys.argv[1])
    if not (root / "functions.json").is_file():
        sys.exit(f"{root}: not a staged copy (no functions.json): this rewrites visibility in place")
    print(f"android_widen: {widen_tree(root)} files of {len(CRATES)} crates made public")


if __name__ == "__main__":
    main()
