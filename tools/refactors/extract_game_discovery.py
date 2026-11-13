#!/usr/bin/env python3
"""Move game area discovery verbatim before making its engine boundary generic."""
from pathlib import Path
import argparse

root = Path(__file__).resolve().parents[2]
source = root / 'crates/horizon-mod-api-gen/src/game_modules.rs'
target = root / 'crates/horizon-game-discovery/src/lib.rs'
start = 'pub(crate) fn declarations('
end = 'pub(crate) fn lan_action_modules('

def extracted(text):
    return text[text.index(start):text.index(end)]

parser = argparse.ArgumentParser()
parser.add_argument('--verify', action='store_true')
parser.add_argument('--moved-commit', help='Verify the pure move commit, before boundary fixes')
args = parser.parse_args()
if args.verify:
    import subprocess
    original = subprocess.check_output(['git', 'show', '9e229a047d:crates/horizon-mod-api-gen/src/game_modules.rs'], cwd=root, text=True)
    moved = (subprocess.check_output(['git', 'show', f'{args.moved_commit}:crates/horizon-game-discovery/src/lib.rs'], cwd=root, text=True)
             if args.moved_commit else target.read_text())
    assert moved.endswith(extracted(original))
else:
    text = source.read_text()
    block = extracted(text)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text('//! Directory-derived game module declarations.\nuse std::{collections::BTreeMap, fs, path::Path};\n\n' + block)
    source.write_text(text.replace(block, 'use horizon_game_discovery::declarations;\n\n'))
