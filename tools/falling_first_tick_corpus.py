"""A summoned falling block or primed TNT ticks on its first tick, even when an experience orb joined after it (#2916).

`ServerLevel.tick` walks `EntityTickList` in join order, so an entity summoned between two ticks takes its first tick
on the next one, whatever joined after it. Each phase summons, between two frozen ticks, a floating falling block (its
`Time`), a falling block resting on the floor (it lands and places its block on its first tick), primed TNT (its `fuse`
and `Pos`) and primed TNT with one tick of fuse left (gone after its first tick; `tnt_explodes` is off, so nothing
explodes). The control phase summons nothing after them; the second phase summons an experience orb last. Vanilla's
values are asserted exactly after each of the first two ticks, and Sunrise must print the same."""
try:
    from .command_exec_corpus import cmd, setup
except ImportError:
    from command_exec_corpus import cmd, setup

FLOOR_Y = 200
AIR_Y = 205
Z = 4


def uuid(n):
    return f'00000b64-0000-0000-0000-{n:012x}'


def summon(kind, x, y, n, nbt):
    return cmd(f'/summon minecraft:{kind} {x}.5 {y} {Z}.5 {{UUID:[I;2916,0,0,{n}],Motion:[0d,0d,0d],{nbt}}}',
               expect_result=1)


def falling_first_tick():
    out = setup('/tick freeze', '/gamerule minecraft:spawn_mobs false', '/gamerule minecraft:advance_time false',
                '/gamerule minecraft:tnt_explodes false', '/weather clear', '/gamemode spectator',
                '/tp @s 24.5 230 24.5 0 0')
    out += [cmd('/forceload add 0 0 31 31', expect_result=4), {'op': 'settle'}]
    for x in [0, 16]:
        for z in [0, 16]:
            out.append(cmd(f'/execute if loaded {x} 200 {z}', expect_result=1))
    out += setup(f'/fill 0 {FLOOR_Y - 1} 0 31 {FLOOR_Y - 1} 31 stone', f'/fill 0 {FLOOR_Y} 0 31 {AIR_Y + 4} 31 air')
    for phase, (x0, orb_after) in enumerate([(1, False), (16, True)]):
        base = 10 * phase
        floating, landing, tnt, short_tnt, orb = (base + i for i in range(1, 6))
        land_x = x0 + 3
        out.append(cmd(f'/execute if block {land_x} {FLOOR_Y} {Z} minecraft:air', expect_result=1))
        out += [summon('falling_block', x0, AIR_Y, floating, 'BlockState:{Name:"minecraft:sand"},Time:0,NoGravity:1b'),
                summon('falling_block', land_x, FLOOR_Y, landing, 'BlockState:{Name:"minecraft:sand"},Time:0'),
                summon('tnt', x0 + 6, AIR_Y, tnt, 'fuse:80s'),
                summon('tnt', x0 + 9, AIR_Y, short_tnt, 'fuse:1s')]
        if orb_after:
            out.append(summon('experience_orb', x0 + 12, AIR_Y, orb, 'Value:1s,Health:5s,Age:0s,NoGravity:1b'))
        for ticks in [1, 2]:
            out += [cmd('/tick step 1', expect_result=1), {'op': 'tick', 'n': 1}]
            out += [cmd(f'/data get entity {uuid(floating)} Time', expect_result=ticks),
                    cmd(f'/data get entity {uuid(floating)} Pos'),
                    cmd(f'/execute unless entity {uuid(landing)}', expect_result=1),
                    cmd(f'/execute if block {land_x} {FLOOR_Y} {Z} minecraft:sand', expect_result=1),
                    cmd(f'/data get entity {uuid(tnt)} fuse', expect_result=80 - ticks),
                    cmd(f'/data get entity {uuid(tnt)} Pos'),
                    cmd(f'/data get entity {uuid(tnt)} Motion'),
                    cmd(f'/execute unless entity {uuid(short_tnt)}', expect_result=1)]
            if orb_after:
                out.append(cmd(f'/execute if entity {uuid(orb)}', expect_result=1))
        out += setup(f'/kill {uuid(floating)}', f'/kill {uuid(tnt)}')
        if orb_after:
            out += setup(f'/kill {uuid(orb)}')
    return out
