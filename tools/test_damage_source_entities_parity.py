#!/usr/bin/env python3
"""Warden, Creaking and Ghast hurt reactions to /damage's direct and causing entities, against pinned Java.

One program runs through the real command path on both sides (tools/test_command_exec_parity.py): each case
hits a fresh mob with `/damage <mob> <amount> <type> [by <direct> [from <cause>]]` and reads the hit's
result and success, Health and the mob's own reaction. The damage type varies with the entities fixed, then
the entities vary with the type fixed:

- Warden (AI on, never digging or emerging): Health, saved `anger`, `execute on target` (the brain's
  ATTACK_TARGET) and the sonic boom cooldown that `setAttackTarget` starts, including a hit the hurt
  cooldown refuses.
- Creaking bound to a real active heart (`home_pos`, the heart's `creaking` protector): the hit's result,
  Health, the bound heart's saved protector and state.
- Ghast: Health and the hit result for a player-caused LargeFireball (reflected, 1000 damage, true even when
  the hurt cooldown refuses it) against other direct entities.

The world is frozen (/tick freeze), so nothing ticks between a hit and its reads. Full bodies and the clock
are raw diagnostics (--full-diagnostic compares them too); every other record is compared bit for bit.
"""
import hashlib
import json
from pathlib import Path
import sys

import test_command_exec_parity as driver

N = 3583  # UUID [I;3583,0,0,k]
OBJ = 'h3583'


def uid(k):
    return f'{N:08x}-0000-0000-0000-{k:012x}'


def nbt_uuid(k):
    return f'UUID:[I;{N},0,0,{k}]'


QUIET = 'NoGravity:1b,Silent:1b'
MOB = QUIET + ',PersistenceRequired:1b'

