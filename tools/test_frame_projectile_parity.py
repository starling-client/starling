#!/usr/bin/env python3
"""Arrows against item frames, through the command execution harness (tools/test_command_exec_parity.py).

Three frames hang on the west face of a stone wall: one holding a diamond, one empty, one fixed and holding a diamond.
An arrow with no owner flies east into each. Item frames are pickable (`BlockAttachedEntity.isPickable`), so
`AbstractArrow.onHitEntity` hurts them: `ItemFrame.hurtServer` gives the diamond up (`shouldDamageDropItem`), breaks
the empty frame (`BlockAttachedEntity.hurtServer`), and a fixed frame refuses the hit (`canHurtWhenFixed`), which
turns the arrow back at a fifth of its speed. Read after the arrows land: whether each frame is there, what it holds,
the items dropped beside it, and whether an arrow's box reaches into the wall behind it. That last read tells a frame
that let the arrow through (filled, empty) from one that took it; a fixed frame's deflected arrow drops right in front
of it, so its box still reaches the wall either way.

    test_frame_projectile_parity.py [--binary PATH] [--allow-gaps] [--reference-only]
"""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import test_command_exec_parity as driver
from tools.command_exec_corpus import cmd  # noqa: E402

# name, z, frame data
FRAMES = [
    ('filled', 2, 'Item:{id:"minecraft:diamond",count:1}'),
    ('empty', 6, ''),
    ('fixed', 10, 'Fixed:1b,Item:{id:"minecraft:diamond",count:1}'),
]


def ops():
    out = []

    def run(text, expected=None, record=True):
        out.append(cmd(text, record=record, expect_result=expected))

    for rule in ['spawn_mobs', 'advance_time', 'advance_weather', 'mob_griefing']:
        run(f'/gamerule {rule} false', record=False)
    run('/gamerule entity_drops true', record=False)
    run('/fill 0 209 0 15 209 15 minecraft:stone', 256)
    run('/fill 8 210 0 8 212 15 minecraft:stone', 48)
    run('/execute if loaded 7 210 6', 1)
    for name, z, data in FRAMES:
        comma = ',' if data else ''
        run(f'/summon minecraft:item_frame 7.5 210.5 {z}.5 {{Facing:4b,Tags:["f_{name}"]{comma}{data}}}', 1)
        run(f'/data get entity @e[tag=f_{name},limit=1] Item.id')
    for name, z, _ in FRAMES:
        run(f'/summon minecraft:arrow 4.5 210.5 {z}.5 {{Tags:["a_{name}"],Motion:[1.0d,0.0d,0.0d]}}', 1)
    out.append({'op': 'tick', 'n': 20})
    for name, z, _ in FRAMES:
        run(f'/execute if entity @e[type=minecraft:item_frame,tag=f_{name}]')
        run(f'/data get entity @e[tag=f_{name},limit=1] Item.id')
        near = f'x=5,y=209,z={z - 1},dx=4,dy=4,dz=2'
        run(f'/execute if entity @e[type=minecraft:item,{near},nbt={{Item:{{id:"minecraft:diamond"}}}}]')
        run(f'/execute if entity @e[type=minecraft:item,{near},nbt={{Item:{{id:"minecraft:item_frame"}}}}]')
        # Stuck in the wall behind the frame (the frame is 7.9375 to 8): the frame let it through.
        run(f'/execute if entity @e[type=minecraft:arrow,tag=a_{name},x=7.9,y=209,z={z},dx=0.2,dy=4,dz=1]')
    return out


