#!/usr/bin/env python3
"""Route the launcher's existing mod-build calls through the platform facade."""
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parents[2]
for path in (root / 'launcher/src').rglob('*.rs'):
    text = path.read_text()
    updated = text.replace('horizon_mod_api_gen::', 'horizon_platform::mod_build::api_gen::')
    updated = re.sub(r'(?<![\w:])point_shape::', 'horizon_platform::mod_build::point_shape::', updated)
    if updated != text:
        path.write_text(updated)
        subprocess.run(['rustfmt', '--edition', '2021', str(path)], check=True)
