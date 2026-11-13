#!/usr/bin/env python3
"""Restore field/module homonyms: the rename table names Game methods only."""
import re
import subprocess
from pathlib import Path

BASE = 'cb01ae304c7136f1598f97650c63ccfc1da47b90'
for name in subprocess.check_output(['git', 'diff', '--name-only', '--diff-filter=M', BASE, '--', '*.rs'], text=True).splitlines():
    if '/host/' in name or '/hooks/' in name or name.startswith('tools/'):
        continue
    old = subprocess.check_output(['git', 'show', BASE + ':' + name], text=True).splitlines(keepends=True)
    path = Path(name)
    new = path.read_text().splitlines(keepends=True)
    if len(old) != len(new):
        continue
    for i, (before, after) in enumerate(zip(old, new)):
        # A field is not the same-named method; a module path is not a call.
        if re.search(r'\bnative_mod_screen\b(?!\s*\()', before):
            new[i] = re.sub(r'\bmod_screen\b(?!\s*\()', 'native_mod_screen', after)
        if 'native_living_effects::' in before:
            new[i] = new[i].replace('living_effects::', 'native_living_effects::')
        if 'game::native_config_files::tests::' in before:
            new[i] = new[i].replace('game::config_files::tests::', 'game::native_config_files::tests::')
    path.write_text(''.join(new))
