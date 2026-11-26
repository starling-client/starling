#!/usr/bin/env python3
"""Compare Sunrise's command execution with vanilla's real command implementations.

Java (VanillaCommandExecOracle) starts a stand-in integrated server: a real
MinecraftServer with a generated world, Commands.CommandSelection.INTEGRATED
and one single-player owner named Player. Rust runs the same script through
Sunrise's integrated server with a connected headless client (SUNRISE_COMMAND_SCRIPT). Both run each command as
the player through Commands.performPrefixedCommand and record every system or
chat message the player receives (text and resolved style: colour, bold,
italic, underlined, strikethrough, obfuscated) together with the results the
command reported to the player's CommandResultCallback. World, entity and item
changes are observed through probe commands whose feedback reports them
(execute if, clear with a zero count, data get, scoreboard queries...).
Messages that depend on time or a random UUID are compared after masking.
Direct falling-block probes also compare the physical entities in each level,
independently of the command selector's support for those entity types.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base
from tools.integration_env import run_game
from tools import command_exec_corpus as corpus_module, seed_agent

SEED = 1


def install_datapack_fixtures(world, predicates, functions, jar, advancements=None, loot_tables=None, recipes=None):
    if not predicates and not functions and not advancements and not loot_tables and not recipes:
        return
    with zipfile.ZipFile(jar) as archive:
        version = json.loads(archive.read('version.json'))['pack_version']
    fmt = [version['data_major'], version['data_minor']]
    pack = world / 'datapacks' / 'command-selector-fixture'
    pack.mkdir(parents=True, exist_ok=True)
    (pack / 'pack.mcmeta').write_text(json.dumps({'pack': {
        'description': 'Command execution fixtures', 'pack_format': fmt[0],
        'min_format': fmt, 'max_format': fmt}}), encoding='utf-8')
    for identifier, predicate in predicates.items():
        namespace, path = identifier.split(':', 1)
        target = pack / 'data' / namespace / 'predicate' / (path + '.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(predicate), encoding='utf-8')
    for identifier, contents in functions.items():
        namespace, path = identifier.split(':', 1)
        target = pack / 'data' / namespace / 'function' / (path + '.mcfunction')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(contents, encoding='utf-8')

    for identifier, advancement in (advancements or {}).items():
        namespace, path = identifier.split(':', 1)
        target = pack / 'data' / namespace / 'advancement' / (path + '.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(advancement), encoding='utf-8')
    for identifier, table in (loot_tables or {}).items():
        namespace, path = identifier.split(':', 1)
        target = pack / 'data' / namespace / 'loot_table' / (path + '.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(table), encoding='utf-8')


    for identifier, recipe in (recipes or {}).items():
        namespace, path = identifier.split(':', 1)
        target = pack / 'data' / namespace / 'recipe' / (path + '.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(recipe), encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--function-admission-fixture', action='store_true', help='compile initial Java functions at its actual GAMEMASTER permission, matching reload and native')
    parser.add_argument('--raid-village-fixture', action='store_true', help='match the existing development command trees in the isolated Java/native fixture processes')
    parser.add_argument('--raid-release-command-diagnostic', action='store_true', help='retain the unmatched native release-tree admission prerequisite diagnostic')
    parser.add_argument('--advancement-loot-inline-parser-diagnostic', action='store_true',
                        help='retain all 100 inline observations, including the three missing-reference ACK differences')
    parser.add_argument('--advancement-loot-named-diagnostic', action='store_true',
                        help='retain the separate named world-loot registry-admission diagnostic')
    parser.add_argument('--pot-string-sherd-diagnostic', action='store_true',
                        help='use string sherd input in the pot data group (known separate codec gap)')
    parser.add_argument('--limit', type=int, default=40, help='print at most this many mismatches')
    parser.add_argument('--allow-gaps', action='store_true', help='report mismatches without failing')
    parser.add_argument('--wolf-anger-clock-diagnostic', action='store_true',
                        help='retain the separate loaded-NoAI absolute anger clock diagnostic')
    parser.add_argument('--vehicle-kill-diagnostic', action='store_true',
                        help='retain random drop UUID/motion and whole vehicle bodies outside the explicit-content gate')
    parser.add_argument('--only', help='run only corpus groups whose name contains this')
    parser.add_argument('--vehicle-container-diagnostic', action='store_true',
                        help='retain all vehicle Tags-list and whole-body observations outside the saved-field gate')
    parser.add_argument('--array-storage-diagnostic', action='store_true',
                        help='run all numeric-array edit cases, including the known wildcard/string storage side effects')
    parser.add_argument('--sulfur-power-load-diagnostic', action='store_true',
                        help='retain the immediate-load powered sulfur cube H249 diagnostic')
    parser.add_argument('--merchant-gate-fixture', action='store_true')
    parser.add_argument('--binary', type=Path, help='use an already built Sunrise game instead of building it')
    parser.add_argument('--trader-llama-discard-fixture', action='store_true',
                        help='an AI trader llama is gone after the tick its delay runs out (H182)')
    parser.add_argument('--hurt-cooldown-incoming-fixture', action='store_true',
                        help='a mob hurt cooldown compares the incoming damage before its reductions (H248)')
    parser.add_argument('--level-world-border-fixture', action='store_true',
                        help='each builtin level keeps its own world border (H260)')
    parser.add_argument('--slot-codec-diagnostic', action='store_true',
                        help='retain the separate invalid inline slot-range codec diagnostic')
    parser.add_argument('--work-dir', type=Path, help='isolated scratch output directory')
    parser.add_argument('--loaded-fixture', action='store_true', help='pin actual view/simulation2 and use real player tickets')
    parser.add_argument('--loaded-block-diagnostic', action='store_true', help='retain separate FULL block-access observations')
    parser.add_argument('--mob-quad-fixture', action='store_true', help='guarded live mob quad-leash scene')
    parser.add_argument('--mob-quad-raw-diagnostic', action='store_true', help='retain full-precision leash diagnostic')
    parser.add_argument('--item-merge-diagnostic', action='store_true',
                        help='retain the original two-tag and spectator-selector item-merge diagnostic')
    parser.add_argument('--orb-water-fixture', action='store_true', help='guarded orb water/air scene')
    parser.add_argument('--orb-water-raw-diagnostic', action='store_true', help='add whole Pos/Motion records')
    parser.add_argument('--orb-lava-fixture', action='store_true', help='guarded server orb lava contact scene')
    parser.add_argument('--falling-first-tick-fixture', action='store_true',
                        help='summoned falling blocks and TNT tick on their first tick, after a later orb too')
    parser.add_argument('--orb-lava-raw-diagnostic', action='store_true', help='retain random Pos/Motion as exact raw diagnostics')
    parser.add_argument('--orb-fire-fixture', action='store_true', help='server orb saved Fire and its burn, tick by tick')
    parser.add_argument('--lava-fire-fixture', action='store_true', help='lava sets Fire on projectiles, falling blocks and orbs')
    parser.add_argument('--mob-home-fixture', action='store_true')
    parser.add_argument('--mob-home-holder-removal-diagnostic', action='store_true')
    parser.add_argument('--mob-home-late-detach-control', action='store_true')
    parser.add_argument('--item-contents-fixture', action='store_true')
    parser.add_argument('--item-contents-provider-empty-control', action='store_true')
    parser.add_argument("--boat-item-hit-fixture", action="store_true")
    parser.add_argument("--stand-entity-collision-fixture", action="store_true")
    parser.add_argument("--reference-only", action="store_true")
    parser.add_argument("--record-summon-name-witness", type=Path, help="write the Java reference records as a version-tagged JSON witness")
    parser.add_argument("--boat-leash-view-fixture", action="store_true")
    parser.add_argument("--boat-motion-diagnostic", action="store_true")
    parser.add_argument('--reinforcement-original-diagnostic', action='store_true', help='replay the original unisolated174/108 fixture, retaining its terrain prerequisite failure')
    parser.add_argument('--reinforcement-target-diagnostic', action='store_true', help='retain the separate creative-callee target relation record (full209)')
    parser.add_argument('--reinforcement-raw-diagnostic', action='store_true', help='add unnormalized full callee snapshots; not the typed gate')
    parser.add_argument("--pufferfish-contact-fixture", action="store_true")
    parser.add_argument("--pufferfish-body-diagnostic", action="store_true")
    parser.add_argument('--moving-piston-history-diagnostic', action='store_true',
                        help='retain the full piston presentation/default-origin controls, including H191 insertion-history gaps')
    parser.add_argument("--armor-stand-honey-fixture", action="store_true")
    parser.add_argument("--pufferfish-mob-fixture", action="store_true")
    parser.add_argument("--pufferfish-mob-duration-diagnostic", action="store_true")
    parser.add_argument("--pufferfish-mob-overlap-control", action="store_true")
    parser.add_argument("--map-decoration-namespace-fixture", action="store_true")
    parser.add_argument("--map-decoration-empty-diagnostic", action="store_true")
    parser.add_argument('--piglin-anger-memory-fixture', action='store_true')
    parser.add_argument('--piglin-anger-raw-diagnostic', action='store_true')
    parser.add_argument('--piglin-anger-memory-diagnostic', action='store_true')
    parser.add_argument('--map-decoration-patch-fixture', action='store_true')
    parser.add_argument('--map-decoration-removal-diagnostic', action='store_true')
    parser.add_argument("--selector-bounds-fixture", action="store_true")
    parser.add_argument('--spectral-duration-fixture', action='store_true')
    parser.add_argument('--item-health-fixture', action='store_true')
    parser.add_argument("--macro-source-type-fixture", action="store_true")
    parser.add_argument("--stand-fall-fixture", action="store_true")
    parser.add_argument('--orb-uuid-fixture', action='store_true')
    parser.add_argument('--baby-modifier-fixture', action='store_true', help='/attribute add and remove of a zombie\'s transient minecraft:baby modifier')
    parser.add_argument('--baby-modifier-raw-diagnostic', action='store_true', help='add the whole attributes lists and entity data')
    parser.add_argument('--mob-attribute-save-fixture', action='store_true', help='which attribute instances a pig, allay and armor stand save before and after a tick')
    parser.add_argument('--mob-attribute-save-raw-diagnostic', action='store_true', help='add the whole attributes lists and entity data')
    parser.add_argument('--record-oxygen-bonus-witness', type=Path, help='record the vanilla underwater oxygen_bonus attribute witness JSON to this path')
    parser.add_argument('--mob-inventory-merge-fixture', action='store_true', help='Piglin and Villager saved inventories loaded as SimpleContainer.addItem does')
    parser.add_argument('--mob-inventory-merge-raw-diagnostic', action='store_true', help='add each mob\'s whole entity data')
    parser.add_argument('--summon-passengers-fixture', action='store_true', help='summon trees with Passengers entries vanilla cannot load')
    parser.add_argument('--summon-passengers-raw-diagnostic', action='store_true', help='add each parent\'s whole entity data')
    parser.add_argument('--summon-uuid-fixture', action='store_true', help='summon trees whose UUIDs conflict with live entities or within the tree')
    parser.add_argument('--summon-uuid-internal-diagnostic', action='store_true', help='add the trees with duplicate UUIDs inside them (#11807)')
    parser.add_argument('--summon-uuid-raw-diagnostic', action='store_true', help='add each member\'s whole entity data')
    parser.add_argument('--mob-absorption-max-fixture', action='store_true', help='a mob\'s max_absorption with Absorption added, upgraded, cleared and run out')
    parser.add_argument('--mob-absorption-max-ops-fixture', action='store_true', help='the Absorption modifier among other max_absorption operations and the range clamp')
    parser.add_argument('--mob-absorption-max-raw-diagnostic', action='store_true', help='add each mob\'s whole entity data')
    parser.add_argument('--mob-health-boost-fixture', action='store_true', help='Health Boost on a mob: heal to its maximum, clamp on removal')
    parser.add_argument('--mob-health-boost-raw-diagnostic', action='store_true', help='add each mob\'s whole entity data')
    parser.add_argument('--sulfur-body-attributes-fixture', action='store_true', help='a sulfur cube\'s body archetype attribute modifiers through /attribute')
    parser.add_argument('--sulfur-body-replace-fixture', action='store_true', help='a sulfur cube\'s body replaced and removed through /data')
    parser.add_argument('--sulfur-body-item-fixture', action='store_true', help='/item replace on a sulfur cube\'s armor.body')
    parser.add_argument('--sulfur-body-attributes-raw-diagnostic', action='store_true', help='add the whole attributes lists and entity data')
    parser.add_argument('--mob-absorption-fixture', action='store_true', help='a mob\'s Absorption effect once the absorption is used up')
    parser.add_argument('--mob-absorption-infinite-fixture', action='store_true', help='infinite Absorption used up, kept, and beside another infinite effect')
    parser.add_argument('--mob-absorption-raw-diagnostic', action='store_true', help='add each mob\'s whole entity data')
    parser.add_argument("--mob-pickup-save-fixture", action="store_true")
    parser.add_argument('--merchant-crafted-fixture', action='store_true')
    parser.add_argument('--mob-name-presentation-diagnostic', action='store_true',
                        help='retain all 312 mob-name records, including ten separate name/saved-text presentation observations')
    parser.add_argument('--potion-effect-icon-fixture', action='store_true')
    parser.add_argument('--unsynced-randoms', action='store_true',
                        help='leave out the sync_randoms steps: the control in which their fields differ run to run')
    parser.add_argument('--groups-file', type=Path,
                        help='run these generated [name, ops] groups after the common setup instead of the corpus')
    parser.add_argument('--record-eye-item-witness', nargs='?',
                        const=str(lab.LOCAL / 'witness' / lab.VERSION / 'eye_item' / 'eye_item.tsv'),
                        help='run the 26.3 Java oracle on summoned eye of ender and write the eye Item witness TSV into .local')
    parser.add_argument('--record-shulker-motion', nargs='?',
                        const=lab.LOCAL / 'witness' / lab.VERSION / 'shulker_motion' / 'shulker_motion.json',
                        type=Path, help='record the shulker motion/yaw witness into .local from VanillaCommandExecOracle only')
    parser.add_argument('--record-moving-piston-witness', nargs='?', type=Path,
                        const=lab.LOCAL / 'witness' / lab.VERSION / 'falling_moving_piston' / 'falling_block_moving_piston.jsonl',
                        help='write the validated Java reference records with a version header and meta.json to this path (default under .local/witness/) and stop')
    parser.add_argument('--record-stand-living-witness', type=Path,
                        help='write a versioned JSON witness from the Java reference run (with --reference-only)')
    parser.add_argument('--record-fangs-ticking-witness', nargs='?',
                        const=lab.LOCAL / 'witness' / lab.VERSION / 'fangs_ticking_gate' / 'fangs_ominous_spawner_ticking_gate.json',
                        type=Path,
                        help='record the 26.3 vanilla ticking-gate witness JSON into .local (Java only)')
    parser.add_argument('--record-thrown-item-witness', type=Path,
                        help='write verified vanilla oracle records to this witness JSON file')
    parser.add_argument('--record', type=Path,
                        help='record vanilla reference steps for non-setup groups to PATH (implies --reference-only)')
    args = parser.parse_args()
    if args.record_eye_item_witness:
        args.reference_only = True
        if not args.only:
            args.only = 'summoned eye of ender'
    build = args.work_dir.resolve() if args.work_dir else lab.LOCAL / 'command-exec-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    if args.loaded_fixture:
        for name in ['client_config.json', 'rust-client-config.json']:
            (build / name).unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependency_path = os.pathsep.join(map(str, libraries))

    def run(command, name, **kw):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            runner = run_game if kw.pop('game', False) else subprocess.run
            result = runner(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT, **kw)
        if result.returncode:
            tail = (build / (name + '.log')).read_text(errors='replace')[-3000:]
            sys.exit(f'{name} failed ({result.returncode}):\n{tail}')

    groups = corpus_module.groups()
    if not args.only:
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_BOAT_TREE]
    if not args.only:
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_EFFECTIVE_PRESENCE]
    if not args.only:
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_FUNCTION_FEEDBACK]
    if not args.only:
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_JUKEBOX_CLOCK]
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_GUEST_CLOSE]
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_ITEM_COUNT]
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_SHULKER_LOCK]
        groups = [g for g in groups if g[0] not in corpus_module.ISOLATED_POT_HOPPER]
    if args.vehicle_container_diagnostic:
        from tools.vehicle_container_corpus import vehicle_containers
        groups = [(name, vehicle_containers(corpus_module.cmd, diagnostic=True)
                   if name == 'vehicle container saved fields' else ops) for name, ops in groups]
    if args.macro_source_type_fixture or args.only == "macro source type error":
        groups = [(name, ops) for name, ops in groups if name == "macro source type error"]
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_MACRO_TYPE]
    if args.stand_fall_fixture or args.only == "stand saved fall distance":
        groups = [(name, ops) for name, ops in groups if name == "stand saved fall distance"]
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_STAND_FALL]
    if args.orb_uuid_fixture or args.only == 'orb saved UUID':
        groups = [(name, ops) for name, ops in groups if name == 'orb saved UUID']
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_ORB_UUID]
    if args.mob_pickup_save_fixture or args.only == "mob saved pickup loot":
        groups = [(name, ops) for name, ops in groups if name == "mob saved pickup loot"]
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_PICKUP_SAVE]
    if not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_FRAME_MAPS]
    if not args.only and not args.wolf_anger_clock_diagnostic:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_WOLF_ANGER]
    if args.wolf_anger_clock_diagnostic:
        from tools.wolf_anger_corpus import groups as anger_groups
        groups = [('setup', corpus_module.group_setup()),
                  *anger_groups(corpus_module.cmd, corpus_module.setup, clock_diagnostic=True)]
    isolated_reinforcements = corpus_module.ISOLATED_REINFORCEMENTS
    if not args.only:
        groups = [(name, ops) for name, ops in groups if name not in isolated_reinforcements]
    if args.reinforcement_raw_diagnostic or args.reinforcement_original_diagnostic:
        groups = [(name, corpus_module.command_zombie_reinforcements(
            once=name == 'command reinforcement once', raw=args.reinforcement_raw_diagnostic, legacy=args.reinforcement_original_diagnostic) if name in isolated_reinforcements else ops)
            for name, ops in groups]
    if not args.reinforcement_target_diagnostic and not args.reinforcement_raw_diagnostic and not args.reinforcement_original_diagnostic:
        # H300's declared208 gate excludes exactly this separate target-policy lead.
        # --reinforcement-target-diagnostic retains full209/all30 original records.
        groups = [(name, [op for op in ops if op.get('label') != 'h300 original/hard/callee target']
                   if name == 'command zombie reinforcements' else ops) for name, ops in groups]
    if args.advancement_loot_named_diagnostic and args.advancement_loot_inline_parser_diagnostic:
        parser.error('choose only one advancement-loot diagnostic')
    if args.advancement_loot_named_diagnostic or args.advancement_loot_inline_parser_diagnostic:
        groups = [(name, corpus_module.advancement_world_loot_rewards(
            named_table=args.advancement_loot_named_diagnostic, include_missing_ack=True)
                   if name == 'advancement world loot rewards' else ops) for name, ops in groups]
    # These overlays replace built-in recipes: run each in its own world, never
    # beside the generic corpus's fixed vanilla recipe-count controls.
    isolated_recipes = corpus_module.ISOLATED_RECIPES | corpus_module.ISOLATED_MAP_CRAFTING
    if not args.only:
        groups = [(name, ops) for name, ops in groups if name not in isolated_recipes]
    if args.loaded_fixture:
        groups = [('setup', corpus_module.group_setup()),
                  ('loaded ticket removal', corpus_module.loaded_ticket_removal(args.loaded_block_diagnostic))]
    if args.spectral_duration_fixture or args.only == 'spectral saved duration':
        groups = [(name, ops) for name, ops in corpus_module.groups() if name == 'spectral saved duration']
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_SPECTRAL_DURATION]
    if args.item_health_fixture or args.only == 'item Health short load':
        groups = [(name, ops) for name, ops in corpus_module.groups() if name == 'item Health short load']
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_ITEM_HEALTH]
    if args.potion_effect_icon_fixture or args.only == 'potion custom effect icons':
        groups = [(name, ops) for name, ops in corpus_module.groups()
                  if name == 'potion custom effect icons']
    elif not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_POTION_ICONS]
    if not args.only:
        groups = [(name, ops) for name, ops in groups if name not in corpus_module.ISOLATED_SEED_SYNC]
    if args.moving_piston_history_diagnostic:
        groups.append(('moving piston presentation diagnostic',
                       corpus_module.moving_piston_presentation_diagnostic()))
    if args.pot_string_sherd_diagnostic:
        groups = [(name, corpus_module.decorated_pot_data(string_sherd=True)
                   if name == 'decorated pot data' else ops) for name, ops in groups]
    if args.array_storage_diagnostic:
        groups = [('setup', corpus_module.group_setup()),
                  ('numeric array edits with storage side effects',
                   corpus_module.numeric_array_edits(include_storage_side_effects=True))]
    if args.sulfur_power_load_diagnostic:
        groups = [('setup', corpus_module.group_setup()),
                  ('sulfur powered-load diagnostic', corpus_module.sulfur_cube_fuse_ticks(True))]
    if args.hurt_cooldown_incoming_fixture:
        from tools.hurt_cooldown_incoming_corpus import hurt_cooldown_incoming
        groups = [('setup', corpus_module.group_setup()), ('hurt cooldown incoming', hurt_cooldown_incoming())]
    if args.level_world_border_fixture:
        from tools.level_world_border_corpus import level_world_border
        groups = [('setup', corpus_module.group_setup()), ('level world border', level_world_border())]
    if args.trader_llama_discard_fixture:
        from tools.trader_llama_discard_corpus import trader_llama_discard
        groups = [('setup', corpus_module.group_setup()), ('trader llama discard', trader_llama_discard())]
    if args.map_decoration_namespace_fixture:
        from tools.map_decoration_namespace_corpus import map_decoration_namespaces
        groups = [("setup", corpus_module.group_setup()), ("map decoration namespaces", map_decoration_namespaces(args.map_decoration_empty_diagnostic))]
    if args.armor_stand_honey_fixture:
        from tools.armor_stand_honey_corpus import armor_stand_honey
        groups = [("setup", corpus_module.group_setup()), ("armor stand honey slide", armor_stand_honey())]
    if args.mob_quad_fixture:
        from tools.mob_quad_leash_corpus import mob_quad_leash
        groups = [('setup', corpus_module.group_setup()), ('mob quad leash dispatch', mob_quad_leash(args.mob_quad_raw_diagnostic))]
    if args.item_merge_diagnostic:
        from tools.item_merge_corpus import stack_limit_merges
        groups = [('setup', corpus_module.group_setup()),
                  ('item stack-size override merges',
                   stack_limit_merges(corpus_module.cmd, corpus_module.setup, diagnostic=True))]
    if args.orb_water_fixture:
        from tools.orb_water_corpus import orb_water, arithmetic_inputs
        groups = [('setup', corpus_module.group_setup()), ('orb underwater arithmetic', orb_water(args.orb_water_raw_diagnostic))]
    if args.orb_lava_fixture:
        from tools.orb_lava_corpus import orb_lava
        groups = [('setup', corpus_module.group_setup()),
                  ('orb lava contact', orb_lava(args.orb_lava_raw_diagnostic))]
    if args.falling_first_tick_fixture:
        from tools.falling_first_tick_corpus import falling_first_tick
        groups = [('setup', corpus_module.group_setup()), ('falling first tick', falling_first_tick())]
    if args.orb_fire_fixture:
        from tools.orb_fire_corpus import orb_fire
        groups = [('setup', corpus_module.group_setup()), ('orb fire', orb_fire())]
    if args.lava_fire_fixture:
        from tools.lava_fire_corpus import lava_fire
        groups = [('setup', corpus_module.group_setup()), ('lava fire', lava_fire())]
    if args.mob_home_fixture:
        from tools.mob_home_corpus import mob_home
        groups = [('setup', corpus_module.group_setup()), ('mob home NBT', mob_home(args.mob_home_holder_removal_diagnostic, args.mob_home_late_detach_control))]
    if args.item_contents_fixture:
        from tools.item_contents_corpus import item_contents
        groups = [('setup', corpus_module.group_setup()), ('item component contents', item_contents(args.item_contents_provider_empty_control))]
    if args.boat_item_hit_fixture:
        from tools.boat_item_hit_corpus import boat_item_hits
        groups = [('boat item hit', boat_item_hits(args.boat_motion_diagnostic))]
    if args.stand_entity_collision_fixture:
        from tools.stand_entity_collision_corpus import stand_entity_collisions
        groups = [('stand entity collisions', stand_entity_collisions())]
    if args.pufferfish_mob_fixture:
        from tools.pufferfish_mob_contact_corpus import mob_contacts, overlapping_cod_control
        groups = [("pufferfish mob contact", overlapping_cod_control() if args.pufferfish_mob_overlap_control
                   else mob_contacts(args.pufferfish_mob_duration_diagnostic))]
    if args.pufferfish_contact_fixture:
        from tools.pufferfish_contact_corpus import contact_fixture
        groups = [("pufferfish player contact", contact_fixture(args.pufferfish_body_diagnostic))]
    if args.only in ('hopper pot saved slot', 'shulker external lock drop'):
        groups = [g for g in groups if g[0] == args.only]
    elif args.only:
        groups = [g for g in groups if args.only in g[0] or g[0] == 'setup']
    if args.only in corpus_module.ISOLATED_BOAT_TREE:
        groups = [g for g in groups if g[0] == args.only]
    if any(n in corpus_module.ISOLATED_BOAT_TREE for n,_ in groups) and len(groups) != 1:
        parser.error('boat tree fixture requires exact --only group')
    if args.only in corpus_module.ISOLATED_EFFECTIVE_PRESENCE:
        groups = [g for g in groups if g[0] == args.only]
    if any(n in corpus_module.ISOLATED_EFFECTIVE_PRESENCE for n, _ in groups) and len(groups) != 1:
        parser.error('effective presence fixture requires exact --only group')
    if args.only in corpus_module.ISOLATED_FUNCTION_FEEDBACK:
        groups = [g for g in groups if g[0] == args.only]
    if args.only in corpus_module.ISOLATED_JUKEBOX_CLOCK:
        groups = [g for g in groups if g[0] == args.only]
    if args.vehicle_kill_diagnostic:
        from tools.vehicle_kill_corpus import vehicle_kill_contents
        groups = [('setup', corpus_module.group_setup()), ('vehicle kill contents', vehicle_kill_contents(corpus_module.cmd, True))]
    if args.only in corpus_module.ISOLATED_ITEM_COUNT:
        groups = [g for g in groups if g[0] == args.only]
    if args.slot_codec_diagnostic:
        groups.append(('slot source codec diagnostic', [corpus_module.cmd(
            '/item fill block 4 200 4 {type:"minecraft:slot_range",slots:"container.99"} with diamond')]))
    if any(name in corpus_module.ISOLATED_JUKEBOX_CLOCK for name, _ in groups) and len(groups) != 1:
        parser.error('jukebox clock fixture requires --only with its exact group')
    if any(name in corpus_module.ISOLATED_ITEM_COUNT for name, _ in groups) and len(groups) != 1:
        parser.error('item count fixture requires --only with its exact group')
    if any(name in corpus_module.ISOLATED_SHULKER_LOCK for name, _ in groups) and len(groups) != 1:
        parser.error('shulker lock fixture requires --only with its exact group')
    if any(name in corpus_module.ISOLATED_POT_HOPPER for name, _ in groups) and len(groups) != 1:
        parser.error('pot hopper fixture requires --only with its exact group')
    if any(name in isolated_reinforcements for name, _ in groups) and sum(name != 'setup' for name, _ in groups) != 1:
        parser.error('reinforcement fixtures require --only with one exact group')
    if any(name in isolated_recipes for name, _ in groups) and sum(name != 'setup' for name, _ in groups) != 1:
        parser.error('crafting recipe fixtures require --only with one exact group')
    if args.piglin_anger_memory_fixture:
        from tools.piglin_anger_memory_corpus import piglin_anger_memory
        groups = [('setup', corpus_module.group_setup()), ('piglin live anger memory', piglin_anger_memory(args.piglin_anger_raw_diagnostic, args.piglin_anger_memory_diagnostic))]
    if args.map_decoration_patch_fixture:
        from tools.map_decoration_patch_corpus import map_decoration_patches
        groups = [('setup', corpus_module.group_setup()), ('map decoration saved patch', map_decoration_patches(args.map_decoration_removal_diagnostic))]
    if args.selector_bounds_fixture:
        from tools.selector_bounds_corpus import selector_bounds
        groups = [("setup", corpus_module.group_setup()), ("selector entity bounds", selector_bounds())]
    if args.baby_modifier_fixture:
        from tools.baby_modifier_corpus import baby_modifier
        groups = [('setup', corpus_module.group_setup()), ('baby modifier mutation', baby_modifier(args.baby_modifier_raw_diagnostic))]
    if args.mob_attribute_save_fixture or args.record_oxygen_bonus_witness:
        from tools.mob_attribute_save_corpus import mob_attribute_save
        groups = [('setup', corpus_module.group_setup()),
                  ('mob attribute save', mob_attribute_save(
                      args.mob_attribute_save_raw_diagnostic or bool(args.record_oxygen_bonus_witness)))]
    if args.mob_inventory_merge_fixture:
        from tools.mob_inventory_merge_corpus import mob_inventory_merge
        groups = [('setup', corpus_module.group_setup()), ('mob inventory merge', mob_inventory_merge(args.mob_inventory_merge_raw_diagnostic))]
    if args.summon_passengers_fixture:
        from tools.summon_passengers_corpus import summon_passengers
        groups = [('setup', corpus_module.group_setup()), ('summon invalid passengers', summon_passengers(args.summon_passengers_raw_diagnostic))]
    if args.summon_uuid_fixture:
        from tools.summon_uuid_corpus import summon_uuid
        groups = [('setup', corpus_module.group_setup()), ('summon UUID conflicts', summon_uuid(args.summon_uuid_raw_diagnostic, True))]
    if args.mob_absorption_max_fixture:
        from tools.mob_absorption_max_corpus import mob_absorption_max
        groups = [('setup', corpus_module.group_setup()), ('mob absorption maximum', mob_absorption_max(args.mob_absorption_max_raw_diagnostic))]
    if args.mob_absorption_max_ops_fixture:
        from tools.mob_absorption_max_corpus import mob_absorption_max_ops
        groups = [('setup', corpus_module.group_setup()), ('mob absorption maximum operations', mob_absorption_max_ops())]
    if args.mob_health_boost_fixture:
        from tools.mob_health_boost_corpus import mob_health_boost
        groups = [('setup', corpus_module.group_setup()), ('mob health boost', mob_health_boost(args.mob_health_boost_raw_diagnostic))]
    if args.sulfur_body_attributes_fixture:
        from tools.sulfur_body_attributes_corpus import sulfur_body_attributes
        groups = [('setup', corpus_module.group_setup()), ('sulfur body attributes', sulfur_body_attributes(args.sulfur_body_attributes_raw_diagnostic))]
    if args.sulfur_body_replace_fixture:
        from tools.sulfur_body_attributes_corpus import sulfur_body_replace
        groups = [('setup', corpus_module.group_setup()), ('sulfur body replace', sulfur_body_replace())]
    if args.sulfur_body_item_fixture:
        from tools.sulfur_body_attributes_corpus import sulfur_body_item_replace
        groups = [('setup', corpus_module.group_setup()), ('sulfur body item replace', sulfur_body_item_replace())]
    if args.mob_absorption_fixture:
        from tools.mob_absorption_corpus import mob_absorption
        groups = [('setup', corpus_module.group_setup()), ('mob depleted absorption', mob_absorption(args.mob_absorption_raw_diagnostic))]
    if args.mob_absorption_infinite_fixture:
        from tools.mob_absorption_corpus import mob_absorption_infinite
        groups = [('setup', corpus_module.group_setup()), ('mob infinite absorption', mob_absorption_infinite())]
    if args.boat_leash_view_fixture:
        from tools.boat_leash_view_corpus import cases as boat_leash_views
        groups = [('setup', corpus_module.group_setup()),
                  ('boat live leash view', boat_leash_views(corpus_module.cmd, corpus_module.setup))]
    if args.merchant_crafted_fixture:
        from tools.merchant_crafted_corpus import cases
        groups = [('merchant crafted statistics', cases())]
    if args.mob_name_presentation_diagnostic:
        from tools.mob_name_predicate_corpus import cases as mob_name_predicates
        groups = [('setup', corpus_module.group_setup()),
                  ('mob custom name components', mob_name_predicates(
                      corpus_module.cmd, corpus_module.setup, presentation_diagnostic=True))]
    if args.merchant_gate_fixture:
        from tools.merchant_gate_corpus import cases
        groups = [('setup', [corpus_module.cmd('/tick freeze', False), *corpus_module.group_setup()]), ('merchant reputation AI gate', cases())]
    if args.groups_file:
        # Generated scripts (tools/parity_fuzz.py): the common setup, then the file's [name, ops] groups only.
        groups = [('setup', corpus_module.group_setup()),
                  *[(name, ops) for name, ops in json.loads(args.groups_file.read_text(encoding='utf-8'))]]
    if args.record_shulker_motion:
        groups = [('setup', corpus_module.group_setup()),
                  (corpus_module.SHULKER_MOTION_GROUP, corpus_module.shulker_motion_and_yaw())]
    if args.record_fangs_ticking_witness:
        witness_cases = corpus_module.fangs_ominous_spawner_ticking_witness_cases()
        pause_spec = corpus_module.fangs_spawner_scheduled_ticket_pause_witness_spec()
        groups = [('setup', corpus_module.group_setup()),
                  ('fangs and ominous spawner ticking witness',
                   corpus_module.fangs_ominous_spawner_ticking_witness_script(witness_cases, pause_spec))]
    script = assemble(groups, args.unsynced_randoms)
    # docs/parity-workflow.md "Randoms that repeat": both engines' unseeded randoms repeat for a script that syncs them.
    synced = any(op['op'] == 'sync_randoms' for op in script)
    script_path = build / 'script.jsonl'
    script_path.write_text(''.join(json.dumps({k: v for k, v in op.items() if k != 'expect_result'},
                                            ensure_ascii=False) + '\n' for op in script), encoding='utf-8')
    predicates = corpus_module.predicate_selector_fixtures([name for name, _ in groups])
    functions = corpus_module.function_return_fixtures([name for name, _ in groups])
    recipes = corpus_module.crafting_priority_recipes([name for name, _ in groups])
    if any(name == 'transmute result patches' for name, _ in groups):
        from tools.transmute_patch_corpus import fixture_recipes
        recipes.update(fixture_recipes())
    advancements = corpus_module.advancement_reward_fixtures([name for name, _ in groups])
    loot_advancements, loot_tables = corpus_module.advancement_loot_fixtures([name for name, _ in groups])
    advancements.update(loot_advancements)
    if os.environ.get('SUNRISE_FISHING_FALLBACK_FIXTURE') == '1':
        # No random_sequence: FishingHook.retrieve must use the level stream.
        loot_tables['minecraft:gameplay/fishing'] = {
            'type': 'minecraft:fishing', 'pools': [{'rolls': 1, 'entries': [
                {'type': 'minecraft:item', 'name': 'minecraft:cod', 'weight': 1},
                {'type': 'minecraft:item', 'name': 'minecraft:salmon', 'weight': 1},
            ]}]}

    if args.pufferfish_contact_fixture:
        from tools.pufferfish_contact_corpus import advancements as contact_advancements
        advancements.update(contact_advancements())

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', str(jars['server']) + os.pathsep + dependency_path, '-d', classes, oracle], 'compile-java')
    vanilla_dir = build / 'vanilla'
    shutil.rmtree(vanilla_dir, ignore_errors=True)
    vanilla_dir.mkdir()
    install_datapack_fixtures(vanilla_dir / 'universe/harness', predicates, functions, jars['server'], advancements, loot_tables, recipes=recipes)
    java_options = ['-DMC_DEBUG_ENABLED=true', '-DMC_DEBUG_DEV_COMMANDS=true'] if args.raid_village_fixture else []
    if args.merchant_gate_fixture:
        java_options.append('-Dsunrise.merchantFixture=true')
    if args.merchant_crafted_fixture:
        java_options.append('-Dsunrise.merchantCraftedFixture=true')
    if args.orb_water_fixture:
        (build / 'orb-water-reference.json').unlink(missing_ok=True)
        direct_input = build / 'orb-water-inputs.json'
        direct_input.write_text(json.dumps(arithmetic_inputs()), encoding='utf-8')
        java_options = ['-Dsunrise.orbWaterArithmetic=' + str(direct_input)]
    if args.function_admission_fixture:
        java_options.append('-Dsunrise.functionAdmissionFixture=true')
    if synced:
        java_options.append(seed_agent.java_option(build, lock['minecraft']['java_major']))
    if args.record_fangs_ticking_witness:
        (build / 'vanilla.jsonl').unlink(missing_ok=True)
    java_cmd = [
        'java', '-Xmx3G', '-Duser.timezone=UTC',
        *(['-Dsunrise.loadedFixture=true'] if args.loaded_fixture or args.record_fangs_ticking_witness else []),
        *java_options, '-cp', os.pathsep.join([str(classes), str(jars['server']), dependency_path]),
        'dev.sunriselab.parity.VanillaCommandExecOracle', vanilla_dir / 'universe', script_path, build / 'vanilla.jsonl', SEED,
    ]
    run(java_cmd, 'java', cwd=vanilla_dir)

    if args.record_fangs_ticking_witness:
        witness_path = args.record_fangs_ticking_witness.resolve()
        write_fangs_ticking_witness(
            witness_path,
            lock['minecraft']['version'],
            witness_cases,
            pause_spec,
            [json.loads(line) for line in (build / 'vanilla.jsonl').read_text(encoding='utf-8').splitlines()],
        )
        java_ver_proc = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
        java_ver = (java_ver_proc.stderr or java_ver_proc.stdout).strip().splitlines()[0]
        lab.write_json(witness_path.with_name('meta.json'), {
            'game_version': lock['minecraft']['version'],
            'java_major': lock['minecraft']['java_major'],
            'java_version': java_ver,
            'harness': 'tools/test_fangs_ticking_gate_parity.py',
            'witness_env': 'SUNRISE_FANGS_TICKING_GATE_WITNESS',
            'oracle': oracle.relative_to(lab.ROOT).as_posix(),
            'command': [str(part) for part in java_cmd],
            'input_hashes': {
                'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
                'server.jar': lab.digest(jars['server'], 'sha256'),
                oracle.relative_to(lab.ROOT).as_posix(): lab.digest(oracle, 'sha256'),
                'tools/command_exec_corpus.py': lab.digest(lab.ROOT / 'tools/command_exec_corpus.py', 'sha256'),
                'script.jsonl': lab.digest(script_path, 'sha256'),
            },
            'outputs': {
                witness_path.name: {'sha256': lab.digest(witness_path, 'sha256'), 'bytes': witness_path.stat().st_size},
            },
        })
        print(f'Recorded {lock["minecraft"]["version"]} fangs/spawner ticking witness: {witness_path}')
        return

    if any(op['op'] == 'raid_state' for op in script):
        from tools.raid_village_corpus import validate_java_inputs
        validate_java_inputs(build, script)

    if args.orb_water_fixture:
        direct = json.loads((build / 'orb-water-reference.json').read_text())
        if ([r.get('input') for r in direct] != arithmetic_inputs() or
                any(len(r.get('bits', [])) != 3 or
                    any(re.fullmatch(r'[0-9a-f]{1,16}', b) is None for b in r['bits']) for r in direct)):
            sys.exit('Java direct underwater reference prerequisite failed')
    if args.reference_only or args.record or args.record_shulker_motion or args.record_moving_piston_witness or args.record_summon_name_witness:
        reference = [json.loads(line) for line in (build / 'vanilla.jsonl').read_text(encoding='utf-8').splitlines()]
        recorded = [op for op in script if op['op'] in ('player_state', 'player_loop', 'map_state', 'frame_map_state', 'mob_anger', 'projectile_saved_type', 'raid_state', 'menu_state', 'craft_menu_state', 'map_files', 'merchant_state', 'merchant_saved_stats') or
                    (op['op'] == 'cmd' and op.get('record', True)) or
                    (op['op'] == 'tick' and op.get('record'))]
        if len(recorded) != len(reference):
            sys.exit('reference-only record count differs')
        for op, row in zip(recorded, reference):
            if "expect_merchant" in op and row.get("state") != op["expect_merchant"]:
                sys.exit(f"Java merchant admission failed: {op}: {row}")
            require_anger_reference(op, row)
            if "expect_observation" in op and row.get("state") != op["expect_observation"]:
                sys.exit(f"Java observation prerequisite failed: {op}: {row}")
            if 'expect_result' in op and row.get('results') != [[True, op['expect_result']]]:
                sys.exit(f'Java fixture guard failed: {op}: {row.get("results")}')
            if 'expect_callback' in op and row.get('results') != [op['expect_callback']]:
                sys.exit(f'Java callback guard failed: {op}: {row.get("results")}')
            if 'expect_map' in op and row.get('messages') != [[[op['expect_map'], [None, False, False, False, False, False]]]]:
                sys.exit(f'Java map prerequisite failed: {op}: {row}')
        if args.record_eye_item_witness:
            write_eye_item_witness(Path(args.record_eye_item_witness), lock['minecraft']['version'], reference)
        if args.record_shulker_motion:
            write_shulker_motion_witness(args.record_shulker_motion, reference, lock, jars['server'],
                                         script_path=script_path, oracle_path=oracle)
            print(f'Recorded {lock["minecraft"]["version"]} shulker motion witness: {args.record_shulker_motion}')
            return
        if args.record_oxygen_bonus_witness:
            from tools.mob_attribute_save_corpus import record_oxygen_bonus_witness
            record_oxygen_bonus_witness(reference, lock['minecraft']['version'], args.record_oxygen_bonus_witness)
            print(f'Recorded oxygen_bonus witness: {args.record_oxygen_bonus_witness}')
        if args.record_stand_living_witness:
            names = [name for name, _ in groups if name != 'setup']
            if names != [corpus_module.STAND_LIVING_GROUP]:
                sys.exit(f'--record-witness supports only {corpus_module.STAND_LIVING_GROUP!r}, got {names!r}')
            witness = corpus_module.extract_stand_living_witness(lock['minecraft']['version'], reference)
            args.record_stand_living_witness.parent.mkdir(parents=True, exist_ok=True)
            args.record_stand_living_witness.write_text(json.dumps(witness, indent=2) + '\n', encoding='utf-8')
            print(f'Recorded witness: {args.record_stand_living_witness}')
        if args.record_thrown_item_witness:
            write_witness(args.record_thrown_item_witness, lock, groups, reference)
        if args.record_moving_piston_witness:
            target = args.record_moving_piston_witness if args.record_moving_piston_witness.is_absolute() else lab.ROOT / args.record_moving_piston_witness
            target.parent.mkdir(parents=True, exist_ok=True)
            recorded_groups = [name for name, _ in groups if name != 'setup']
            header = json.dumps({
                'minecraft': lock['minecraft']['version'],
                'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
                'groups': recorded_groups,
            }, ensure_ascii=False)
            target.write_text(header + '\n' + (build / 'vanilla.jsonl').read_text(encoding='utf-8'), encoding='utf-8')
            java_ver_proc = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
            java_version = (java_ver_proc.stderr or java_ver_proc.stdout).strip().splitlines()[0]
            lab.write_json(target.parent / 'meta.json', {
                'game_version': lock['minecraft']['version'],
                'java_major': lock['minecraft']['java_major'],
                'java_version': java_version,
                'harness': 'tools/test_falling_moving_piston_parity.py',
                'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
                'groups': recorded_groups,
                'witness_env': 'SUNRISE_FALLING_MOVING_PISTON_WITNESS',
                'command': [sys.executable, *sys.argv],
                'input_hashes': {
                    'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
                    'server.jar': lab.digest(jars['server'], 'sha256'),
                    'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java': lab.digest(oracle, 'sha256'),
                    'tools/command_exec_corpus.py': lab.digest(lab.ROOT / 'tools/command_exec_corpus.py', 'sha256'),
                    'tools/test_command_exec_parity.py': lab.digest(Path(__file__), 'sha256'),
                },
                'outputs': {
                    target.name: {'sha256': lab.digest(target, 'sha256'), 'bytes': target.stat().st_size},
                },
            })
            print(f'Recorded witness ({len(reference)} records): {target}')
        if args.record_summon_name_witness:
            payload = {
                'minecraft': lock['minecraft']['version'],
                'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
                'groups': [name for name, _ in groups if name != 'setup'],
                'records': reference,
            }
            args.record_summon_name_witness.parent.mkdir(parents=True, exist_ok=True)
            args.record_summon_name_witness.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
            print(f'Recorded {len(reference)} vanilla {lock["minecraft"]["version"]} witness records to {args.record_summon_name_witness}')
        if args.record:
            target_groups = [(name, ops) for name, ops in groups if name != 'setup']
            ref_iter = iter(reference)
            steps = []
            for _, ops in target_groups:
                for op in ops:
                    if op['op'] == 'cmd':
                        if op.get('record', True):
                            row = next(ref_iter)
                            if row['cmd'] != op['cmd']:
                                sys.exit(f'--record command mismatch: {row["cmd"]!r} != {op["cmd"]!r}')
                            steps.append({
                                'op': 'cmd',
                                'cmd': row['cmd'],
                                'messages': row['messages'],
                                'results': row['results'],
                            })
                        else:
                            steps.append({'op': 'cmd', 'cmd': op['cmd'], 'record': False})
                    elif op['op'] == 'tick':
                        if op.get('record'):
                            next(ref_iter)
                        steps.append({'op': 'tick', 'n': op.get('n', 1)})
                    elif op['op'] in ('sync_randoms', 'wait_chunks'):
                        steps.append(dict(op))
                    else:
                        sys.exit(f'--record does not support op {op["op"]}')
            if next(ref_iter, None) is not None:
                sys.exit('--record left unconsumed vanilla reference records')
            witness = {
                'minecraft': lock['minecraft']['version'],
                'oracle': 'VanillaCommandExecOracle',
                'groups': [name for name, _ in target_groups],
                'steps': steps,
            }
            args.record.parent.mkdir(parents=True, exist_ok=True)
            args.record.write_text(json.dumps(witness, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
            print(f'Recorded {len(steps)} steps ({len(reference)} commands) to {args.record}')
        print(f'Java reference: {len(reference)} records, fixture guards passed')
        return

    if args.binary:
        binary = args.binary.resolve(strict=True)
    else:
        run(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client'], 'compile-rust', cwd=lab.ROOT)
        metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
        binary = Path(metadata['target_directory']) / 'fast' / ('sunrise-client.exe' if os.name == 'nt' else 'sunrise-client')
    rust_dir = build / 'rust'
    shutil.rmtree(rust_dir, ignore_errors=True)
    rust_dir.mkdir()
    install_datapack_fixtures(rust_dir / 'saves/harness', predicates, functions, jars['server'], advancements, loot_tables, recipes=recipes)
    if any(op['op'] == 'crafter_max_map_fixture' for op in script):
        source = vanilla_dir / 'universe/harness/data/minecraft/maps/31.dat'
        target = rust_dir / 'saves/harness/data/minecraft/maps/31.dat'
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    # The oracle's Services resolve no profile (/fetchprofile fails on the next tick): the game gets no services
    # either, a discovery address that refuses at once, rather than Mojang's over the network.
    env = lab.game_env(SUNRISE_PLAYER_NAME="Player", SUNRISE_COMMAND_SCRIPT=script_path,
                       SUNRISE_COMMAND_OUTPUT=build / 'rust.jsonl', SUNRISE_API_DISCOVERY='http://127.0.0.1:9/')
    if synced:
        env.update(seed_agent.ENV)
    if args.raid_village_fixture and not args.raid_release_command_diagnostic:
        env['SUNRISE_DEV_COMMANDS'] = '1'
    elif args.raid_release_command_diagnostic:
        env.pop('SUNRISE_DEV_COMMANDS', None)
    if args.loaded_fixture:
        env['SUNRISE_HARNESS_LOADED_FIXTURE'] = '1'
        env['SUNRISE_HARNESS_CLIENT_CONFIG'] = str(build / 'rust-client-config.json')
    run([binary, '--resources', lab.RESOURCES, '--world', 'harness', '--seed', SEED, '--radius', 2 if args.loaded_fixture else 4], 'rust', game=True, cwd=rust_dir, env=env)

    if any(op['op'] == 'raid_state' for op in script):
        from tools.raid_village_corpus import validate_native_inputs
        if args.raid_release_command_diagnostic:
            print('REJECTED admission prerequisite diagnostic: native release tree vs Java development tree; not H162 loss evidence')
        else:
            validate_native_inputs(build, script)

    if args.loaded_fixture:
        for name in ['client_config.json', 'rust-client-config.json']:
            config = json.loads((build / name).read_text())
            if config.get('view_distance') != 2 or config.get('simulation_distance') != 2:
                sys.exit(f'actual login distance prerequisite failed: {name}: {config}')
        java = json.loads((build / 'client_config.json').read_text())
        if java.get('server_view') != 2 or java.get('server_simulation') != 2 or not java.get('client_loaded'):
            sys.exit(f'Java server/client-loaded prerequisite failed: {java}')
        if not args.merchant_gate_fixture:
            for name in ['java.log', 'rust.log']:
                waits = re.findall(r'harness paced wait: (\d+) ticks, (\d+) ms', (build / name).read_text())
                if len(waits) != 2 or any(int(n) != ticks or not ticks * 50 <= int(ms) <= upper
                        for (n, ms), ticks, upper in zip(waits, [40, 120], [4000, 10000])):
                    sys.exit(f'bounded paced-wait prerequisite failed: {name}: {waits}')
        native = json.loads((build / 'rust-client-config.json').read_text())
        if native.get('render_setting') != 2 or native.get('simulation_setting') != 2 or not native.get('joined'):
            sys.exit(f'native settings/join prerequisite failed: {native}')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').open(encoding='utf-8')]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').open(encoding='utf-8')]
    if len(vanilla) != len(rust):
        sys.exit(f'record count differs: vanilla {len(vanilla)} rust {len(rust)}')
    recorded_ops = [op for op in script if op['op'] in ('falling', 'destroy_progress', 'weather_state', 'chunk_ready', 'server_distances', 'map_state', 'frame_map_state', 'light', 'player_state', 'player_loop', 'mob_anger', 'projectile_saved_type', 'raid_state', 'menu_state', 'craft_menu_state', 'map_files', 'merchant_state', 'merchant_saved_stats') or
                    (op['op'] == 'cmd' and op.get('record', True)) or
                    (op['op'] == 'tick' and op.get('record'))]
    if len(recorded_ops) != len(vanilla):
        sys.exit(f'script record count differs: expected {len(recorded_ops)}, got {len(vanilla)}')
    for op, reference in zip(recorded_ops, vanilla):
        if "expect_merchant" in op and reference.get("state") != op["expect_merchant"]:
            sys.exit(f"Java merchant admission failed: {op}: {reference}")
        require_anger_reference(op, reference)
        if "expect_observation" in op and reference.get("state") != op["expect_observation"]:
            sys.exit(f"Java observation prerequisite failed: {op}: {reference}")
        if 'expect_result' in op and reference.get('results') != [[True, op['expect_result']]]:
            sys.exit(f'Java fixture control failed: {op["cmd"]}: {reference.get("results")}')
        if 'expect_values' in op and reference.get('results') != [[True, n] for n in op['expect_values']]:
            sys.exit(f'Java readiness/distance prerequisite failed: {op}: {reference}')
        if 'expect_callback' in op and reference.get('results') != [op['expect_callback']]:
            sys.exit(f'Java loaded contract failed: {op}: {reference}')
        if 'expect_callback' in op and reference.get('results') != [op['expect_callback']]:
            sys.exit(f'Java callback guard failed: {op}: {reference.get("results")}')
        if 'expect_map' in op and reference.get('messages') != [[[op['expect_map'], [None, False, False, False, False, False]]]]:
            sys.exit(f'Java map fixture control failed: {op}: {reference}')
        if 'expect_state' in op and reference.get('results') != [[True, v] for v in op['expect_state']]:
            sys.exit(f'Java weather guard failed: {op["label"]}: {reference.get("results")}')
        if 'expect_light' in op and reference.get('results') != [[True, n] for n in op['expect_light']]:
            sys.exit(f'Java light prerequisite failed: {op}: {reference}')
    if args.record_thrown_item_witness:
        write_witness(args.record_thrown_item_witness, lock, groups, vanilla)
    commands_by_group = []
    index = 0
    for name, lines in groups:
        n = sum(1 for op in lines if op['op'] in ('falling', 'destroy_progress', 'weather_state', 'chunk_ready', 'server_distances', 'map_state', 'frame_map_state', 'light', 'player_state', 'player_loop', 'mob_anger', 'projectile_saved_type', 'raid_state', 'menu_state', 'craft_menu_state', 'map_files', 'merchant_state', 'merchant_saved_stats') or (op['op'] == 'cmd' and op.get('record', True)) or (op['op'] == 'tick' and op.get('record')))
        commands_by_group.append((name, index, index + n))
        index += n
    mismatches = []
    failed_positions = set()
    for position, (v, r) in enumerate(zip(vanilla, rust)):
        assert v['cmd'] == r['cmd'], (v['cmd'], r['cmd'])
        kind = ('strict_record' if v != r else None) if args.merchant_gate_fixture or args.merchant_crafted_fixture else compare(v, r, recorded_ops[position].get('comparison'))
        if kind:
            failed_positions.add(position)
            mismatches.append({'kind': kind, 'cmd': v['cmd'], 'vanilla': v, 'rust': r})
    (build / 'mismatches.jsonl').write_text(''.join(json.dumps(m, ensure_ascii=False) + '\n' for m in mismatches), encoding='utf-8')
    summary = []
    for name, lo, hi in commands_by_group:
        # By position: two groups that run the same probe command (the orb
        # groups' `data get` lines) must not be charged each other's failures.
        failed = sum(1 for position in failed_positions if lo <= position < hi)
        summary.append({'group': name, 'commands': hi - lo, 'mismatches': failed})
        print(f'{name}: {hi - lo - failed}/{hi - lo} commands match', flush=True)
    for m in mismatches[:args.limit]:
        print(m['kind'], repr(m['cmd']))
        print('  vanilla', render(m['vanilla'])[:400])
        print('  rust   ', render(m['rust'])[:400])
    total = len(vanilla)
    print(f'total: {total - len(mismatches)}/{total} commands match', flush=True)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'seed': SEED,
              'commands': total, 'mismatches': len(mismatches), 'groups': summary,
              'comparison_overrides': [
                  {'cmd': op['cmd'], 'comparison': op['comparison'],
                   'vanilla_results': v['results'], 'rust_results': r['results']}
                  for op, v, r in zip(recorded_ops, vanilla, rust) if op.get('comparison')],
              'scope': 'Feedback and failure messages (text and resolved style), chat lines, and CommandResultCallback results of commands run by the single-player owner of an integrated server; world, entity and item effects through probe commands. Explicit positive_result operations compare presence only; their unmodified raw counts remain in the records and comparison_overrides.'}
    for override in report['comparison_overrides']:
        print(f'{override["comparison"]}: {override["cmd"]}: '
              f'Java {override["vanilla_results"]}, Sunrise {override["rust_results"]}', flush=True)
    lab.write_json(report_path, report)
    if mismatches and not args.allow_gaps:
        sys.exit(f'{len(mismatches)} command execution mismatches; see {build}')
    print(f'Command execution parity report: {report_path}')


def write_witness(path, lock, groups, reference):
    witness = {
        'minecraft': lock['minecraft']['version'],
        'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
        'seed': SEED,
        'groups': [name for name, _ in groups if name != 'setup'],
        'records': [
            {
                'cmd': row['cmd'],
                'text': ''.join(span[0] for m in row['messages'] for span in m),
                'results': row['results'],
            }
            for row in reference
        ],
    }
    lab.write_json(path, witness)
    print(f'Wrote witness: {path}')


def assemble(groups, unsynced_randoms=False):
    """The script: the groups in order, with corpus_module.group_cleanup() between two groups."""
    script = []
    for index, (name, lines) in enumerate(groups):
        if index and groups[index - 1][0] != 'setup' and name not in corpus_module.CONTINUES_PREVIOUS:
            script.extend(corpus_module.group_cleanup())
        script.extend(op for op in lines if not (unsynced_randoms and op['op'] == 'sync_randoms'))
    return script


def require_anger_reference(op, record):
    """Validate independent live-clock expectations, retaining raw absolute ends."""
    expected = op.get('expect_anger')
    if expected is None:
        return
    state = record.get('state', {})
    fields = ('uuid', 'kind', 'index')
    if (any(state.get(key) != expected[key] for key in fields) or
            state.get('serializer') != 2 or type(state.get('value')) is not int or
            type(state.get('game_time')) is not int):
        sys.exit(f'Java anger identity/type guard failed: {op}: {record}')
    value = -1 if expected['delta'] is None else state['game_time'] + expected['delta']
    if state['value'] != value:
        sys.exit(f'Java anger live-clock guard failed: {op}: {record}')


MASKS = [
    (re.compile(r'debug-trace-\d{4}-\d\d-\d\d_\d\d\.\d\d\.\d\d\.txt'), 'debug-trace-<time>.txt'),
    (re.compile(r'after [0-9.]+ second\(s\) and \d+ tick\(s\) \([0-9.,NaInfity]+ tick\(s\) per second\)'), 'after <time>'),
    (re.compile(r'Average time per tick: [0-9.]+ms'), 'Average time per tick: <ms>'),
    (re.compile(r'P50: [0-9.]+ms P95: [0-9.]+ms P99: [0-9.]+ms'), 'P50: <ms> P95: <ms> P99: <ms>'),
    (re.compile(r'debug/client-[0-9_.-]+\.(jfr|json)'), 'debug/client-<time>'),
    # BaseCommandBlock.sendSystemMessage stamps the last output with the wall clock.
    (re.compile(r'\[\d\d:\d\d:\d\d\] '), '[<time>] '),
]


GAMERULES = re.compile(r'/gamerule \(([^)]*)\)')


ATTRIBUTES_LIST = re.compile(r'attributes: \[')


def unordered_attributes(record):
    """Sort only the root attributes list in full `/data get entity` feedback.

    `AttributeMap.pack` lists the instances in the iteration order of an `Object2ObjectOpenHashMap` keyed by
    `Holder.Reference`, which keeps `Object.hashCode`: an identity hash, so the order changes with the JVM's
    startup (the same script gives another order under another main class). Each entry keeps its text and
    styles; only the entries' order is normalised. parity_fuzz compares these lists unordered too."""
    out = json.loads(json.dumps(record))
    if not re.fullmatch(r'/data get entity \S+', out.get('cmd', '')):
        return out
    for line in out.get('messages') or []:
        if not isinstance(line, list) or not all(isinstance(span, list) and len(span) == 2
                                               and isinstance(span[0], str) for span in line):
            continue
        chars = [(ch, span[1]) for span in line for ch in span[0]]
        text = ''.join(ch for ch, _ in chars)
        prefix = ' has the following entity data: '
        body = text.find(prefix)
        if body < 0 or text[body + len(prefix):body + len(prefix) + 1] != '{':
            continue
        # Find actual root keys, not quoted text, suffixes of other keys or nested custom data.
        root = body + len(prefix)
        stack, quote, matches = [], None, []
        i = root
        while i < len(text):
            ch = text[i]
            if quote:
                if ch == '\\':
                    i += 1
                elif ch == quote:
                    quote = None
            elif ch in ('"', "'"):
                quote = ch
            elif ch in '[{':
                stack.append(']' if ch == '[' else '}')
            elif ch in ']}':
                if not stack or stack.pop() != ch:
                    break
                if not stack:
                    i += 1
                    break
            elif stack == ['}'] and (i == root + 1 or text[i - 2:i] == ', '):
                match = ATTRIBUTES_LIST.match(text, i)
                if match:
                    matches.append(match)
            i += 1
        # Malformed or truncated SNBT stays exact, rather than being repaired by sorting.
        if i != len(text) or stack or quote or not matches:
            continue
        for match in reversed(matches):
            start = i = match.end()
            depth, quoted, cuts = 0, None, [start]
            while i < len(text):
                ch = text[i]
                if quoted:
                    if ch == '\\':
                        i += 1
                    elif ch == quoted:
                        quoted = None
                elif ch in ('"', "'"):
                    quoted = ch
                elif ch in '[{':
                    depth += 1
                elif ch in ']}':
                    if depth == 0:
                        break
                    depth -= 1
                elif ch == ',' and depth == 0 and text[i + 1:i + 2] == ' ':
                    cuts += [i, i + 2]
                    i += 1
                i += 1
            if len(cuts) == 1:
                continue
            cuts.append(i)
            entries = [chars[a:b] for a, b in zip(cuts[0::2], cuts[1::2])]
            if not all(entry and entry[0][0] == '{' and entry[-1][0] == '}' for entry in entries):
                continue
            separators = [chars[a:b] for a, b in zip(cuts[1::2], cuts[2::2])]
            ordered = sorted(entries, key=lambda e: json.dumps(e))
            joined = ordered[0]
            for separator, entry in zip(separators, ordered[1:]):
                joined = joined + separator + entry
            chars[start:i] = joined
            text = ''.join(ch for ch, _ in chars)
        spans = []
        for ch, style in chars:
            if spans and spans[-1][1] == style:
                spans[-1][0] += ch
            else:
                spans.append([ch, style])
        line[:] = spans
    return out


