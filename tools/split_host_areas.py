#!/usr/bin/env python3
"""Split the host areas `levels`, `entities` and `players` by concern.

Their backend traits (`LevelsBackend`, `LevelsBackendMut`, `EntitiesBackend`,
`EntitiesBackendMut`, `PlayersBackend`, `PlayersBackendMut`) held 55 to 92 methods each. Every
method moves, verbatim with its docs and attributes, into the area file of its concern under
`crates/sunrise-mod-vocab/src/host/`: a file `level_blocks.rs` holds `LevelBlocksBackend` (the
reads) and `LevelBlocksBackendMut` (the writes). The `Host`, `ServerHost`, `ClientHost` and client
traits of the three files stay where they are. Each method keeps its tier.

    python3 tools/split_host_areas.py          # move the methods
    python3 tools/split_host_areas.py --table  # print the split as a markdown table
"""

import re
import sys
from pathlib import Path

HOST = Path(__file__).resolve().parent.parent / "crates/sunrise-mod-vocab/src/host"

# source file stem -> (method prefix, trait prefix)
SOURCES = {"levels": "level_", "entities": "entity_", "players": "player_"}

# The name of a method and the concern it moves to: area stem -> (job, methods).
GROUPS = {
    "level_blocks": ("Reading and writing the block at a position.", """
        level_block_state level_block level_block_property level_block_transform level_block_break_speed
        level_set_block level_update_from_neighbour_shapes level_place_block level_remove_block
        level_destroy_block level_block_drops level_bone_meal"""),
    "level_block_updates": ("Telling neighbours, clients and capability caches that a block changed.", """
        level_send_block_updated level_set_break_progress level_block_event level_update_neighbors
        level_neighbor_changed level_capability_generation level_invalidate_capabilities"""),
    "level_collision": ("Block shapes, collision and rays through them.", """
        level_block_floor_height level_block_shape level_block_face_sturdy level_collision_boxes
        level_has_collisions level_clip level_line_of_sight"""),
    "level_fluids": ("Fluid states, shapes, heights and their flow step.", """
        level_fluid_flow level_fluid_state level_fluid_shape level_fluid_tick_delay level_fluid_height
        level_fluid_height_for_camera level_fluid_can_be_replaced_with level_fluid_tick"""),
    "level_signals": ("Redstone power at a position, and whether a block conducts it.", """
        level_signal level_direct_signal level_direct_signal_to level_best_neighbor_signal
        level_has_neighbor_signal level_is_redstone_conductor"""),
    "level_light": ("Stored and sky light, and relighting.", """
        level_get_sky_darken level_brightness level_can_see_sky level_check_light level_relight_chunks"""),
    "level_chunks": ("Chunks of an open level: handles, status, loading and resending.", """
        level_chunk_ref chunk_loaded chunk_status chunk_is_slime_chunk chunk_watchers level_is_loaded
        level_loaded_chunks chunk_resend"""),
    "level_tickets": ("What keeps chunks and watered areas active: forced chunks and tickets.", """
        level_has_water_ticket level_add_water_ticket water_ticket_remove chunk_forced level_forced_chunks
        level_mod_forced_chunks chunk_set_forced chunk_set_mod_forced"""),
    "level_ticking": ("What the level ticks now, and running a tick by hand.", """
        level_in_place level_entity_ticking level_should_tick_blocks_at level_random level_set_random
        level_block_tick level_block_random_tick"""),
    "level_time": ("The level's game, day and clock time.", """
        level_game_time level_day_time level_clock_time level_clock_marker_time level_set_clock_time"""),
    "level_weather": ("Rain, thunder and lightning.", """
        level_weather level_is_raining level_is_raining_at level_is_thundering level_set_weather
        level_set_weather_levels level_lightning_target level_strike_lightning"""),
    "level_worldgen": ("Biomes, structures and the seed the generator uses.", """
        level_biome_at level_client_biome level_find_closest_biome level_find_nearest_structure
        level_biome_spawners level_biome_effects level_biome_in_tag level_biome_properties level_set_biome
        level_place_feature level_place_placed_feature level_seed"""),
    "level_datapacks": ("Data-driven lookups: datapack registry entries, recipe matching and loot tables.", """
        level_datapack_entries level_datapack_handle level_datapack_read level_recipe_for level_roll_loot"""),
    "level_rules": ("Game rules, difficulty and the features a level enables.", """
        level_game_rule_bool level_game_rule_int level_game_rule level_game_rules level_set_game_rule
        level_difficulty level_local_difficulty level_enabled_features"""),
    "level_saved_data": ("Data the world saves: raids, maps, the respawn data, and saving itself.", """
        level_raid level_raid_at level_raids level_map_data level_set_map_data level_respawn_data
        level_set_respawn_data level_request_save"""),
    "level_entities": ("Entities and players in a level, and what a level spawns as items and experience.", """
        level_all_entities level_entities_in level_ray_trace_entities level_online_players level_pop_resource
        level_spawn_item level_spawn_item_with_motion level_award_experience"""),
    "level_spawning": ("Natural spawning limits and flags, and spawning a mob.", """
        level_spawn_limits level_spawn_flags level_can_spawn_entities_in_chunk level_set_spawn_limits
        level_set_spawn_flags level_spawn_mob"""),
    "level_effects": ("Sounds, particles, level and game events, and explosions.", """
        level_play_sound level_send_particles level_level_event level_block_break_effect level_global_level_event
        level_game_event
        level_explode level_explosion_resistance level_explosion_resistances level_explosion_destroy_blocks"""),
    "level_world_border": ("The level's world border.", """
        level_world_border level_set_world_border_center level_set_world_border_size
        level_set_world_border_damage level_set_world_border_warning"""),
    "level_dimension": ("The dimension a level is: its key, type, height, bounds and passages.", """
        dimension_key_id level_dimension_type level_dimension_shape level_logical_height level_height
        level_sea_level level_in_world_bounds level_in_spawnable_bounds level_register_passage
        level_remove_passage level_portal_destination level_find_or_create_portal_exit"""),

    "entity_identity": ("What an entity is: its name, type, level and UUID, and the players it is sent to.", """
        entity_name entity_custom_name_visible entity_classification entity_kind entity_level
        entity_is_multipart entity_parts entity_sound_source entity_set_custom_name
        entity_set_custom_name_visible entity_uuid entity_trackers"""),
    "entity_lifetime": ("How long an entity has lived, and removing it.", """
        entity_tick_count entity_added_to_level entity_portal_cooldown entity_set_portal_cooldown
        entity_discard entity_remove"""),
    "entity_data": ("An entity's tags and saved data.", """
        entity_tags entity_nbt entity_merge_nbt entity_add_tag entity_remove_tag"""),
    "entity_actions": ("Doing something as an entity: commands, sounds and events.", """
        entity_run_command entity_run_command_with_output entity_spawn_at_location entity_play_sound
        entity_play_bound_sound entity_game_event entity_broadcast_event"""),
    "entity_position": ("Where an entity is and which way it faces, and moving it between places.", """
        entity_position entity_block_position entity_rotation entity_on_ground entity_on_block
        entity_distance_to_sqr entity_head_yaw entity_body_yaw entity_direction entity_teleport_to entity_teleport_to_level
        entity_move_by entity_set_rotation entity_set_head_yaw entity_set_hanging_direction entity_set_item_frame_rotation"""),
    "entity_sight": ("What an entity sees: eyes, view and picking.", """
        entity_eye_height entity_eye_position entity_view_vector entity_up_vector entity_pick"""),
    "entity_physics": ("An entity's box, motion and collision.", """
        entity_velocity entity_bounding_box entity_dimensions entity_no_physics entity_set_velocity entity_push
        entity_set_collision_exclusions entity_set_no_physics entity_reset_fall_distance
        entity_collider entity_set_collider entity_set_collider_transform"""),
    "entity_damage": ("Invulnerability, damage cooldowns and hurting an entity.", """
        entity_permanently_invulnerable entity_invulnerable_time entity_damage_cooldown_time
        entity_invulnerable_to_damage entity_entities_team_damage_allowed
        entity_invulnerable_to_piercing_weapon entity_set_invulnerable entity_set_damage_cooldown_time
        entity_set_invulnerable_time entity_hurt"""),
    "entity_riding": ("Vehicles and passengers.", """
        entity_vehicle entity_is_passenger entity_root_vehicle entity_passengers entity_is_vehicle
        entity_indirect_passengers entity_controlling_passenger entity_is_passenger_of_same_vehicle
        entity_dismount_location entity_start_riding entity_ride_player entity_stop_riding
        entity_eject_passengers"""),
    "entity_leash": ("Leads between an entity and its holder.", """
        entity_leash_holder entity_leash_to entity_leash_to_knot entity_detach_leash"""),
    "entity_flags": ("An entity's persistent flags: silent, glowing, no gravity, invisible.", """
        entity_is_silent entity_glowing entity_no_gravity entity_is_invisible entity_set_silent
        entity_set_glowing entity_set_no_gravity entity_set_invisible"""),
    "entity_stance": ("How an entity holds itself: spectating, crouching, sprinting, swimming, its pose.", """
        entity_spectator entity_is_shift_key_down entity_is_crouching entity_is_sprinting entity_set_sprinting entity_is_swimming entity_pose"""),
    "entity_air_fluids": ("The fluids an entity touches, and its air.", """
        entity_fluid_heights entity_eye_fluid entity_is_in_water entity_is_in_lava entity_air_supply
        entity_max_air_supply entity_set_air_supply entity_is_under_water"""),
    "entity_fire_frost": ("Burning and freezing.", """
        entity_fire_immune entity_fire_ticks entity_ticks_frozen entity_set_remaining_fire_ticks
        entity_clear_fire entity_extinguish_fire entity_set_ticks_frozen entity_is_on_fire"""),

    "player_original": ("The player a clone hook copies from: what it was before the data copy.", """
        original_player_uuid original_player_position original_player_dimension original_player_level
        original_player_inventory_slot original_player_health original_player_experience
        original_player_effects"""),
    "player_session": ("Who a player is and how it is connected: name, profile, address, permission, persistent data.", """
        player_address player_name player_profile player_permission_level player_text_filtering
        player_client_brand player_announced_channels player_sees player_persistent_data player_disconnect
        player_set_nickname player_hide player_set_persistent_data"""),
    "player_messages": ("Chat, action bar, tab list, sounds and maps sent to one player.", """
        player_tell player_action_bar player_tab_list player_send_map player_stop_sound player_play_sound"""),
    "player_world": ("Where a player is, where it respawns, and moving it between places.", """
        player_level player_dimension player_last_death_location player_respawn_config
        player_set_respawn_config player_set_last_death_location player_set_chunk_areas player_simulate_move
        player_respawn"""),
    "player_interaction": ("What a player does to blocks and entities: range, permission, breaking and placing.", """
        player_attack_strength_scale player_may_use_item_at player_can_harm player_within_entity_reach
        player_destroy_block player_has_correct_tool_for_drops player_ack_block_changes player_show_block player_place_block"""),
    "player_inventory": ("A player's inventory, ender chest and selected slot.", """
        player_selected_slot player_inventory player_inventory_slots player_ender_chest
        player_set_selected_slot player_drop_stack player_set_inventory_slot player_set_ender_chest_slot
        player_give_stack"""),
    "player_menus": ("The menu a player has open.", """
        player_container_menu player_open_menu player_open_mod_menu player_back_mod_menu_slots
        player_open_ender_chest"""),
    "player_cooldowns": ("Item cooldowns and the warden warning.", """
        player_item_cooldowns player_has_item_cooldown player_item_cooldown_percent player_set_item_cooldown
        player_remove_item_cooldown player_set_warden_warning_cooldown"""),
    "player_food": ("Food level, saturation and exhaustion.", """
        player_needs_food player_has_enough_food player_can_eat player_food player_feed player_add_exhaustion
        player_set_food_level player_set_saturation player_set_exhaustion"""),
    "player_experience": ("Experience points and levels.", """
        player_exp_cooldown player_experience player_xp_needed_for_next_level player_set_exp_cooldown
        player_set_experience_points player_set_experience_levels player_set_total_experience
        player_give_experience_points player_give_levels"""),
    "player_modes": ("Game mode and abilities, and what they allow.", """
        player_game_mode player_is_creative player_is_spectator player_may_build player_abilities
        player_may_fly player_has_infinite_materials player_can_use_game_master_blocks
        player_previous_game_mode player_is_secondary_use_active player_set_game_mode player_set_abilities"""),
    "player_recipes": ("The recipe book: discovered recipes and custom book settings.", """
        player_recipe_book_settings player_has_discovered_recipe player_discovered_recipes
        player_set_recipe_book_settings player_award_recipes player_reset_recipes"""),
    "player_progress": ("Statistics and criteria triggers.", """
        player_stat player_award_stat player_reset_stat player_trigger_criterion player_trigger_block_use
        player_trigger_item_used_on_block player_trigger_consume_item"""),
    "player_sleep": ("Sleeping: the timer and the night's sleep count.", """
        player_sleep_ignored player_counts_for_sleep player_sleep_timer player_set_sleep_ignored
        player_set_sleep_timer"""),
    "player_fakes": ("Connectionless fake players and the actions they perform.", """
        player_is_fake player_perform_action player_perform_action_scoped player_remove_fake"""),
}

