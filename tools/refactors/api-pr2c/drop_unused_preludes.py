#!/usr/bin/env python3
"""Remove only the unused prelude globs named by rustc (no fix-it is supplied).

The split's general drop_unused_imports.py handles fix-its; unused glob imports
have only a primary span. Retain the exact removed blocks for the move proof.
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
paths = set()
for line in Path(sys.argv[1]).read_text().splitlines():
    d = json.loads(line)
    if d.get('reason') != 'compiler-message':
        continue
    m = d['message']
    code = m.get('code')
    if code and code['code'] == 'unused_imports' and 'mod_api::prelude' in m['message']:
        paths.update(Path(s['file_name']) for s in m['spans'] if s['is_primary'])
records = []
for path in sorted(paths):
    relative = path.relative_to(ROOT) if path.is_absolute() else path
    path = ROOT / relative
    lines = path.read_text().splitlines(keepends=True)
    matches = [i for i, line in enumerate(lines) if line.strip() == 'use mod_api::prelude::*;']
    assert len(matches) == 1, (relative, matches)
    end = matches[0] + 1
    start = matches[0]
    if start and lines[start-1].strip() == '#[cfg(test)]':
        start -= 1
    records.append(dict(path=str(relative), old=''.join(lines[start:end])))
    path.write_text(''.join(lines[:start] + lines[end:]))
    print('cleaned', relative)
Path(__file__).with_name('unused-import-edits.json').write_text(json.dumps(records, indent=2) + '\n')
