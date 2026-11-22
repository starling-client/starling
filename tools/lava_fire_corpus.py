"""Lava sets fire on every entity that moves through it (#2689): `LavaFluid.entityInside` applies LAVA_IGNITE
(`Entity.lavaIgnite`: `igniteForSeconds(15)` unless `fireImmune()`) and then `Entity::lavaHurt`, for each entity whose
move collects inside-block effects. Each kind below sits still in a lava pool (no gravity, no motion, no acceleration),
beside an air control of the same kind; their Fire and presence are read after 1, 2, 5 and 20 ticks, and the
falling block's Pos and Time (its first tick, which comes before the late pass) with the lava cell it sits in."""
try:
    from .command_exec_corpus import cmd, setup
except ImportError:
    from command_exec_corpus import cmd, setup

# (name, summon id, extra NBT): projectiles of every family the issue names, a falling block and an orb.
KINDS = [
    ('fireball', 'fireball', 'acceleration_power:0d'),
    ('small_fireball', 'small_fireball', 'acceleration_power:0d'),
    ('dragon_fireball', 'dragon_fireball', 'acceleration_power:0d'),
    ('wither_skull', 'wither_skull', 'acceleration_power:0d'),
    ('wind_charge', 'wind_charge', 'acceleration_power:0d'),
    ('breeze_wind_charge', 'breeze_wind_charge', 'acceleration_power:0d'),
    ('snowball', 'snowball', ''),
    ('egg', 'egg', ''),
    ('ender_pearl', 'ender_pearl', ''),
    ('experience_bottle', 'experience_bottle', ''),
    ('shulker_bullet', 'shulker_bullet', ''),
    ('falling_block', 'falling_block', 'BlockState:{Name:"minecraft:sand"},Time:1'),
    ('experience_orb', 'experience_orb', 'Health:5s,Value:1s,Age:0s'),
]
LAVA_Y = 201
FALLING = [k[0] for k in KINDS].index('falling_block')
AIR_Y = 206


def uuid(n):
    return f'00000a81-0000-0000-0000-{n:012x}'


def lava_fire():
    out = setup('/tick freeze', '/gamerule minecraft:spawn_mobs false',
                '/gamerule minecraft:advance_time false', '/gamemode spectator',
                '/tp @s 40.5 215 40.5 0 0')
    out += [cmd('/forceload add 32 32 79 63', expect_result=6), {'op': 'settle'}]
    for x in [32, 48, 64]:
        for z in [32, 48]:
            out.append(cmd(f'/execute if loaded {x} 200 {z}', expect_result=1))
    out += setup('/fill 32 199 32 79 199 63 stone', '/fill 32 200 32 79 210 63 air')
    actors = []
    for i, (name, kind, nbt) in enumerate(KINDS):
        x = 34 + 3 * i
        # A lava pool two blocks deep in a glass well, so the source never flows.
        out += [cmd(f'/fill {x - 1} 200 35 {x + 1} {LAVA_Y + 1} 37 glass', expect_result=27),
                cmd(f'/fill {x} 200 36 {x} {LAVA_Y} 36 lava', expect_result=2),
                # Open above the lava: a taller box (a fireball's) would otherwise reach into the glass.
                cmd(f'/setblock {x} {LAVA_Y + 1} 36 air', expect_result=1),
                cmd(f'/execute if block {x} {LAVA_Y} 36 lava[level=0]', expect_result=1)]
        extra = (',' + nbt) if nbt else ''
        for where, y, n in [('lava', f'{LAVA_Y}.2', 2 * i + 1), ('air', f'{AIR_Y}.2', 2 * i + 2)]:
            out.append(cmd(f'/summon minecraft:{kind} {x}.5 {y} 36.5 '
                           f'{{UUID:[I;2689,0,0,{n}],NoGravity:1b,Motion:[0d,0d,0d]{extra}}}', expect_result=1))
            actors.append((f'{name} {where}', n))
    for name, n in actors:
        out.append(cmd(f'/data get entity {uuid(n)} Fire'))
    for steps in [1, 1, 3, 15]:
        out += [cmd(f'/tick step {steps}', expect_result=1), {'op': 'tick', 'n': steps}]
        for name, n in actors:
            present = cmd(f'/execute if entity {uuid(n)}')
            out.append(present)
            out.append(cmd(f'/data get entity {uuid(n)} Fire'))
        # The falling block's own state each step: where it is, its Time, and the lava cell it sits in.
        x = 34 + 3 * FALLING
        out += [cmd(f'/data get entity {uuid(2 * FALLING + 1)} Pos'), cmd(f'/data get entity {uuid(2 * FALLING + 1)} Time'),
                cmd(f'/execute if block {x} {LAVA_Y} 36 lava[level=0]')]
    return out