# (case, damage type, direct `by`, cause `from`, cooldown) for the Warden. Each case has its own warden at
# x = 3 + 6i, z = 4.5 and its own helpers: pig_far/stand_far 10 blocks away, pig_near 3 blocks away, an arrow.
WARDEN = [
    ('type_mob_projectile', 'mob_projectile', 'pig_far', None, False),
    ('type_arrow', 'arrow', 'pig_far', None, False),
    ('type_mob_attack', 'mob_attack', 'pig_far', None, False),
    ('type_generic', 'generic', 'pig_far', None, False),
    ('arrow_from_far_pig', 'mob_attack', 'arrow', 'pig_far', False),
    ('arrow_from_near_pig', 'mob_attack', 'arrow', 'pig_near', False),
    ('arrow_alone', 'mob_attack', 'arrow', None, False),
    ('stand_direct', 'mob_attack', 'stand_far', None, False),
    ('arrow_from_far_stand', 'mob_attack', 'arrow', 'stand_far', False),
    ('creative_player_direct', 'mob_attack', 'player', None, False),
    ('no_entity', 'mob_attack', None, None, False),
    ('cooldown_refused', 'mob_attack', 'pig_far', None, True),
]
# The Creaking's: shared helpers (direct entities are not changed by a hit on a bound creaking).
CREAKING = [
    ('pig_generic', 'generic', 'pig', None, False),
    ('pig_magic', 'magic', 'pig', None, False),
    ('pig_explosion', 'explosion', 'pig', None, False),
    ('pig_thrown', 'thrown', 'pig', None, False),
    ('pig_sting', 'sting', 'pig', None, False),
    ('pig_in_fire', 'in_fire', 'pig', None, False),
    ('pig_on_fire', 'on_fire', 'pig', None, False),
    ('pig_generic_kill', 'generic_kill', 'pig', None, False),
    ('arrow_generic', 'generic', 'arrow', None, False),
    ('arrow_magic', 'magic', 'arrow', None, False),
    ('arrow_explosion', 'explosion', 'arrow', None, False),
    ('arrow_thrown', 'thrown', 'arrow', None, False),
    ('tnt_generic', 'generic', 'tnt', None, False),
    ('item_generic', 'generic', 'item', None, False),
    ('cloud_generic', 'generic', 'cloud', None, False),
    ('no_entity_generic', 'generic', None, None, False),
    ('player_direct', 'generic', 'player', None, False),
    ('tnt_from_player', 'generic', 'tnt', 'player', False),
    ('cloud_from_pig', 'generic', 'cloud', 'pig', False),
    ('second_hit_refused', 'generic', 'pig', None, True),
]
# The Ghast's: shared helpers; `fireball` and `small_fireball` are owned by the player.
GHAST = [
    ('reflected_fireball', 'fireball', 'fireball', 'player', False),
    ('reflected_generic', 'generic', 'fireball', 'player', False),
    ('reflected_explosion', 'explosion', 'fireball', 'player', False),
    ('reflected_in_fire', 'in_fire', 'fireball', 'player', False),
    ('reflected_cooldown', 'fireball', 'fireball', 'player', True),
    ('small_fireball', 'fireball', 'small_fireball', 'player', False),
    ('snowball', 'fireball', 'snowball', 'player', False),
    ('player_direct', 'fireball', 'player', None, False),
    ('fireball_from_pig', 'fireball', 'fireball', 'pig', False),
    ('fireball_alone', 'fireball', 'fireball', None, False),
    ('fireball_alone_generic', 'generic', 'fireball', None, False),
]
# Vanilla's hurtServer result for each hit (first, second): the fixture reaches the branch it names.
EXPECT = {
    **{'warden_' + c[0]: (True,) for c in WARDEN},
    'warden_cooldown_refused': (True, False),
    **{f'creaking_{c}': (True,) for c in ['pig_generic', 'pig_magic', 'pig_explosion', 'pig_thrown',
                                           'pig_sting', 'pig_generic_kill', 'arrow_generic', 'arrow_magic',
                                           'arrow_explosion', 'arrow_thrown', 'player_direct', 'tnt_from_player']},
    **{f'creaking_{c}': (False,) for c in ['tnt_generic', 'item_generic', 'cloud_generic', 'no_entity_generic',
                                            'cloud_from_pig']},
    'creaking_second_hit_refused': (True, False),
    **{f'ghast_{c}': (True,) for c in ['reflected_fireball', 'reflected_generic', 'reflected_explosion',
                                        'reflected_in_fire', 'fireball_alone_generic']},
    'ghast_reflected_cooldown': (True, True),
    **{f'ghast_{c}': (False,) for c in ['small_fireball', 'snowball', 'player_direct', 'fireball_from_pig',
                                         'fireball_alone']},
}


