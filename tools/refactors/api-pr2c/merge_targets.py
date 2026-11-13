#!/usr/bin/env python3
"""Convert the three additional targets brought by the one main merge."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).parent
NAMES = {f'native_{name}': name for name in ('add_particle', 'level_collision_boxes', 'player_simulate_move')}
pattern = re.compile(r'\b(' + '|'.join(NAMES) + r')\b')
for path in ROOT.rglob('*.rs'):
    if any(p in {'target', '.local', '.git'} for p in path.relative_to(ROOT).parts):
        continue
    source = path.read_text()
    new = pattern.sub(lambda m: NAMES[m[0]], source)
    if new != source:
        path.write_text(new)
table = HERE / 'forwarder-renames.tsv'
existing = table.read_text()
with table.open('a') as output:
    for old, new in NAMES.items():
        if old + '\t' not in existing:
            output.write(f'{old}\t{new}\n')