# The old backend traits, by tier, that these methods leave.
TIERS = ("Backend", "BackendMut")


def camel(stem):
    return "".join(word.capitalize() for word in stem.split("_"))


def split_trait(lines, name):
    """The lines of `pub trait <name>` from its docs to its closing brace, with the start line."""
    start = next(i for i, line in enumerate(lines) if line.startswith(f"pub trait {name} {{"))
    first = start
    while first > 0 and lines[first - 1].startswith(("///", "//", "#[")):
        first -= 1
    end = next(i for i in range(start, len(lines)) if lines[i] == "}")
    return first, start, end


def methods(lines, start, end):
    """The method chunks of a trait body (docs, attributes and signature), in order."""
    chunks, current, depth = [], [], 0
    for line in lines[start + 1 : end]:
        current.append(line)
        code = line.strip()
        if code.startswith("//"):
            continue
        depth += sum(code.count(c) for c in "([{") - sum(code.count(c) for c in ")]}")
        if depth == 0 and code.endswith(";") and not code.startswith("#["):
            chunks.append(current)
            current = []
    assert not current, f"trailing lines in trait: {current}"
    return chunks


def method_name(chunk):
    for line in chunk:
        found = re.match(r"\s+fn (\w+)", line)
        if found:
            return found.group(1)
    raise AssertionError(chunk)