def program(full=False):
    ops = []

    def cmd(text, label, result=None, admission=True, diagnostic=False):
        op = {'op': 'cmd', 'cmd': text, 'label': label, 'admission': admission}
        if result is not None:
            op['expect_result'] = result
        if diagnostic and not full:
            op.update(record=False, trace=True)
        ops.append(op)

    def entity(kind, k, x, y, z, extra, label):
        cmd(f'/summon {kind} {x} {y} {z} {{{nbt_uuid(k)},{extra}}}', label + '-summon', 1)
        cmd(f'/execute as {uid(k)} if entity @s[type=minecraft:{kind}]', label + '-type', 1)
        cmd(f'/data get entity {uid(k)} UUID', label + '-UUID', 4)
        cmd(f'/data get entity {uid(k)} Pos', label + '-Pos', 3)

    def source(by, cause, refs):
        if by is None:
            return ''
        text = ' by ' + ('@s' if by == 'player' else uid(refs[by]))
        if cause is not None:
            text += ' from ' + ('@s' if cause == 'player' else uid(refs[cause]))
        return text

    def hit(label, target, amount, kind, by, cause, refs):
        for holder in ['result', 'success']:
            cmd(f'/scoreboard players set {holder} {OBJ} -99', label + '-reset-' + holder, -99)
        cmd(f'/execute store result score result {OBJ} store success score success {OBJ} '
            f'run damage {uid(target)} {amount} minecraft:{kind}{source(by, cause, refs)}', label, admission=False)
        for holder in ['result', 'success']:
            cmd(f'/scoreboard players get {holder} {OBJ}', label + '-' + holder, admission=False)

    def read(label, target, reads, admission):
        for field in reads:
            cmd(f'/data get entity {uid(target)} {field}', f'{label}-{field}', admission=admission)
        cmd(f'/data get entity {uid(target)}', label + '-body', diagnostic=True)

    cmd('/tick freeze', 'freeze')
    cmd('/gamerule spawn_mobs false', 'no-spawn')
    cmd('/gamerule advance_time false', 'no-daylight')
    cmd('/gamerule random_tick_speed 0', 'no-random')
    cmd('/difficulty normal', 'normal')
    cmd('/difficulty', 'actual-difficulty', 2)
    cmd('/forceload add 0 0 79 63', 'force', 20)
    chunks = [[x, z] for x in range(5) for z in range(4)]
    ops.append({'op': 'settle', 'required_chunks': chunks})
    for z0, z1 in [(0, 31), (32, 63)]:
        cmd(f'/fill 0 199 {z0} 79 206 {z1} barrier', f'clear-prefill-{z0}', 20480)
        cmd(f'/fill 0 199 {z0} 79 206 {z1} air', f'clear-{z0}', 20480)
    cmd('/setblock 2 206 60 barrier', 'owner-support')
    ops.append({'op': 'settle', 'required_chunks': chunks})
    cmd('/tp @s 2.5 207 60.5 0 0', 'owner-position', 1)
    cmd('/gamemode creative @s', 'owner-mode')
    cmd('/execute if entity @s[type=player,name=Player]', 'owner-type', 1)
    for field, result in [('UUID', 4), ('playerGameType', 1), ('Pos', 3)]:
        cmd(f'/data get entity @s {field}', 'owner-' + field, result)
    cmd(f'/scoreboard objectives add {OBJ} dummy', 'objective')
    cmd(f'/execute store result score start {OBJ} run time query gametime', 'clock-start', diagnostic=True)

    # Warden.hurtServer: after super.hurtServer, whatever it returned, increaseAngerAt(source.getEntity(),
    # ANGRY + 20) and, with no ATTACK_TARGET, setAttackTarget(cause) if it is living and the source is
    # direct or the cause is within 5 blocks.
    warden_reads = ['Health', 'anger', 'Brain.memories."minecraft:sonic_boom_cooldown"']
    for i, (case, kind, by, cause, cooldown) in enumerate(WARDEN):
        label, x, w = 'warden_' + case, 3 + 6 * i, 100 + i
        refs = {}
        helpers = {'pig_far': ('pig', 201, 14.5, ',NoAI:1b,' + MOB), 'pig_near': ('pig', 201, 7.5, ',NoAI:1b,' + MOB),
                   'arrow': ('arrow', 203, 9.5, ',pickup:0b,' + QUIET), 'stand_far': ('armor_stand', 201, 14.5, ',' + QUIET)}
        for j, name in enumerate(sorted({by, cause} & helpers.keys())):
            mob, y, z, extra = helpers[name]
            refs[name] = 200 + 4 * i + j
            entity(mob, refs[name], x + .5, y, z, extra[1:], f'{label}-{name}')
        entity('warden', w, x + .5, 201, 4.5, MOB, label)
        cmd(f'/execute unless data entity {uid(w)} NoAI', label + '-ai-enabled', 1)
        cmd(f'/data get entity {uid(w)} Health', label + '-start-Health', 500)

        def warden_state(phase):
            read(f'{label}-{phase}', w, warden_reads, phase == 'before')
            cmd(f'/execute as {uid(w)} on target run data get entity @s UUID', f'{label}-{phase}-target',
                admission=phase == 'before')

        warden_state('before')
        if cooldown:
            hit(label + '-first', w, 5, 'generic', None, None, refs)
            warden_state('first')
        hit(label + '-hit', w, 1, kind, by, cause, refs)
        warden_state('after')

    # Creaking.hurtServer for a heart-bound creaking: no health loss; true (and the heart told when it is
    # the protector) when the direct entity is living or a projectile, or a player is responsible.
    shared = {'pig': ('pig', 10.5, 201, ',NoAI:1b,' + MOB), 'arrow': ('arrow', 14.5, 203, ',pickup:0b,' + QUIET),
              'tnt': ('tnt', 18.5, 201, ',fuse:32767s,' + QUIET),
              'item': ('item', 22.5, 201, ',Item:{id:"minecraft:stone",count:1},PickupDelay:32767s,' + QUIET),
              'cloud': ('area_effect_cloud', 26.5, 201, ',Radius:1f,Duration:32767,' + QUIET)}
    refs = {}
    for j, name in enumerate(shared):
        mob, x, y, extra = shared[name]
        refs[name] = 400 + j
        entity(mob, refs[name], x, y, 30.5, extra[1:], f'creaking-helper-{name}')
    for i, (case, kind, by, cause, twice) in enumerate(CREAKING):
        label, x, c = 'creaking_' + case, 2 + 4 * i, 300 + i
        heart = f'{x} 201 26'
        cmd(f'/setblock {x} 200 26 pale_oak_log[axis=y]', label + '-log-below', 1)
        cmd(f'/setblock {x} 202 26 pale_oak_log[axis=y]', label + '-log-above', 1)
        cmd(f'/setblock {heart} creaking_heart[axis=y,creaking_heart_state=awake,natural=false]'
            f'{{creaking:[I;{N},0,0,{c}]}}', label + '-heart', 1)
        entity('creaking', c, x + .5, 201, 20.5, f'home_pos:[I;{x},201,26],' + MOB, label)
        cmd(f'/data get entity {uid(c)} home_pos', label + '-home_pos', 3)
        cmd(f'/data get entity {uid(c)} Health', label + '-start-Health', 1)

        def creaking_state(phase):
            admission = phase == 'before'
            read(f'{label}-{phase}', c, ['Health', 'home_pos'], admission)
            cmd(f'/data get block {heart} creaking', f'{label}-{phase}-protector', 4 if admission else None,
                admission=admission)
            cmd(f'/execute if block {heart} creaking_heart[axis=y,creaking_heart_state=awake,natural=false]',
                f'{label}-{phase}-heart-awake', 1 if admission else None, admission=admission)
            # creakingHurt's effect (the heart's emitter) is not saved: the whole block entity is a diagnostic.
            cmd(f'/data get block {heart}', f'{label}-{phase}-heart-body', diagnostic=True)

        creaking_state('before')
        if twice:
            hit(label + '-first', c, 1, kind, by, cause, refs)
            creaking_state('first')
        hit(label + '-hit', c, 1, 'magic' if twice else kind, by, cause, refs)
        creaking_state('after')

    # Ghast: a LargeFireball direct entity with a Player cause is reflected: never invulnerable, 1000 damage,
    # true whatever the base hurt returned. Anything else meets the Ghast's fire immunity.
    shared = {'fireball': ('fireball', 10.5, 202, ',Motion:[0d,0d,0d],acceleration_power:0d,' + QUIET),
              'small_fireball': ('small_fireball', 14.5, 202, ',Motion:[0d,0d,0d],acceleration_power:0d,' + QUIET),
              'snowball': ('snowball', 18.5, 202, ',Motion:[0d,0d,0d],' + QUIET),
              'pig': ('pig', 22.5, 201, ',NoAI:1b,' + MOB)}
    refs = {}
    for j, name in enumerate(shared):
        mob, x, y, extra = shared[name]
        refs[name] = 600 + j
        entity(mob, refs[name], x, y, 56.5, extra[1:], f'ghast-helper-{name}')
        if mob.endswith('fireball'):
            cmd(f'/data modify entity {uid(refs[name])} Owner set from entity @s UUID', f'ghast-helper-{name}-own',
                admission=False)
            cmd(f'/data get entity {uid(refs[name])} Owner', f'ghast-helper-{name}-Owner', admission=False)
    # Vanilla 26.3 Attributes.MAX_HEALTH permits up to 1024: the Ghast survives the first 1000-damage hit.
    sturdy = 'attributes:[{id:"minecraft:max_health",base:1024d}],Health:1024f,NoAI:1b,' + MOB
    for i, (case, kind, by, cause, twice) in enumerate(GHAST):
        label, x, g = 'ghast_' + case, 3 + 6 * i, 500 + i
        entity('ghast', g, x + .5, 201, 44.5, sturdy, label)
        read(label + '-before', g, ['Health'], True)
        if twice:
            hit(label + '-first', g, 1, kind, by, cause, refs)
            read(label + '-first', g, ['Health'], False)
        hit(label + '-hit', g, 1, kind, by, cause, refs)
        read(label + '-after', g, ['Health'], False)

    cmd(f'/execute store result score now {OBJ} run time query gametime', 'clock-end', diagnostic=True)
    cmd(f'/scoreboard players operation now {OBJ} -= start {OBJ}', 'clock-delta-operation', 0)
    cmd(f'/scoreboard players get now {OBJ}', 'clock-delta', 0)
    return ops