def masked(record):
    text = json.dumps(record['messages'], ensure_ascii=False)
    # GameRuleCommand registers the rules in identity-hash order, different
    # in every JVM: compare /help's gamerule alternatives as a set.
    text = GAMERULES.sub(lambda m: '/gamerule (' + '|'.join(sorted(m.group(1).split('|'))) + ')', text)
    for pattern, replacement in MASKS:
        text = pattern.sub(replacement, text)
    return text


def positive_result(record):
    """A counted operation's presence, retaining its unmodified raw record."""
    results = record.get('results')
    if not isinstance(results, list) or len(results) != 1 or not isinstance(results[0], list) or len(results[0]) != 2:
        return None
    accepted, count = results[0]
    if type(accepted) is not bool or type(count) is not int or count < 0 or not accepted and count != 0:
        return None
    return count > 0


def compare(v, r, comparison=None):
    # Explicit per-operation scope for random counts (for example explosion
    # fire). All raw feedback/results remain in the JSON and report; every
    # other command still compares exact feedback, styles and results.
    if comparison == 'positive_result':
        left, right = positive_result(v), positive_result(r)
        return None if left is not None and right is not None and left == right else 'positive_result'
    if comparison is not None:
        raise ValueError(f'unknown command comparison: {comparison}')
    if v.get("state") != r.get("state"):
        return "state"
    if masked(v) != masked(r):
        return 'messages'
    if v.get('components') != r.get('components'):
        return 'components'
    if v['results'] != r['results']:
        if v['cmd'].startswith('/debug stop'):
            return None
        return 'results'
    return None