def non_mob_target_ops():
    out = []

    def run(text, expected=None, record=True):
        out.append(cmd(text, record=record, expect_result=expected))

    for rule in ['spawn_mobs', 'advance_time', 'advance_weather', 'mob_griefing']:
        run(f'/gamerule {rule} false', record=False)
    run('/gamerule entity_drops true', record=False)
    run('/kill @e[type=!minecraft:player]', record=False)
    run('/fill 0 209 0 15 209 15 minecraft:stone', record=False)
    run('/fill 0 210 0 15 215 15 minecraft:air', record=False)
    run('/fill 11 210 0 11 213 15 minecraft:stone', record=False)
    # Wall at x=8 for painting and glow item frame lanes (z=2, 4).
    run('/fill 8 210 1 8 212 5 minecraft:stone', record=False)
    # Fence post at (7, 210, 12) for the tied leash knot.
    run('/setblock 7 210 12 minecraft:oak_fence', record=False)
    # Wall targets: painting (z=2) hit by snowball, glow frame (z=4) hit by egg.
    run('/summon minecraft:painting 7.5 210.5 2.5 {block_pos:[I;7,210,2],facing:1b,variant:"minecraft:kebab",Tags:["t_paint"]}', 1)
    run('/summon minecraft:glow_item_frame 7.5 210.5 4.5 {Facing:4b,Tags:["t_glow"],Item:{id:"minecraft:diamond",count:1}}', 1)
    # Armor stands: normal (z=6) by arrow, marker (z=8) by arrow, nogravity (z=10) by wind_charge.
    run('/summon minecraft:armor_stand 7.5 210.0 6.5 {Tags:["t_stand"]}', 1)
    run('/summon minecraft:armor_stand 7.5 210.0 8.5 {Marker:1b,Tags:["t_marker"]}', 1)
    run('/summon minecraft:armor_stand 7.5 210.0 10.5 {NoGravity:1b,Tags:["t_nograv"]}', 1)
    # Natural tied leash knot (z=12) by arrow, falling sand (z=14) by arrow.
    run('/summon minecraft:pig 7.5 210.0 13.5 {NoAI:1b,Silent:1b,leash:[I;7,210,12],Tags:["t_pig"]}', 1)
    out.append({'op': 'tick', 'n': 1})
    run('/summon minecraft:falling_block 7.5 211.0 14.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d],Tags:["t_fb"]}', 1)
    # Fire projectiles east from x=5.5.
    run('/summon minecraft:snowball 5.5 210.5 2.5 {Motion:[1.0d,0.0d,0.0d],Tags:["p_paint"]}', 1)
    run('/summon minecraft:egg 5.5 210.5 4.5 {Motion:[1.0d,0.0d,0.0d],Tags:["p_glow"]}', 1)
    run('/summon minecraft:arrow 5.5 210.5 6.5 {Motion:[1.0d,0.0d,0.0d],Tags:["p_stand"]}', 1)
    run('/summon minecraft:arrow 5.5 210.5 8.5 {Motion:[1.0d,0.0d,0.0d],Tags:["p_marker"]}', 1)
    run('/summon minecraft:wind_charge 5.5 210.5 10.5 {acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d],Tags:["p_nograv"]}', 1)
    run('/summon minecraft:arrow 5.5 210.6 12.5 {Motion:[1.0d,0.0d,0.0d],Tags:["p_knot"]}', 1)
    run('/summon minecraft:arrow 5.5 211.3 14.5 {Motion:[1.0d,0.0d,0.0d],Tags:["p_fb"]}', 1)
    out.append({'op': 'tick', 'n': 10})
    # Check each target and its projectile outcome.
    run('/execute if entity @e[tag=t_paint]')
    run('/execute if entity @e[type=minecraft:item,x=5,y=209,z=1,dx=4,dy=3,dz=2,nbt={Item:{id:"minecraft:painting"}}]')
    run('/execute if entity @e[tag=t_glow]')
    run('/data get entity @e[tag=t_glow,limit=1] Item.id')
    run('/execute if entity @e[type=minecraft:item,x=5,y=209,z=3,dx=4,dy=3,dz=2,nbt={Item:{id:"minecraft:diamond"}}]')
    run('/execute if entity @e[tag=t_stand]')
    run('/execute if entity @e[tag=p_stand]')
    run('/execute if entity @e[type=minecraft:item,x=5,y=209,z=5,dx=4,dy=3,dz=2,nbt={Item:{id:"minecraft:armor_stand"}}]')
    run('/execute if entity @e[tag=t_marker]')
    # Marker arrow flew past x=7.5 to the backstop at x=11 (or floor past x=8).
    run('/execute if entity @e[tag=p_marker,x=8,y=209,z=8,dx=4,dy=4,dz=1]')
    # Wind charge passed straight through NoGravity stand and burst on backstop, leaving stand alive.
    run('/execute if entity @e[tag=t_nograv]')
    run('/execute if entity @e[tag=p_nograv]')
    # Tied leash knot broke and dropped the pig's lead.
    run('/execute if entity @e[type=minecraft:leash_knot,x=6,y=209,z=11,dx=3,dy=3,dz=3]')
    run('/execute if entity @e[tag=p_knot]')
    run('/execute if entity @e[type=minecraft:item,x=5,y=209,z=11,dx=5,dy=3,dz=4,nbt={Item:{id:"minecraft:lead"}}]')
    # Falling block deflected the arrow back (x < 7.2) and still landed as sand.
    run('/execute if entity @e[tag=p_fb,x=4,y=209,z=14,dx=3.2,dy=4,dz=1]')
    run('/execute if block 7 210 14 minecraft:sand')
    return out