def main():
    full = '--full-diagnostic' in sys.argv
    if full:
        sys.argv.remove('--full-diagnostic')
    reference = None
    if '--reference' in sys.argv:
        i = sys.argv.index('--reference')
        reference = Path(sys.argv[i + 1]).resolve()
        del sys.argv[i:i + 2]
    if '--work-dir' not in sys.argv:
        sys.argv += ['--work-dir', str(driver.lab.LOCAL / 'damage-source-entities-parity')]
    build = Path(sys.argv[sys.argv.index('--work-dir') + 1]).resolve()
    ops = program(full)
    driver.corpus_module.groups = lambda: [('damage source entities', ops)]
    sys.argv.append('--allow-gaps')
    driver.main()
    recorded = [op for op in ops if op['op'] == 'cmd' and op.get('record', True)]
    java = [json.loads(s) for s in (build / 'vanilla.jsonl').read_text().splitlines()]
    assert len(java) == len(recorded), ('Java record count', len(java), len(recorded))
    rows = {op['label']: row for op, row in zip(recorded, java)}
    # The fresh reference reaches the branch each case names: a refused hit throws (false/0), an accepted one
    # stores 1 in both scores.
    for case, results in EXPECT.items():
        hits = [case + '-first', case + '-hit'] if len(results) == 2 else [case + '-hit']
        for label, accepted in zip(hits, results):
            assert rows[label]['results'] == [[accepted, int(accepted)]], ('Java hit result', label, rows[label])
            for holder in ['result', 'success']:
                assert rows[label + '-' + holder]['results'] == [[True, int(accepted)]], \
                    ('Java hit score', label, holder, rows[label + '-' + holder])
    if '--reference-only' in sys.argv:
        return
    source = reference or build / 'vanilla.jsonl'
    if reference:
        assert source.read_bytes() == (build / 'vanilla.jsonl').read_bytes(), 'fresh Java bytes changed'
    expected = [json.loads(s) for s in source.read_text().splitlines()]
    native = [json.loads(s) for s in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(expected) == len(native) == len(recorded)
    differences = [{'label': op['label'], 'expected': a, 'actual': b}
                   for op, a, b in zip(recorded, expected, native) if a != b]
    report = {'records': len(native), 'passed': len(native) - len(differences),
              'reference_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
              'comparison': 'complete raw records, no masks/tolerances', 'differences': differences}
    (build / 'strict-report.json').write_text(json.dumps(report, indent=2) + '\n')
    if not full:
        for op, a, b in zip(recorded, expected, native):
            if op['admission']:
                assert a == b, ('actual fixture admission', op, a, b)
    print(f'Strict raw records: {report["passed"]}/{report["records"]}')
    if differences:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