def write_shulker_motion_witness(path, reference, lock, server_jar, script_path=None, oracle_path=None):
    # Extract exact vanilla Motion and Rotation SNBT + IEEE-754 bits from /data get records.
    readings = {}
    prefix = ' has the following entity data: '
    for row in reference:
        match = re.fullmatch(r'/data get entity (\S+) (Motion|Rotation)', row['cmd'])
        if not match:
            continue
        uid, field = match.groups()
        if len(row['messages']) != 1:
            sys.exit(f'unexpected /data get message count for {row["cmd"]}: {row["messages"]}')
        text = ''.join(span[0] for span in row['messages'][0])
        if prefix not in text:
            sys.exit(f'missing entity data prefix for {row["cmd"]}: {text!r}')
        snbt = text.split(prefix, 1)[1]
        readings.setdefault((uid, field), []).append(snbt)

    def parse_vector(snbt, count, suffix, fmt):
        if not (snbt.startswith('[') and snbt.endswith(']')):
            sys.exit(f'expected SNBT list, got {snbt!r}')
        parts = [p.strip() for p in snbt[1:-1].split(',')]
        if len(parts) != count or any(not p.lower().endswith(suffix) for p in parts):
            sys.exit(f'expected {count} {suffix}-suffixed values, got {snbt!r}')
        return [struct.pack(fmt, float(p[:-1])).hex() for p in parts]

    def sample(uid, idx):
        motion_snbt = readings[(uid, 'Motion')][idx]
        rotation_snbt = readings[(uid, 'Rotation')][idx]
        return {
            'motion_snbt': motion_snbt,
            'motion_bits': parse_vector(motion_snbt, 3, 'd', '>d'),
            'rotation_snbt': rotation_snbt,
            'rotation_bits': parse_vector(rotation_snbt, 2, 'f', '>f'),
        }

    loaded_cases = []
    for case in corpus_module.SHULKER_MOTION_LOADED_CASES:
        uid = corpus_module.shulker_motion_uuid(case['uuid_index'])
        want = 2 if case['no_ai'] else 1
        for field in ('Motion', 'Rotation'):
            if len(readings.get((uid, field), [])) != want:
                sys.exit(f'expected {want} {field} readings for {case["name"]}, got {readings.get((uid, field))}')
        entry = {
            'name': case['name'],
            'no_ai': case['no_ai'],
            'nbt': case['nbt'],
            'input_motion_bits': [struct.pack('>d', v).hex() for v in case['motion']],
            'input_rotation_bits': [struct.pack('>f', v).hex() for v in case['rotation']],
            'after_load': sample(uid, 0),
        }
        if case['no_ai']:
            entry['after_ticks'] = {'ticks': corpus_module.SHULKER_MOTION_WATER_CASE['ticks'], **sample(uid, 1)}
        loaded_cases.append(entry)

    water = corpus_module.SHULKER_MOTION_WATER_CASE
    water_uid = corpus_module.shulker_motion_uuid(water['uuid_index'])
    want_water = water['ticks'] + 1
    for field in ('Motion', 'Rotation'):
        if len(readings.get((water_uid, field), [])) != want_water:
            sys.exit(f'expected {want_water} {field} readings for {water["name"]}, got {readings.get((water_uid, field))}')
    water_case = {
        'name': water['name'],
        'no_ai': water['no_ai'],
        'trough_levels': list(water['trough_levels']),
        'shulker_trough_index': water['shulker_trough_index'],
        'input_motion_bits': [struct.pack('>d', v).hex() for v in water['motion']],
        'input_rotation_bits': [struct.pack('>f', v).hex() for v in water['rotation']],
        'ticks': [{'tick': tick, **sample(water_uid, tick)} for tick in range(want_water)],
    }

    java_version_proc = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    java_version = (java_version_proc.stderr or java_version_proc.stdout).strip().splitlines()[0]
    command = 'python3 tools/test_shulker_motion_parity.py --record'
    server_jar_sha256 = lab.digest(server_jar, 'sha256')
    witness = {
        'schema': 1,
        'minecraft': lock['minecraft']['version'],
        'oracle': 'VanillaCommandExecOracle',
        'group': corpus_module.SHULKER_MOTION_GROUP,
        'command': command,
        'server_jar_sha256': server_jar_sha256,
        'java': java_version,
        'loaded_cases': loaded_cases,
        'flowing_water_case': water_case,
    }
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(witness, indent=2) + '\n', encoding='utf-8')
    oracle_file = Path(oracle_path) if oracle_path else lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'server.jar': server_jar_sha256,
        oracle_file.relative_to(lab.ROOT).as_posix(): lab.digest(oracle_file, 'sha256'),
        'tools/command_exec_corpus.py': lab.digest(lab.ROOT / 'tools/command_exec_corpus.py', 'sha256'),
        'tools/test_command_exec_parity.py': lab.digest(lab.ROOT / 'tools/test_command_exec_parity.py', 'sha256'),
    }
    if script_path is not None and Path(script_path).is_file():
        input_hashes['script.jsonl'] = lab.digest(Path(script_path), 'sha256')
    lab.write_json(path.parent / 'meta.json', {
        'game_version': lock['minecraft']['version'],
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version,
        'harness': 'tools/test_shulker_motion_parity.py',
        'oracle': oracle_file.relative_to(lab.ROOT).as_posix(),
        'group': corpus_module.SHULKER_MOTION_GROUP,
        'witness_env': 'SUNRISE_SHULKER_MOTION_WITNESS',
        'command': command,
        'input_hashes': input_hashes,
        'outputs': {
            path.name: {
                'sha256': lab.digest(path, 'sha256'),
                'bytes': path.stat().st_size,
            },
        },
    })


