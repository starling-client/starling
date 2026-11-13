#!/usr/bin/env python3
"""Applies rustc's own `unused_imports` suggestions, for the files in the PR only.

The import rewrite of the split (`import_prelude.py`, the game forwarders' `use mod_api::{prelude::*, *}`)
left imports a file does not use. Feed it `cargo check -p <crate> --lib --profile test --message-format=json`
(the test build sees every use) and the list of files to touch (one repo-relative path per line, argv[1]).
"""
import json
import sys
from collections import defaultdict
from pathlib import Path

allowed = set(Path(sys.argv[1]).read_text().split())
edits = defaultdict(set)
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("reason") != "compiler-message":
        continue
    d = message["message"]
    if not d.get("code") or d["code"]["code"] != "unused_imports":
        continue
    for child in d["children"]:
        for span in child["spans"]:
            if span.get("suggested_replacement") is not None and span["file_name"] in allowed:
                edits[span["file_name"]].add((span["byte_start"], span["byte_end"], span["suggested_replacement"]))
for name, spans in edits.items():
    data = Path(name).read_bytes()
    for start, end, text in sorted(spans, reverse=True):
        data = data[:start] + text.encode() + data[end:]
    Path(name).write_bytes(data)
    print("cleaned", name)
