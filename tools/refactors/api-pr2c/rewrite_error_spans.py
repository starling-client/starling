#!/usr/bin/env python3
"""Rewrite missing internal method names only at rustc's primary error spans.

Run after renaming definitions and cargo check --message-format=json. Recheck
and repeat until clean. Every replacement is recorded for the moved-lines proof.
"""
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
NAMES = dict(row.split('\t')[:2] for row in (HERE / 'internal-renames.tsv').read_text().splitlines())
NAMES.update(dict(row.split('\t') for row in (HERE / 'forwarder-renames.tsv').read_text().splitlines()))
edits = defaultdict(set)
for line in Path(sys.argv[1]).read_text().splitlines():
    message = json.loads(line)
    if message.get('reason') != 'compiler-message':
        continue
    message = message['message']
    if message['level'] != 'error':
        continue
    match = re.search(r'(?:method|associated item|function or associated item|associated function or constant) (?:named )?`(\w+)`', message['message'])
    if not match or match[1] not in NAMES:
        continue
    old = match[1]
    for span in message['spans']:
        if not span['is_primary']:
            continue
        path = Path(span['file_name'])
        if path.is_absolute():
            path = path.relative_to(HERE.parents[2])
        if not str(path).startswith('crates/'):
            continue
        start, end = span['byte_start'], span['byte_end']
        source = path.read_bytes()
        if source[start:end].decode() != old:
            continue
        edits[path].add((start, end, old, NAMES[old]))
records = []
for path, changes in edits.items():
    source = path.read_bytes()
    for start, end, old, new in sorted(changes, reverse=True):
        assert source[start:end].decode() == old
        records.append({'path': str(path), 'start': start, 'end': end, 'old': old, 'new': new})
        source = source[:start] + new.encode() + source[end:]
    path.write_bytes(source)
with (HERE / 'compiler-edits.jsonl').open('a') as log:
    for record in records:
        log.write(json.dumps(record, sort_keys=True) + '\n')
print(f'Rewrote {len(records)} compiler-identified method references in {len(edits)} files.')