def write_fangs_ticking_witness(path, game_version, cases, pause_spec, records):
    """Write the real-run vanilla ticking-gate witness JSON from VanillaCommandExecOracle output."""
    pause_records = sum(9 if s['action'] == 'schedule_remove_and_tick' else 8 for s in pause_spec['stages'])
    expected_total = 1 + 2 * len(cases) + pause_records
    if len(records) != expected_total:
        sys.exit(f'unexpected record count for ticking witness: got {len(records)}, expected {expected_total}')
    distances = records[0].get('results', [])
    if len(distances) != 2 or not all(r[0] is True for r in distances):
        sys.exit(f'unexpected server_distances record: {records[0]}')
    view_distance, simulation_distance = distances[0][1], distances[1][1]

    def parse_warmup(rec):
        text = ''.join(span[0] for msg in rec['messages'] for span in msg)
        match = re.search(r'has the following entity data: (-?\d+)$', text)
        if not match:
            sys.exit(f'could not parse Warmup from vanilla output: {rec}')
        warmup = int(match.group(1))
        if rec['results'] != [[True, warmup]]:
            sys.exit(f'Warmup callback disagreed with message: {rec}')
        return warmup

    def parse_count(rec):
        return rec['results'][0][1] if rec['results'] and rec['results'][0][0] else 0

    def parse_flag(rec):
        return bool(rec['results'] and rec['results'][0][0] and rec['results'][0][1] > 0)

    out_cases = []
    for index, case in enumerate(cases):
        present_rec = records[1 + 2 * index]
        detail_rec = records[2 + 2 * index]
        present = parse_flag(present_rec)
        entry = dict(case)
        entry['expected_present'] = present
        if case['kind'] == 'evoker_fangs':
            entry['expected_warmup'] = parse_warmup(detail_rec) if present else None
        else:
            entry['expected_spawned_items'] = parse_count(detail_rec)
        out_cases.append(entry)

    cursor = 1 + 2 * len(cases)
    out_stages = []
    for stage in pause_spec['stages']:
        entry = dict(stage)
        if stage['action'] == 'schedule_remove_and_tick':
            cb_rec = records[cursor]
            cursor += 1
            entry['expected_command_block_success'] = parse_count(cb_rec)
        else:
            entry['expected_command_block_success'] = None
        t_loaded, c_loaded, t_warmup, t_spawner, t_items, c_warmup, c_spawner, c_items = records[cursor:cursor + 8]
        cursor += 8
        entry['expected_target_loaded'] = parse_flag(t_loaded)
        entry['expected_control_loaded'] = parse_flag(c_loaded)
        entry['expected_target_warmup'] = parse_warmup(t_warmup)
        entry['expected_target_spawner_present'] = parse_flag(t_spawner)
        entry['expected_target_items'] = parse_count(t_items)
        entry['expected_control_warmup'] = parse_warmup(c_warmup)
        entry['expected_control_spawner_present'] = parse_flag(c_spawner)
        entry['expected_control_items'] = parse_count(c_items)
        out_stages.append(entry)

    pause_out = dict(pause_spec)
    pause_out['stages'] = out_stages
    payload = {
        'game_version': game_version,
        'oracle': 'VanillaCommandExecOracle',
        'command': 'python3 tools/test_fangs_ticking_gate_parity.py --record',
        'player_pos': [8.5, 230.0, 8.5],
        'view_distance': view_distance,
        'simulation_distance': simulation_distance,
        'cases': out_cases,
        'scheduled_ticket_pause': pause_out,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2) + '\n', encoding='utf-8')


