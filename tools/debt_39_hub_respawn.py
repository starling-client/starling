#!/usr/bin/env python3
"""Migrate direct Hub respawn tests to the separate remove and place lifecycle."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LAN = ROOT / "crates/sunrise-client/src/lan"
for filename in (
    "hub.rs",
    "hub/tests/collision_exclusion_tests.rs",
    "hub/tests/pending_move_scope_tests.rs",
):
    path = LAN / filename
    lines = path.read_text().splitlines(keepends=True)
    for i in reversed(range(len(lines))):
        line = lines[i]
        if "hub.respawn(0," in line:
            indent = line[: len(line) - len(line.lstrip())]
            lines.insert(i, f"{indent}hub.remove_player_for_respawn(0);\n")
    path.write_text("".join(lines))
