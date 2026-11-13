#!/usr/bin/env python3
"""Keep homonymous non-Game symbols and frozen hook docs out of the rename.

Only revert tokens actually changed from native_X to X, never an original API
method X. Game definitions retain their new names; compiler errors identify
remaining callers on Game in the next pass.
"""
import difflib
import re
import subprocess
from pathlib import Path

BASE = 'cb01ae304c7136f1598f97650c63ccfc1da47b90'
ROOT = Path(__file__).resolve().parents[3]
OVERLAPS = {'native_living_heal', 'native_set_block_property', 'native_set_block_property_with', 'native_level_random'}
parsed = subprocess.check_output(['cargo', 'run', '--quiet', '-p', 'horizon-mod-api-gen', '--example', 'audit_game_methods'], text=True)
keep = set()
for row in parsed.splitlines():
    name, path, line, kind = row.split('\t')
    if 'native_' + name in OVERLAPS and kind == 'inherent':
        keep.add((path, int(line), name))
paths = subprocess.check_output(['git', 'diff', '--name-only', '--diff-filter=M', BASE, '--', '*.rs'], text=True).splitlines()
for name in paths:
    path = ROOT / name
    if not path.exists():
        continue
    old = subprocess.check_output(['git', 'show', BASE + ':' + name], text=True)
    new = path.read_text()
    if name.startswith('crates/horizon-mod-api/src/hooks/'):
        path.write_text(old)  # No hook catalog change is part of 2c.
        continue
    a, b = list(re.finditer(r'\w+|[^\w\s]', old)), list(re.finditer(r'\w+|[^\w\s]', new))
    edits = []
    for tag, i, j, x, y in difflib.SequenceMatcher(None, [m[0] for m in a], [m[0] for m in b], autojunk=True).get_opcodes():
        if tag != 'replace' or j-i != y-x:
            continue
        for before, after in zip(a[i:j], b[x:y]):
            if before[0] not in OVERLAPS or after[0] != before[0][7:]:
                continue
            if before[0] == 'native_set_block_property_with' and name.endswith('/native_hooks/shared.rs'):
                continue  # The source-contract test names Game's renamed method, not another receiver.
            line = new.count('\n', 0, after.start()) + 1
            if (name, line, after[0]) not in keep:
                edits.append((after.start(), after.end(), before[0]))
    for start, end, replacement in reversed(edits):
        new = new[:start] + replacement + new[end:]
    if name.startswith('crates/horizon-mod-api/src/host/'):
        docs = iter(re.findall(r'(?m)^\s*///.*$', old))
        new = re.sub(r'(?m)^\s*///.*$', lambda _: next(docs), new)
    path.write_text(new)