def render(record):
    lines = [''.join(span[0] for span in m) for m in record['messages']]
    return json.dumps({'messages': lines, 'styles': [[span[1] for span in m] for m in record['messages']], 'results': record['results']}, ensure_ascii=False)


def write_eye_item_witness(path, version, reference):
    """Write the recorded eye_of_ender live and reloaded Item witness from a real vanilla oracle run."""
    prefix = ' has the following entity data: '
    rows = [
        f'# Recorded from real vanilla {version} (VanillaCommandExecOracle).',
        '# Command: python3 tools/test_eye_item_parity.py --record',
        f'# minecraft_version\t{version}',
        '# case\tsummon_nbt\tlive_item_snbt\treloaded_item_snbt',
    ]
    for index, (name, nbt) in enumerate(corpus_module.EYE_ITEM_CASES, 1):
        query = f'/data get entity {corpus_module.eye_item_uuid(index)} Item'
        matches = [row for row in reference if row.get('cmd') == query]
        if len(matches) != 2:
            sys.exit(f'expected 2 records (live, reloaded) for {name} ({query}), got {len(matches)}')
        values = []
        for phase, row in zip(('live', 'reloaded'), matches):
            results = row.get('results')
            if not (isinstance(results, list) and len(results) == 1 and results[0][0] is True):
                sys.exit(f'vanilla {version} {phase} Item query failed for {name}: {row}')
            messages = row.get('messages') or []
            if len(messages) != 1:
                sys.exit(f'expected 1 message line for {name} {phase}: {row}')
            text = ''.join(span[0] for span in messages[0])
            if prefix not in text:
                sys.exit(f'unexpected /data get output for {name} {phase}: {text!r}')
            snbt = text.split(prefix, 1)[1]
            if '\t' in snbt or '\n' in snbt or '\r' in snbt:
                sys.exit(f'unexpected control character in witness SNBT for {name} {phase}: {snbt!r}')
            values.append(snbt)
        rows.append(f'{name}\t{nbt}\t{values[0]}\t{values[1]}')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text('\n'.join(rows) + '\n', encoding='utf-8')
    print(f'Wrote {len(corpus_module.EYE_ITEM_CASES)} eye_of_ender Item witness rows to {path}')


if __name__ == '__main__':
    main()
