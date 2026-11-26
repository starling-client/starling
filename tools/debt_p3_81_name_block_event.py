#!/usr/bin/env python3
"""Rename the advancement loot context slot before giving it the event state."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"
for name in (
    "advancements.rs",
    "game/administration/commands/compute.rs",
    "game/blocks/machines/crafter.rs",
    "game/interface/advancements/mod.rs",
    "game/mobs/zombie_villagers.rs",
    "game/modding/native_criteria.rs",
):
    path = ROOT / name
    text = path.read_text()
    text = re.sub(r"^(\s+(?:pub )?)block_pos:(?= None| Some| Option<\[i32; 3\]>,\n    /// `LootContextParams)",
                  r"\1event_block:", text, flags=re.MULTILINE)
    path.write_text(text.replace("ctx.block_pos", "ctx.event_block"))
