#!/usr/bin/env python3
"""Write the declared internal collision table from the compiler-parsed audit.

The public AttachmentHost::attachments remains unchanged: it is not private.
"""
import re
from pathlib import Path

NAMES = {
    "block_face_sprite": "falling_block_face_sprite",
    "block_in_tag": "named_block_in_tag",
    "container_size": "open_container_size",
    "dispense_default": "dispense_default_in_level",
    "edit_blocks": "apply_block_edit",
    "enchantments": "active_enchantment_definitions",
    "entity_glowing": "tracked_entity_glowing",
    "entity_nbt": "command_target_nbt",
    "entity_uuid": "hover_event_entity_uuid",
    "give": "add_stack_to_inventory",
    "menu_click": "click_open_inventory",
    "menu_slot": "inventory_screen_slot",
    "mob_can_use_portal": "mob_portal_eligible",
    "open_menu": "open_block_effect_menu",
    "place_recipe": "place_book_recipe",
    "playing": "require_playing_screen",
    "raid_living_health": "remaining_raider_health",
    "read_columns": "read_loaded_block_columns",
    "send_chat": "send_signed_chat_input",
    "set_camera_type": "apply_camera_type",
    "summon": "summon_command_mob",
    "surface_heights": "loaded_column_heights",
    "tick_stats": "tick_player_stat_counters",
    "transaction_close": "close_storage_transaction",
    "transaction_open": "open_storage_transaction",
    "translate": "translate_with_fallback",
    "open_chat": "begin_chat_screen",
}

if __name__ == "__main__":
    import subprocess
    import sys
    only = sys.argv[2] if len(sys.argv) == 3 and sys.argv[1] == '--only' else None
    parsed = subprocess.check_output(
        ["cargo", "run", "--quiet", "-p", "horizon-mod-api-gen", "--example", "audit_game_methods"], text=True,
    )
    definitions = [line.split("\t") for line in parsed.splitlines()]
    table = Path(__file__).with_name("internal-renames.tsv")
    rows = [row for row in table.read_text().splitlines() if row.split('\t')[0] != only] if only and table.exists() else []
    for old, new in NAMES.items():
        if only and old != only:
            continue
        assert not any(row[0] == new for row in definitions), new
        matches = [row for row in definitions if row[0] == old and row[3] == "inherent"]
        assert len(matches) == 1, (old, matches)
        _, path, line, _ = matches[0]
        rows.append(f"{old}\t{new}\t{path}:{line}")
        source = Path(path).read_text()
        lines = source.splitlines(keepends=True)
        assert re.search(r"\bfn " + old + r"\b", lines[int(line) - 1])
        lines[int(line) - 1] = re.sub(r"\bfn " + old + r"\b", "fn " + new, lines[int(line) - 1])
        Path(path).write_text("".join(lines))
    table.write_text("\n".join(rows) + "\n")
