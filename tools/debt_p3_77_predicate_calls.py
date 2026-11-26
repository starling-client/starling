#!/usr/bin/env python3
"""Migrate predicate call sites on the P3-77 review baseline (9f48fa499e4).

Constructors take only the target and constraints after the follow-up. Run once
before formatting the affected Rust files and applying the validation change.
"""

from pathlib import Path
import re


root = Path(__file__).resolve().parents[1]
files = {
    "crates/sunrise-client/src/game/blocks/block_edits.rs": (2, 0),
    "crates/sunrise-client/src/game/modding/api_values/block_edit.rs": (3, 0),
    "crates/sunrise-client/src/game/native_hooks/extend_poi_types.rs": (4, 1),
    "crates/sunrise-mod-vocab/src/state_predicate/tests.rs": (5, 2),
}

for path, expected in files.items():
    file = root / path
    source, blocks = re.subn(
        r"StatePredicate::block\(\s*(?:&?metadata|block_metadata_table\(\)\.unwrap\(\)),\s*",
        "StatePredicate::block(",
        file.read_text(),
    )
    source, tags = re.subn(
        r"StatePredicate::tag\(\s*&?metadata,\s*&Tags(?:Available)?,\s*",
        "StatePredicate::tag(",
        source,
    )
    if (blocks, tags) != expected:
        raise SystemExit(f"{path}: expected {expected} calls, found {(blocks, tags)}")
    file.write_text(source)
