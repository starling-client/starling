#!/usr/bin/env python3
"""Compare a mob's and an armor stand's saved `attributes` (a base, a permanent modifier with its id) and
the item modifiers of LivingEntity.collectEquipmentChanges with vanilla.

Each case loads an entity from NBT, equips it, runs the equipment pass (twice for a swap) and prints the
attribute values and bases asked for plus the saved `attributes` list as exact double bits. The item
modifiers are transient, so they show in the values and never in the saved list."""
import argparse
import json
import os
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

POS = 'Pos:[0.0d,64.0d,0.0d],Motion:[0.0d,0.0d,0.0d],Rotation:[0.0f,0.0f]'
BONUS = '{id:"test:bonus",amount:0.5d,operation:"add_multiplied_base"}'
LEADER_BONUS = '{id:"minecraft:leader_zombie_bonus",amount:10.0d,operation:"add_value"}'


def case(name, entity, attributes, equip=None, then=None, read=(), health='20.0f', extra='', check_health=False):
    parts = [f'id:"{entity}"', POS]
    if health is not None:
        parts.append(f'Health:{health}')
    if extra:
        parts.append(extra)
    if attributes:
        parts.append(f'attributes:[{",".join(attributes)}]')
    out = dict(name=name, entity=entity, snbt='{' + ','.join(parts) + '}', equip=equip or {}, then=then or {}, read=list(read))
    if check_health:
        out['health'] = True
    return out


ZOMBIE, STAND = 'minecraft:zombie', 'minecraft:armor_stand'
LOADED_HEALTH_CASES = [
    # Loaded Health clamped to [0, getMaxHealth()] after attributes and cube sizing (vanilla 26.3, #2360).
    case('loaded_health_endermite_over_max', 'minecraft:endermite', [], health='20.0f', check_health=True),
    case('loaded_health_parched_over_max', 'minecraft:parched', [], health='20.0f', check_health=True),
    case('loaded_health_zombie_bonus_kept', ZOMBIE,
         [f'{{id:"minecraft:max_health",base:20.0d,modifiers:[{LEADER_BONUS}]}}'],
         health='25.0f', check_health=True),
    case('loaded_health_zombie_bonus_over_max', ZOMBIE,
         [f'{{id:"minecraft:max_health",base:20.0d,modifiers:[{LEADER_BONUS}]}}'],
         health='40.0f', check_health=True),
    case('loaded_health_pig_negative', 'minecraft:pig', [], health='-3.0f', check_health=True),
    case('loaded_health_pig_absent', 'minecraft:pig', [], health=None, check_health=True),
    case('loaded_health_endermite_below_max', 'minecraft:endermite', [], health='3.5f', check_health=True),
    case('loaded_health_slime_size1_over_max', 'minecraft:slime', [], health='20.0f', extra='Size:1', check_health=True),
    case('loaded_health_magma_cube_size0_over_max', 'minecraft:magma_cube', [], health='20.0f', extra='Size:0', check_health=True),
    case('loaded_health_sulfur_cube_size0_over_max', 'minecraft:sulfur_cube', [], health='20.0f', extra='Size:0', check_health=True),
    case('loaded_health_sulfur_cube_size1_bonus_kept', 'minecraft:sulfur_cube',
         ['{id:"minecraft:max_health",base:8.0d,modifiers:[{id:"test:boost",amount:6.0d,operation:"add_value"}]}'],
         health='12.0f', extra='Size:1', check_health=True),
    # A cube sizes before the saved attributes load, so a raised or default-valued saved base wins.
    case('loaded_health_slime_size1_raised_base_kept', 'minecraft:slime',
         ['{id:"minecraft:max_health",base:50.0d}'], health='30.0f', extra='Size:1', check_health=True),
    case('loaded_health_magma_cube_size0_raised_base_over_max', 'minecraft:magma_cube',
         ['{id:"minecraft:max_health",base:10.0d}'], health='20.0f', extra='Size:0', check_health=True),
    case('loaded_health_slime_size0_default_base_kept', 'minecraft:slime',
         ['{id:"minecraft:max_health",base:20.0d}'], health='15.0f', extra='Size:0', check_health=True),
]
CASES = [
    case('zombie_defaults', ZOMBIE, [], read=['minecraft:max_health', 'minecraft:attack_damage', 'minecraft:armor']),
    # A base override and a non-transient modifier with an id on a modelled attribute, a base and a
    # modifier on one the game does not model (step_height), and an attribute the type lacks (luck).
    case('zombie_saved', ZOMBIE, [
        f'{{id:"minecraft:max_health",base:40.0d,modifiers:[{BONUS}]}}',
        '{id:"minecraft:attack_damage",base:5.0d}',
        '{id:"minecraft:step_height",base:1.5d,modifiers:[{id:"test:step",amount:0.25d,operation:"add_value"}]}',
        '{id:"minecraft:luck",base:2.0d}',
    ], read=['minecraft:max_health', 'minecraft:attack_damage', 'minecraft:step_height', 'minecraft:luck']),
    case('zombie_sword', ZOMBIE, [], equip={'mainhand': 'minecraft:diamond_sword'}, read=['minecraft:attack_damage']),
    case('zombie_sword_swapped', ZOMBIE, ['{id:"minecraft:attack_damage",base:5.0d}'],
         equip={'mainhand': 'minecraft:diamond_sword'}, then={'mainhand': 'minecraft:netherite_sword'},
         read=['minecraft:attack_damage']),
    case('zombie_sword_put_away', ZOMBIE, [], equip={'mainhand': 'minecraft:diamond_sword'}, then={'mainhand': ''},
         read=['minecraft:attack_damage']),
    case('zombie_armor', ZOMBIE, [f'{{id:"minecraft:max_health",base:40.0d,modifiers:[{BONUS}]}}'], equip={
        'head': 'minecraft:iron_helmet', 'chest': 'minecraft:iron_chestplate', 'feet': 'minecraft:netherite_boots',
        'offhand': 'minecraft:diamond_sword',
    }, read=['minecraft:armor', 'minecraft:armor_toughness', 'minecraft:knockback_resistance', 'minecraft:attack_damage']),
    case('zombie_sword_on_head', ZOMBIE, [], equip={'head': 'minecraft:diamond_sword'}, read=['minecraft:attack_damage']),
    # A ticked stand has the movement_speed instance LivingEntity.removeFrost makes on its first tick.
    case('stand_ticked', STAND, ['{id:"minecraft:movement_speed",base:0.7d}'], read=['minecraft:max_health', 'minecraft:armor', 'minecraft:attack_damage']),
    case('stand_saved', STAND, [
        f'{{id:"minecraft:max_health",base:30.0d,modifiers:[{BONUS}]}}',
        '{id:"minecraft:movement_speed",base:0.25d}',
    ], equip={'head': 'minecraft:diamond_helmet'}, read=['minecraft:max_health', 'minecraft:armor', 'minecraft:movement_speed']),
    *LOADED_HEALTH_CASES,
]


