"""Which attribute instances a living entity saves, before and after a tick.

AttributeMap.pack saves the instances that exist. AttributeMap.getInstance
creates one (computeIfAbsent); getValue of an absent one does not. A fresh
entity has none. Its first tick creates movement_speed (LivingEntity.aiStep
-> removeFrost -> getAttribute), a drowning tick creates oxygen_bonus
(LivingEntity.decreaseAirSupply -> getAttribute), and an /attribute query
creates the one it names.
"""
import json
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.command_exec_corpus import cmd

COMMON = 'NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b'
# Instances the mob's own load makes (vanilla 26.3, before any tick): an enderman's movement_speed
# (Enderman.setTarget, from readPersistentAngerSaveData), an untamed wolf's max_health
# (TamableAnimal.readAdditionalSaveData -> setTame(false, true) -> applyTamingSideEffects) and a baby
# goat's or hoglin's attack_damage (setAge -> ageBoundaryReached).
# `armor_stand_empty` is a stand whose summon NBT already says `attributes:[]` (a save made before its
# first tick): vanilla still saves movement_speed (base 0.7d) once the first tick has made the instance.
KINDS = {'pig': COMMON, 'allay': COMMON, 'armor_stand': 'NoGravity:1b',
         'armor_stand_empty': 'NoGravity:1b,attributes:[]', 'enderman': COMMON, 'wolf': COMMON,
         'goat': COMMON + ',Age:-24000', 'hoglin': COMMON + ',Age:-24000', 'rabbit': COMMON}
IDS = ['movement_speed', 'max_health', 'follow_range', 'flying_speed', 'armor', 'scale', 'attack_damage',
       'oxygen_bonus']

# Underwater oxygen_bonus save cases (vanilla 26.3, #2679): (name, entity, medium, effect, stored, ticks, pos).
OXYGEN_BONUS_CASES = [
    ('pig_water_pre', 'minecraft:pig', 'water', None, None, 0, (13.5, 200, 13.5)),
    ('pig_water', 'minecraft:pig', 'water', None, None, 1, (9.5, 200, 9.5)),
    ('squid_water', 'minecraft:squid', 'water', None, None, 1, (11.5, 200, 9.5)),
    ('iron_golem_water', 'minecraft:iron_golem', 'water', None, None, 1, (13.5, 200, 10.5)),
    ('pig_air', 'minecraft:pig', 'air', None, None, 1, (6.5, 200, 8.5)),
    ('pig_water_breathing', 'minecraft:pig', 'water', 'minecraft:water_breathing', None, 1, (9.5, 200, 11.5)),
    ('pig_water_conduit', 'minecraft:pig', 'water', 'minecraft:conduit_power', None, 1, (11.5, 200, 11.5)),
    ('pig_water_nautilus', 'minecraft:pig', 'water', 'minecraft:breath_of_the_nautilus', None, 1, (9.5, 200, 13.5)),
    ('pig_bubble_column', 'minecraft:pig', 'bubble_column', None, None, 1, (1.5, 200, 13.5)),
    ('pig_water_stored_bonus', 'minecraft:pig', 'water', None, 2.0, 1, (11.5, 200, 13.5)),
]


def uuid(n):
    return f'[I;105,0,0,{n}]'


