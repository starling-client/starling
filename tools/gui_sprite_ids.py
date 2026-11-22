#!/usr/bin/env python3
"""Rewrite the GUI sprite names of the game's own code from the walked texture name
(`gui/sprites/container/slot`, `lobby:gui/sprites/panel`) to the GUI atlas id
(`container/slot`, `lobby:panel`), the key the GUI atlas publishes its sprites by.

Run from the repository root: `python3 tools/gui_sprite_ids.py`. Lines that handle the walked
folder itself (`starts_with`, `strip_prefix`, file paths) are left alone, and so are the files
that build the atlas and the lookups of its packed names."""
import pathlib
import re

SKIP_FILES = {
    "crates/sunrise-client-render/src/hud/gui_sprite.rs",
    "crates/sunrise-client-render/src/hud/sprite_modifiers.rs",
    "crates/sunrise-client-render/src/hud/menu_pictures.rs",
    "crates/sunrise-client-render/src/atlas_sources.rs",
}
SKIP_DIRS = ("crates/sunrise-client-render/src/hud/tests/",)
SKIP_LINE = re.compile(r"starts_with\(|strip_prefix\(|textures/|\.mcmeta")
WHOLE = re.compile(r'"gui/sprites/')
NAMESPACED = re.compile(r'"([a-z0-9_.-]+):gui/sprites/')

changed = 0
for path in sorted(pathlib.Path("crates").rglob("*.rs")):
    if str(path) in SKIP_FILES or str(path).startswith(SKIP_DIRS):
        continue
    text = path.read_text(encoding="utf-8", errors="surrogateescape")
    if "gui/sprites/" not in text:
        continue
    lines = text.split("\n")
    for index, line in enumerate(lines):
        if "gui/sprites/" not in line or SKIP_LINE.search(line):
            continue
        new = NAMESPACED.sub(r'"\1:', WHOLE.sub('"', line))
        if new != line:
            lines[index] = new
            changed += 1
    path.write_text("\n".join(lines), encoding="utf-8", errors="surrogateescape")
print(f"{changed} lines rewritten")
