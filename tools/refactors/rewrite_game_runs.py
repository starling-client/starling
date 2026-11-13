#!/usr/bin/env python3
"""Retarget documented game runs and relocated mod examples."""
from pathlib import Path
import re
import sys
from verify_refactor import verify

if '--verify' in sys.argv:
    verify(Path(__file__).resolve(), 'docs: run the platform executable and mod host examples')
    raise SystemExit(0)

root = Path(__file__).resolve().parents[2]
paths = [root / 'README.md', root / 'CONTRIBUTING.md']
paths += list((root / 'docs').rglob('*.md')) + list((root / 'tools').glob('*.py'))
for p in paths:
    text = p.read_text()
    new = []
    for line in text.split('\n'):
        if 'cargo run' in line and '--example' not in line:
            line = re.sub(r'-p horizon-client(?![\w-])', '-p horizon-game', line)
        if '--example' in line and any(name in line for name in ['native_waypoints', 'native_gui_overlay_probe', 'wind_burst_hook_probe']):
            line = re.sub(r'-p horizon-client(?![\w-])', '-p horizon-mod-host', line)
        new.append(line)
    updated = '\n'.join(new)
    if updated != text:
        p.write_text(updated)