def mob_attribute_save(raw=False):
    ops = []

    def run(text, expected=None, record=True):
        ops.append(cmd(text, record=record, expect_result=expected))

    run('/fill 0 199 0 15 199 15 minecraft:stone', 256)
    run('/fill 8 200 8 15 204 15 minecraft:glass', 320)
    run('/fill 9 200 9 14 204 14 minecraft:water', 180)
    run('/setblock 1 199 13 minecraft:soul_sand', 1)
    run('/fill 0 200 12 2 204 14 minecraft:glass', 45)
    run('/fill 1 200 13 1 204 13 minecraft:bubble_column[drag=false]', 5)
    run('/execute if loaded 4 200 4', 1)
    run('/scoreboard objectives add h105 dummy', 1)

    def admit(tag, kind):
        # Routes that save or test the entity; none of them makes an attribute instance.
        run(f'/execute if entity @e[tag={tag},type=minecraft:{kind}]', 1)
        run(f'/data get entity @e[tag={tag},limit=1] UUID', 4)
        run(f'/data get entity @e[tag={tag},limit=1] Health')
        run(f'/data get entity @e[tag={tag},limit=1] Pos')
        run(f'/data get entity @e[tag={tag},limit=1] NoGravity')

    def attributes(tag):
        run(f'/execute store result score {tag} h105 run data get entity @e[tag={tag},limit=1] attributes', record=False)
        run(f'/scoreboard players get {tag} h105')
        for a in IDS:
            run(f'/execute if data entity @e[tag={tag},limit=1] attributes[{{id:"minecraft:{a}"}}]')
            run(f'/data get entity @e[tag={tag},limit=1] attributes[{{id:"minecraft:{a}"}}].base')
        run(f'/data get entity @e[tag={tag},limit=1] attributes[{{id:"minecraft:max_health"}}].modifiers')
        if raw:
            run(f'/data get entity @e[tag={tag},limit=1] attributes')
            run(f'/data get entity @e[tag={tag},limit=1]')

    actors = []
    n = 0
    for phase in ['pre', 'tick']:
        for kind, fields in KINDS.items():
            n += 1
            tag = f'h105_{phase}_{kind}'
            entity = kind.removesuffix('_empty')
            actors.append((phase, tag, entity))
            run(f'/summon minecraft:{entity} {1.5 + n} 200 3.5 {{UUID:{uuid(n)},Tags:["{tag}"],{fields}}}', 1)
    run(f'/summon minecraft:pig 2.5 200 8.5 {{UUID:{uuid(20)},Tags:["h105_read"],{COMMON}}}', 1)
    saved = ('attributes:[{id:"minecraft:max_health",base:20d,'
             'modifiers:[{id:"h105:extra",amount:2d,operation:"add_value"}]}]')
    run(f'/summon minecraft:pig 4.5 200 8.5 {{UUID:{uuid(21)},Tags:["h105_saved"],{COMMON},{saved}}}', 1)

    oxygen_actors = []
    for idx, (name, entity, medium, effect, stored, ticks, (x, y, z)) in enumerate(OXYGEN_BONUS_CASES, 30):
        tag = f'h105_ox_{name}'
        parts = [f'UUID:{uuid(idx)}', f'Tags:["{tag}"]', COMMON]
        if effect is not None:
            parts.append(f'active_effects:[{{id:"{effect}",duration:100,amplifier:0b}}]')
        if stored is not None:
            parts.append(f'attributes:[{{id:"minecraft:oxygen_bonus",base:{stored}d}}]')
        body = '{' + ','.join(parts) + '}'
        run(f'/summon {entity} {x} {y} {z} {body}', 1)
        oxygen_actors.append((name, tag, entity.removeprefix('minecraft:'), ticks))

    # Before any tick.
    run('/execute store result score #t0 h105 run time query gametime', record=False)
    for phase, tag, kind in actors:
        admit(tag, kind)
    admit('h105_read', 'pig')
    admit('h105_saved', 'pig')
    for name, tag, kind, ticks in oxygen_actors:
        admit(tag, kind)
    for phase, tag, kind in actors:
        if phase == 'pre':
            attributes(tag)
    for name, tag, kind, ticks in oxygen_actors:
        if ticks == 0:
            attributes(tag)
    attributes('h105_saved')
    # Magma cube load writes armor from the unclamped size. Read the saved base
    # without opening a live attribute instance via /attribute.
    for n, (size, armor) in enumerate([(-1, 0), (-5, -12), (199, 600)], 22):
        tag = f'h105_magma_{n}'
        run(f'/summon minecraft:magma_cube 4.5 240 4.5 {{UUID:{uuid(n)},Tags:["{tag}"],{COMMON},Size:{size}}}', 1)
        run(f'/data get entity @e[tag={tag},limit=1] attributes[{{id:"minecraft:armor"}}].base', armor)
    # A query makes the instance it reads.
    run('/attribute @e[tag=h105_read,limit=1] minecraft:max_health get')
    attributes('h105_read')
    run('/execute store result score #t1 h105 run time query gametime', record=False)
    run('/scoreboard players operation #t1 h105 -= #t0 h105', record=False)
    run('/scoreboard players get #t1 h105')

    # Exactly one tick.
    ops.append({'op': 'tick', 'n': 1})
    run('/execute store result score #t2 h105 run time query gametime', record=False)
    run('/scoreboard players operation #t2 h105 -= #t0 h105', record=False)
    run('/scoreboard players get #t2 h105')
    for phase, tag, kind in actors:
        if phase == 'tick':
            attributes(tag)
    for name, tag, kind, ticks in oxygen_actors:
        if ticks == 1:
            attributes(tag)
    attributes('h105_saved')
    attributes('h105_read')
    return ops


