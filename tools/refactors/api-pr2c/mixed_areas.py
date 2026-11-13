#!/usr/bin/env python3
"""Extract area impls from files also holding unchanged draw/service impls."""
import re
import runpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).parent
blocks = runpy.run_path(str(HERE / 'rename.py'))['blocks']
for stem in ('draw', 'services'):
    path = ROOT / f'crates/horizon-client/src/game/host/{stem}.rs'
    source = path.read_text()
    edits = []
    for impl, body, offset in blocks(source, r'impl (\w+) for Game<[^{}]*\{'):
        if impl[1] not in {'DrawClientHost', 'ServicesServerHost', 'ServicesClientHost'}:
            continue
        edits.append((impl.start(), offset + len(body) + 1))
    for start, end in reversed(edits):
        source = source[:start] + source[end:]
    path.write_text(source)
pattern = re.compile(r'\bnative_(fluid_name|bake_layer)\b')
for path in ROOT.rglob('*.rs'):
    if any(p in {'target', '.local', '.git'} for p in path.relative_to(ROOT).parts):
        continue
    source = path.read_text()
    new = pattern.sub(lambda m: m[1], source)
    if new != source:
        path.write_text(new)
with (HERE / 'forwarder-renames.tsv').open('a') as out:
    out.write('native_fluid_name\tfluid_name\nnative_bake_layer\tbake_layer\n')