def format_loaded_health_witness(version, vanilla_lines):
    by_name = {}
    for line in vanilla_lines:
        parts = line.split('\t')
        if any(p.startswith('FAIL ') for p in parts[1:]):
            raise SystemExit(f'Java oracle failed: {line}')
        by_name[parts[0]] = parts[1:]
    rows = [
        f'# Vanilla {version} mob loaded Health witness (recorded by tools/test_mob_attribute_nbt_parity.py).',
        '# Columns: name\tentity\tdefault_max_health_bits\tloaded_max_health_bits\tloaded_health_bits\tsaved_health_bits\tsnbt',
    ]
    for c in LOADED_HEALTH_CASES:
        tokens = by_name[c['name']]
        health_tok = next((t for t in tokens if t.startswith('health ')), None)
        if health_tok is None:
            raise SystemExit(f'Missing health output for {c["name"]}: {tokens}')
        _, loaded_health, loaded_max, saved_health, default_max = health_tok.split(' ')
        rows.append('\t'.join([c['name'], c['entity'], default_max, loaded_max, loaded_health, saved_health, c['snbt']]))
    return '\n'.join(rows) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-mob-attribute-nbt-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaMobAttributeNbtOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))
    cases = build / 'cases.jsonl'
    cases.write_text(''.join(json.dumps(c) + '\n' for c in CASES), encoding='utf-8')

    def run(cmd, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            subprocess.run(list(map(str, cmd)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]), 'dev.sunriselab.parity.VanillaMobAttributeNbtOracle', cases, build / 'vanilla.txt'], 'java')
    vanilla = build.joinpath('vanilla.txt').read_text(encoding='utf-8').splitlines()
    # The recorded loaded-health witness stays in .local; the Rust test reads it from SUNRISE_MOB_LOADED_HEALTH_WITNESS.
    witness = build / 'mob_loaded_health.tsv'
    witness.write_text(format_loaded_health_witness(lock['minecraft']['version'], vanilla), encoding='utf-8')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-client-lib', '--example', 'client_probe'], 'compile-rust')
    binary = lab.cargo_target() / 'fast/examples/client_probe'
    run([binary, 'mob_attribute_nbt_fixture', lab.RESOURCES, cases, build / 'rust.txt'], 'rust')
    rust = build.joinpath('rust.txt').read_text(encoding='utf-8').splitlines()
    problems = []
    for v, r in zip(vanilla, rust):
        if v == r:
            continue
        v, r = v.split('\t'), r.split('\t')
        problems.append(f'{v[0]}:')
        problems += [f'  vanilla only: {x}' for x in v[1:] if x not in r[1:]]
        problems += [f'  rust only:    {x}' for x in r[1:] if x not in v[1:]]
    if len(vanilla) != len(rust):
        problems.append(f'vanilla has {len(vanilla)} cases, rust {len(rust)}')
    if problems:
        print('\n'.join(problems))
        raise SystemExit('mob attribute NBT divergences')
    test = 'mobs::finalize::load_tests::a_loaded_health_is_clamped_to_the_loaded_max_health'
    with (build / 'loaded-health-test.log').open('w', encoding='utf-8') as log:
        result = subprocess.run(['cargo', 'test', '-p', 'sunrise-client-lib', '--lib', '--', '--ignored', '--exact', test],
                                cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
                                env={**os.environ, 'SUNRISE_MOB_LOADED_HEALTH_WITNESS': str(witness)})
    if result.returncode != 0:
        raise SystemExit(f'{test} failed against the recorded witness; see {build / "loaded-health-test.log"}')
    print(f'Mob attribute NBT parity passed ({len(CASES)} cases, loaded-health test against {witness})')


if __name__ == '__main__':
    main()
