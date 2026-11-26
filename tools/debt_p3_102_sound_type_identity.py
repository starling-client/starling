#!/usr/bin/env python3
"""Give existing full BlockSoundType literals an explicit initial identity."""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]
files = [
    ('sunrise-client/src/menu.rs', 'mod_api::', 1),
    ('sunrise-client/src/mobs/spawn_rules.rs', 'mod_api::', 1),
    ('sunrise-items/src/blocks.rs', 'sunrise_mod_vocab::', 2),
    ('sunrise-mod-api/src/parts/blocks_fixtures/sound_type_game_thread.rs', 'crate::', 2),
]

edits = []
for name, prefix, expected in files:
    path = root / 'crates' / name
    before = path.read_text()
    # A full literal starts with volume and pitch and then a break event. Struct
    # updates already inherit the identity of their base and must stay that way.
    after, count = re.subn(
        r'(BlockSoundType \{\n)( +)(volume:[^\n]+\n +pitch:[^\n]+\n +break_sound:)',
        lambda m: f'{m[1]}{m[2]}identity: {prefix}BlockSoundTypeIdentity::Other,\n{m[2]}{m[3]}',
        before,
    )
    if count != expected:
        raise SystemExit(f'{name}: expected {expected} full sound types, got {count}')
    edits.append((path, after))
for path, after in edits:
    path.write_text(after)