def _plain_text(record):
    return ''.join(span[0] for line in record.get('messages', []) for span in line)


def _parse_attributes_snbt(snbt):
    """Parse vanilla `/data get entity ... attributes` SNBT list into sorted (id, base_bits) rows."""
    text = snbt.strip()
    if text == '[]':
        return []
    if not (text.startswith('[') and text.endswith(']')):
        raise ValueError(f'unexpected attributes SNBT: {snbt!r}')
    rows = []
    # Each entry is `{base: <num>d, id: "<id>"}` (or with `modifiers: [...]`).
    for match in re.finditer(r'\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\}', text[1:-1]):
        entry = match.group(0)
        id_match = re.search(r'\bid:\s*"([^"]+)"', entry)
        base_match = re.search(r'\bbase:\s*([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)d', entry)
        if not id_match or not base_match:
            raise ValueError(f'could not parse attribute entry: {entry!r}')
        value = float(base_match.group(1))
        bits = struct.pack('>d', value).hex()
        rows.append({'id': id_match.group(1), 'base_bits': bits})
    rows.sort(key=lambda r: r['id'])
    return rows


def record_oxygen_bonus_witness(reference, version, destination):
    """Extract the vanilla 26.3 underwater oxygen_bonus witness from VanillaCommandExecOracle records."""
    by_cmd = {row['cmd']: row for row in reference}
    cases = []
    for name, entity, medium, effect, stored, ticks, _ in OXYGEN_BONUS_CASES:
        tag = f'h105_ox_{name}'
        count_cmd = f'/scoreboard players get {tag} h105'
        attrs_cmd = f'/data get entity @e[tag={tag},limit=1] attributes'
        if count_cmd not in by_cmd or attrs_cmd not in by_cmd:
            raise ValueError(f'missing oracle records for {tag}; run with --mob-attribute-save-raw-diagnostic')
        count_results = by_cmd[count_cmd].get('results')
        if not count_results or len(count_results) != 1 or not count_results[0][0]:
            raise ValueError(f'unexpected count record for {tag}: {by_cmd[count_cmd]}')
        expected_count = count_results[0][1]
        text = _plain_text(by_cmd[attrs_cmd])
        marker = ' has the following entity data: '
        if marker not in text:
            raise ValueError(f'unexpected attributes feedback for {tag}: {text!r}')
        attrs = _parse_attributes_snbt(text.split(marker, 1)[1])
        if len(attrs) != expected_count:
            raise ValueError(f'parsed {len(attrs)} attributes for {tag}, expected {expected_count}: {text!r}')
        cases.append({
            'name': name,
            'entity': entity,
            'medium': medium,
            'effect': effect,
            'stored_oxygen_bonus_bits': None if stored is None else struct.pack('>d', stored).hex(),
            'ticks': ticks,
            'attributes': attrs,
        })
    witness = {
        'minecraft': version,
        'oracle': 'VanillaCommandExecOracle',
        'harness': 'python3 tools/test_mob_oxygen_bonus_parity.py',
        'cases': cases,
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(witness, indent=2) + '\n', encoding='utf-8')