def owner():
    seen = {}
    for stem, (_, names) in GROUPS.items():
        for name in names.split():
            assert name not in seen, f"{name} is in {seen[name]} and {stem}"
            seen[name] = stem
    return seen


def check():
    """Refuse to run while a method of the old traits has no concern, or a named one is gone."""
    owners = owner()
    unassigned = []
    for source in SOURCES:
        lines = (HOST / f"{source}.rs").read_text().split("\n")
        for tier in TIERS:
            name = f"{camel(source)}{tier}"
            _, start, end = split_trait(lines, name)
            for method in (method_name(c) for c in methods(lines, start, end)):
                if owners.pop(method, None) is None:
                    unassigned.append(f"{method} ({name})")
    if unassigned:
        sys.exit(f"no concern for: {', '.join(unassigned)}; add each to a group in GROUPS")
    if owners:
        sys.exit(f"named in GROUPS but not in the traits: {sorted(owners)}")


def table():
    check()
    owners = owner()
    for source in SOURCES:
        lines = (HOST / f"{source}.rs").read_text().split("\n")
        old = {}
        for tier in TIERS:
            name = f"{camel(source)}{tier}"
            _, start, end = split_trait(lines, name)
            old[name] = [method_name(c) for c in methods(lines, start, end)]
            for method in old[name]:
                assert owners.pop(method, None), f"{method} of {name} has no concern"
        prefix = SOURCES[source]
        print(f"### `{source}`\n")
        print("| Area file | Reads | Writes | Methods | Job |\n|---|---|---|---|---|")
        for stem, (job, names) in GROUPS.items():
            if not stem.startswith(prefix):
                continue
            names = names.split()
            reads = sum(n in old[f"{camel(source)}Backend"] for n in names)
            writes = sum(n in old[f"{camel(source)}BackendMut"] for n in names)
            print(f"| `{stem}` | `{camel(stem)}Backend` {reads} | `{camel(stem)}BackendMut` {writes} | {reads + writes} | {job} |")
        print()
    assert not owners, f"named but not found: {sorted(owners)}"


