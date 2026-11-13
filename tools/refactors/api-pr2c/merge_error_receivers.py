#!/usr/bin/env python3
"""Rewrite receivers of rustc type errors introduced by newly merged callers.

The primary span is sometimes the argument or field following the method, not
its identifier. Inspect only that line (or its immediately preceding receiver).
"""
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).parent
edits = defaultdict(set)
for line in Path(sys.argv[1]).read_text().splitlines():
    d = json.loads(line)
    if d.get('reason') != 'compiler-message':
        continue
    error = d['message']
    if error['level'] != 'error':
        continue
    if error['message'] == 'no field `list` on type `Vec<mod_api::EnchantmentInfo>`':
        old, new, lookback = 'enchantments', 'active_enchantment_definitions', 2
    elif error['message'] == 'the type `[mod_api::EnchantmentInfo]` cannot be indexed by `&str`':
        old, new, lookback = 'enchantments', 'active_enchantment_definitions', 0
    elif error['message'] == 'mismatched types' and 'expected `EntityRef`, found `Target`' in error['rendered']:
        old, new, lookback = 'entity_nbt', 'command_target_nbt', 0
    else:
        continue
    for span in error['spans']:
        if not span['is_primary']:
            continue
        path = Path(span['file_name'])
        if path.is_absolute():
            path = path.relative_to(ROOT)
        source = path.read_bytes()
        lines = source.splitlines(keepends=True)
        start = sum(map(len, lines[:max(0, span['line_start'] - 1 - lookback)]))
        end = sum(map(len, lines[:span['line_start']]))
        matches = list(re.finditer(rb'\b' + old.encode() + rb'(?=\s*\()', source[start:end]))
        assert len(matches) == 1, (path, span, matches)
        m = matches[0]
        edits[path].add((start + m.start(), start + m.end(), old, new))
records = []
for path, changes in edits.items():
    source = path.read_bytes()
    for start, end, old, new in sorted(changes, reverse=True):
        assert source[start:end].decode() == old
        source = source[:start] + new.encode() + source[end:]
        records.append(dict(path=str(path), start=start, end=end, old=old, new=new))
    path.write_bytes(source)
with (HERE / 'compiler-edits.jsonl').open('a') as log:
    for record in records:
        log.write(json.dumps(record, sort_keys=True) + '\n')
print(f'Rewrote {len(records)} compiler-identified receiver method spans.')
