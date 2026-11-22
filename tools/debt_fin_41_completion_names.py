#!/usr/bin/env python3
"""Rename the engine-only hit carrier and mob death entry points for item 41."""

from pathlib import Path
import re


root = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"
completion_paths = (
    "game/items/totem.rs",
    "game/mobs/creatures/mod.rs",
    "game/network/lan_actions/guest_body.rs",
    "game/network/lan_actions/turns.rs",
    "health.rs",
    "hunger.rs",
    "mobs.rs",
    "mobs/ai.rs",
    "mobs/living.rs",
    "motion.rs",
)
death_paths = (
    "game/administration/commands/mod.rs",
    "game/mobs/creatures/death_loot_context_tests.rs",
    "game/mobs/creatures/death_weapon_review_tests.rs",
    "game/mobs/creatures/death_weapon_tests.rs",
    "game/mobs/creatures/enchanted_entity_loot_tests.rs",
    "game/mobs/creatures/general_creature_tests.rs",
    "game/mobs/creatures/living_drops_tests.rs",
    "game/mobs/creatures/mod.rs",
    "game/mobs/piglin_admire_tests.rs",
    "game/modding/item_drop_state_tests.rs",
    "game/modding/native_damage_tests.rs",
    "game/modding/native_shield_totem_tests.rs",
    "game/native_hooks/living_death_post.rs",
)

for name in sorted(set(completion_paths + death_paths)):
    path = root / name
    text = path.read_text()
    replacements = {}
    if name in completion_paths:
        replacements["pending_damage_post"] = "damage_completion"
    if name in death_paths:
        replacements.update(mob_died="mob_die", mob_die="apply_mob_death")
    words = re.compile(r"\b(?:" + "|".join(replacements) + r")\b")
    updated = words.sub(lambda match: replacements[match[0]], text)
    if text != updated:
        path.write_text(updated)
