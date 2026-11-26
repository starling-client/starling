#!/usr/bin/env python3
"""One-off: route every game-side `<game>.audio.play*(` call through the `Game::play_audio*`
methods of `game/native_hooks/play_sound.rs`, which fire `play_sound` and play at once."""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates/sunrise-client/src"
CALL = re.compile(r"\b(self|game|g)\.audio\s*\.\s*(play(?:_[a-z_]+)?)\(")


def new_name(method: str) -> str:
    return "play_audio" + method[len("play"):]


changed = 0
for path in ROOT.rglob("*.rs"):
    text = path.read_text()
    out = CALL.sub(lambda m: f"{m.group(1)}.{new_name(m.group(2))}(", text)
    if out != text:
        path.write_text(out)
        changed += 1
print(f"{changed} files rewritten")