def main():
    if "--table" in sys.argv:
        return table()
    check()
    owners = owner()
    for source, prefix in SOURCES.items():
        path = HOST / f"{source}.rs"
        lines = path.read_text().split("\n")
        header = lines[:3]
        moved = {}  # stem -> tier -> chunks
        removed = []
        for tier in TIERS:
            name = f"{camel(source)}{tier}"
            first, start, end = split_trait(lines, name)
            for chunk in methods(lines, start, end):
                target = owners.pop(method_name(chunk), None)
                assert target, f"{method_name(chunk)} of {name} has no concern"
                moved.setdefault(target, {}).setdefault(tier, []).extend(chunk)
            removed.append((first, end))
        for first, end in sorted(removed, reverse=True):
            del lines[first : end + 1]
        path.write_text("\n".join(lines))
        for stem, tiers in moved.items():
            job = GROUPS[stem][0]
            out = [
                f"//! The `{stem}` area: {job[0].lower()}{job[1:]} Host method declarations; access defaults and game forwarders are generated.",
                header[1],
                header[2],
            ]
            for tier in TIERS:
                if tier not in tiers:
                    continue
                union, word = ("Backend", "reads") if tier == "Backend" else ("BackendMut", "writes")
                out += [
                    f"/// The handles' {word} of the `{stem}` area, implemented by the game and reached through [`{union}`].",
                    "// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait",
                    "#[doc(hidden)]",
                    "#[sunrise_mod_api_macros::host_area]",
                    f"pub trait {camel(stem)}{tier} {{",
                ]
                out += [line for i, line in enumerate(tiers[tier]) if line or any(tiers[tier][:i])]
                out.append("}")
            (HOST / f"{stem}.rs").write_text("\n".join(out) + "\n")
    assert not owners, f"named but not found: {sorted(owners)}"


main()
