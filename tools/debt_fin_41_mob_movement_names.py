#!/usr/bin/env python3
"""Name the borrowed mob landing slot for the whole suspended move."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"
RENAMES = {
    "MobTrample": "MobMovement",
    "TramplePhase": "MovementPhase",
    "resume_after_trample": "resume_movement",
    "server_ai_until_trample": "server_ai_until_movement_pause",
    "pause_trample": "pause_movement",
}
FILES = (
    "mobs.rs",
    "mobs/ai.rs",
    "mobs/living.rs",
    "mobs/neutral_anger.rs",
    "mobs/riders.rs",
    "game/mobs/creatures/borrowed_tick.rs",
    "game/native_hooks/farmland_trample.rs",
)

for name in FILES:
    path = ROOT / name
    original = path.read_text(encoding="utf-8")
    updated = re.sub(r"\b(?:" + "|".join(RENAMES) + r")\b", lambda match: RENAMES[match[0]], original)
    if name.endswith(("/ai.rs", "/neutral_anger.rs", "/riders.rs", "mobs.rs")):
        # These spellings are only the borrowed movement slot, not a block decision.
        updated = re.sub(r"\btrample\b(?=(?:[.,);]| =|: (?:Option|&mut Option)))", "movement", updated)
    if name == "mobs/ai.rs":
        updated = updated.replace("/// decided movement,", "/// decided trample,")
    if name.endswith("/borrowed_tick.rs"):
        updated = re.sub(r"\btrample\b(?= = None|\.as_ref\(\)|,)", "movement", updated)
    if name.endswith("/farmland_trample.rs"):
        updated = updated.replace("let mut trample = None;", "let mut movement = None;")
        updated = updated.replace("&mut trample,", "&mut movement,")
        updated = updated.replace("assert!(trample.is_none());", "assert!(movement.is_none());")
    if updated != original:
        path.write_text(updated, encoding="utf-8")
