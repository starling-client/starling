#!/usr/bin/env python3
"""Thread the logical sky pass flag through renderer frame literals and captures."""

import re
import subprocess
from pathlib import Path


pattern = re.compile(r"^(?P<indent> +)(?P<decl>pub )?sky_blocked: (?P<value>[^\n]+)$", re.M)
values = {
    "bool,": "bool,",
    "false,": "false,",
    "frame.sky_blocked,": "frame.sky_pass,",
    "f.sky_blocked,": "f.sky_pass,",
    "sunrise_client::fog::sky_pass_skipped(": "false,",
}
paths = subprocess.check_output(
    ["git", "grep", "-l", "sky_blocked:", "--", "*.rs"], text=True
).splitlines()
for name in paths:
    path = Path(name)
    source = path.read_text()
    if re.search(r"^ +(?:pub )?sky_pass:", source, re.M):
        continue
    updated = pattern.sub(
        lambda m: (
            f"{m['indent']}{m['decl'] or ''}sky_pass: {values[m['value']]}\n{m[0]}"
        ),
        source,
    )
    assert updated != source, path
    path.write_text(updated)
