#!/usr/bin/env python3
"""Rename the `render_name_tag` hook to `render_name_tag_can_render` (#1164), the name of the
NeoForge event it is (`RenderNameTagEvent.CanRender`), beside the new
`render_name_tag_do_render`. Renames its vocabulary and game files, the generated names that
follow the hook's (`native_`/`fire_` functions, `HookId::RenderNameTag`) and the book."""
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SELF = Path(__file__).resolve()
MOVES = [
    "crates/sunrise-mod-vocab/src/hooks/render_name_tag.rs",
    "crates/sunrise-client/src/game/native_hooks/render_name_tag.rs",
]
for old in MOVES:
    subprocess.run(
        ["git", "mv", old, old.replace("render_name_tag.rs", "render_name_tag_can_render.rs")],
        cwd=ROOT,
        check=True,
    )

tracked = subprocess.run(
    ["git", "ls-files", "*.rs", "*.md", "*.toml"], cwd=ROOT, capture_output=True, text=True, check=True
).stdout.split()
for name in tracked:
    path = ROOT / name
    if path.resolve() == SELF or name.startswith("crates/sunrise-mod-vocab/changes/"):
        continue
    old = path.read_text()
    new = re.sub(r"render_name_tag(?![a-z0-9_])", "render_name_tag_can_render", old)
    new = re.sub(r"RenderNameTag(?![A-Za-z0-9_])", "RenderNameTagCanRender", new)
    new = new.replace('neoforge = "RenderNameTagEvent",', 'neoforge = "RenderNameTagEvent.CanRender",')
    if new != old:
        path.write_text(new)