def projectiles_121_124_ops():
    out = []

    def run(text, expected=None, record=True):
        out.append(cmd(text, record=record, expect_result=expected))

    for rule in ['spawn_mobs', 'advance_time', 'advance_weather']:
        run(f'/gamerule {rule} false', record=False)
    run('/gamerule mob_griefing true', record=False)
    run('/gamerule entity_drops true', record=False)
    run('/kill @e[type=!minecraft:player]', record=False)
    run('/fill 0 209 0 15 209 15 minecraft:stone', record=False)
    run('/fill 0 210 0 15 215 15 minecraft:air', record=False)
    run('/fill 12 210 0 12 213 15 minecraft:stone', record=False)
    # #121: wind charge passes through End crystal (z=2) and hits pig behind it.
    run('/summon minecraft:end_crystal 7.25 210.0 2.5 {Tags:["c121"]}', 1)
    run('/summon minecraft:pig 10.5 210.0 2.5 {NoAI:1b,Silent:1b,Tags:["p121"]}', 1)
    run('/summon minecraft:wind_charge 4.5 211.0 2.5 {acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d],Tags:["w121"]}', 1)
    # #123: piercing arrow kills two mobs in the very first tick (z=8).
    run('/summon minecraft:chicken 5.5 210.0 8.5 {NoAI:1b,Silent:1b,Health:1.0f,Tags:["m123a"]}', 1)
    run('/summon minecraft:pig 6.8 210.0 8.5 {NoAI:1b,Silent:1b,Health:1.0f,Tags:["m123b"]}', 1)
    run('/summon minecraft:arrow 4.5 210.5 8.5 {Motion:[3.0d,0.0d,0.0d],damage:20.0d,PierceLevel:2b,Tags:["a123"]}', 1)
    # #124: firework rocket detonates on oak boat (z=11) and breaks nearby normal stand only (z=14).
    run('/summon minecraft:oak_boat 6.5 210.0 11.5 {Tags:["b124"]}', 1)
    run('/summon minecraft:firework_rocket 4.5 210.3 11.5 {LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],Tags:["r124b"],FireworksItem:{id:"minecraft:firework_rocket",count:1,components:{"minecraft:fireworks":{flight_duration:1,explosions:[{shape:"small_ball",colors:[I;11743532]}]}}}}', 1)
    run('/summon minecraft:armor_stand 8.5 210.0 14.5 {Tags:["s124_norm"]}', 1)
    run('/summon minecraft:armor_stand 6.5 210.0 12.5 {Invisible:1b,Tags:["s124_inv"]}', 1)
    run('/summon minecraft:armor_stand 4.5 210.0 14.5 {Marker:1b,Tags:["s124_mark"]}', 1)
    run('/summon minecraft:firework_rocket 6.5 210.5 14.5 {LifeTime:1,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],Tags:["r124s"],FireworksItem:{id:"minecraft:firework_rocket",count:1,components:{"minecraft:fireworks":{flight_duration:1,explosions:[{shape:"small_ball",colors:[I;11743532]}]}}}}', 1)
    # Check #123 after 1 single tick: both mobs are already dying (Health == 0).
    out.append({'op': 'tick', 'n': 1})
    run('/data get entity @e[tag=m123a,limit=1] Health')
    run('/data get entity @e[tag=m123b,limit=1] Health')
    # Advance 9 more ticks to settle #121 and #124.
    out.append({'op': 'tick', 'n': 9})
    run('/execute if entity @e[tag=c121]')
    run('/execute if entity @e[tag=w121]')
    run('/data get entity @e[tag=p121,limit=1] Health')
    run('/execute if entity @e[tag=r124b]')
    run('/execute if entity @e[tag=b124]')
    run('/execute if entity @e[tag=s124_norm]')
    run('/execute if entity @e[tag=s124_inv]')
    run('/execute if entity @e[tag=s124_mark]')
    # #122 runs alone so the enderman's random dodge teleport cannot land inside #124's firework blasts.
    run('/kill @e[type=!minecraft:player]', record=False)
    run('/summon minecraft:enderman 6.5 210.0 5.5 {NoAI:1b,Silent:1b,Tags:["e122"]}', 1)
    run('/summon minecraft:pig 9.5 210.0 5.5 {NoAI:1b,Silent:1b,Tags:["p122"]}', 1)
    run('/summon minecraft:trident 4.5 211.0 5.5 {Motion:[1.0d,0.0d,0.0d],Tags:["t122"]}', 1)
    out.append({'op': 'tick', 'n': 10})
    run('/data get entity @e[tag=e122,limit=1] Health')
    run('/data get entity @e[tag=p122,limit=1] Health')
    run('/execute if entity @e[tag=t122,x=8,y=209,z=5,dx=5,dy=4,dz=1]')
    return out


if __name__ == '__main__':
    driver.corpus_module.groups = lambda: [
        ('setup', driver.corpus_module.group_setup()),
        ('arrows against item frames', ops()),
        ('projectiles against non-mob pickable targets', non_mob_target_ops()),
        ('projectiles parity #121-#124', projectiles_121_124_ops()),
    ]
    if '--work-dir' not in sys.argv:
        sys.argv += ['--work-dir', str(Path(__file__).resolve().parents[1] / '.local/frame-projectile-parity')]
    driver.main()
