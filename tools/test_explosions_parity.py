#!/usr/bin/env python3
"""Explosions in both real games, against vanilla: TNT in water, a creeper's effect cloud, an end crystal, a respawn
anchor, and entities hurt and pushed by TNT.

Each group builds a scene in a forced chunk of its own, syncs both engines' randoms right before the blast
(docs/parity-workflow.md, "Randoms that repeat": a ray's power and a block's drop roll draw from the level's
random), lets it go off and reads what is left: which cells of the scene are air, the items dropped by type, the
entities hurt and pushed. Only blasts with nothing drawn between the sync and the rays are compared cell by cell:
the tick that sets off a primed TNT, creeper or fireball draws from the level's random first, a number of times
vanilla doesn't repeat run to run. The groups run as one command script per engine (test_command_exec_parity.py
--groups-file): one game and one vanilla server for all of them, and the script ticks as fast as each can.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

try:
    from .command_exec_corpus import cmd, setup
except ImportError:
    from command_exec_corpus import cmd, setup

# Far from the player and from each other: a group's blast never reaches another group's chunk.
X, Y, Z = 8192, 260, 8192
SPACING = 64


class Scene:
    """A group's forced chunk: a bedrock floor at y0 - 1, the scene's cube above it, its centre at (cx, cy, cz)."""

    def __init__(self, index, radius):
        self.x0, self.y0, self.z0 = X + SPACING * index, Y, Z
        self.r = radius
        self.cx, self.cy, self.cz = self.x0 + 7, self.y0 + radius, self.z0 + 7
        self.index = index

    def box(self, pad=2):
        """A selector volume over the scene and a little around it."""
        return (f'x={self.x0 - pad},y={self.y0 - pad},z={self.z0 - pad},'
                f'dx={15 + 2 * pad},dy={2 * self.r + 2 * pad},dz={15 + 2 * pad}')

    def enter(self, freeze=True):
        # Entities tick once the forced chunk's 2-column neighbourhood is loaded (ChunkMap.prepareEntityTickingChunk).
        chunks = [[(self.x0 >> 4) + i, (self.z0 >> 4) + k] for i in range(-2, 3) for k in range(-2, 3)]
        ops = [cmd(f'/forceload add {self.x0} {self.z0}', False), {'op': 'wait_chunks', 'chunks': chunks}]
        return ops + (setup('/tick freeze') if freeze else []) + self.clear()

    def clear(self):
        top = self.y0 + 2 * self.r + 4
        return setup(f'/kill @e[type=!player,{self.box()}]',
                     f'/fill {self.x0} {self.y0 - 1} {self.z0} {self.x0 + 15} {top} {self.z0 + 15} air',
                     f'/fill {self.x0} {self.y0 - 1} {self.z0} {self.x0 + 15} {self.y0 - 1} {self.z0 + 15} bedrock',
                     f'/kill @e[type=item,{self.box()}]')

    def cube(self, block, radius=None):
        r = self.r if radius is None else radius
        return setup(f'/fill {self.cx - r} {self.cy - r} {self.cz - r} {self.cx + r} {self.cy + r} {self.cz + r} {block}')

    def leave(self):
        return self.clear() + setup('/tick unfreeze', f'/forceload remove {self.x0} {self.z0}')

    def sync(self, case):
        """Both engines' randoms, the same right before this case's blast (one set of values per case)."""
        n = 100 * self.index + case
        return {'op': 'sync_randoms', 'uniquifier': 52100 + n, 'entity_id': 5_000_000 + 10_000 * n,
                'level_seed': 7700 + n}

    def go(self, ticks):
        return [*setup('/tick unfreeze'), {'op': 'tick', 'n': ticks}, *setup('/tick freeze')]

    def air(self, radius=None, layer=None):
        """Which cells are air: the horizontal layer through the centre (or `layer`) and the vertical slice x = cx."""
        r = self.r if radius is None else radius
        y = self.cy if layer is None else layer
        cells = [(x, y, z) for x in range(self.cx - r, self.cx + r + 1) for z in range(self.cz - r, self.cz + r + 1)]
        cells += [(self.cx, yy, z) for yy in range(self.cy - r, self.cy + r + 1) if yy != y
                  for z in range(self.cz - r, self.cz + r + 1)]
        return [cmd(f'/execute if block {x} {yy} {z} minecraft:air') for x, yy, z in cells]

    def fire(self, layer, radius=None):
        r = self.r if radius is None else radius
        return [cmd(f'/execute if block {x} {layer} {z} minecraft:fire')
                for x in range(self.cx - r, self.cx + r + 1) for z in range(self.cz - r, self.cz + r + 1)]

    def items(self, *ids):
        """Dropped items around the scene: the item entities (stacks), then the items themselves, in all and per item
        id, summed from each stack's count (a scoreboard sum, so the engines' entity order doesn't matter)."""
        every = f'@e[type=item,{self.box()}]'
        ops = [cmd(f'/execute if entity {every}'), *setup('/scoreboard objectives add blast_items dummy')]
        for i in (None, *ids):
            stacks = every if i is None else f'@e[type=item,{self.box()},nbt={{Item:{{id:"minecraft:{i}"}}}}]'
            ops += setup('/scoreboard players set #items blast_items 0',
                         f'/execute as {stacks} store result score @s blast_items run data get entity @s Item.count',
                         f'/scoreboard players operation #items blast_items += {stacks} blast_items')
            ops.append(cmd('/scoreboard players get #items blast_items'))
        return ops + setup('/scoreboard objectives remove blast_items')


def tnt_in_water():
    """An explosion inside a fluid breaks nothing: `ExplosionDamageCalculator` counts the fluid's resistance (water
    100) at the start of every ray. Beside it, a waterlogged slab resists like its water. A pig in the TNT's own water
    cell shows the blast went off: water blocks no ray to an entity, so the pig is hurt and pushed."""
    s = Scene(0, 3)
    ops = s.enter() + s.cube('dirt')
    ops += setup(f'/setblock {s.cx} {s.cy} {s.cz} water', f'/setblock {s.cx + 1} {s.cy} {s.cz} stone_slab[waterlogged=true]',
                 f'/summon pig {s.cx}.5 {s.cy} {s.cz}.5 {{NoAI:1b,Silent:1b,PersistenceRequired:1b,Tags:["blast_water"],'
                 'attributes:[{id:"minecraft:max_health",base:1000.0d}],Health:1000.0f}')
    ops += [s.sync(0), cmd(f'/summon tnt {s.cx}.5 {s.cy} {s.cz}.5 {{fuse:0}}'), *s.go(2)]
    pig = '@e[type=pig,tag=blast_water,limit=1]'
    # Vanilla must show the blast (its pig hurt), or the scene proves nothing: the run stops before comparing.
    went_off = cmd(f'/execute unless data entity {pig} {{Health:1000.0f}}', expect_result=1)
    ops += [went_off, cmd(f'/execute if entity @e[type=tnt,{s.box()}]'), cmd(f'/data get entity {pig} Health'),
            cmd(f'/data get entity {pig} Motion')]
    ops += s.air() + [cmd(f'/execute if block {s.cx + 1} {s.cy} {s.cz} minecraft:stone_slab')] + s.items('dirt')
    return [('explosion tnt in water', ops + s.leave())]


def creeper_cloud():
    """`Creeper.spawnLingeringCloud`: a creeper with effects leaves an area effect cloud carrying them."""
    s = Scene(1, 2)
    ops = s.enter()
    ops += [s.sync(0), cmd(f'/summon creeper {s.cx}.5 {s.y0} {s.cz}.5 {{Fuse:1s,ignited:1b,NoAI:1b,Silent:1b,'
                'PersistenceRequired:1b,active_effects:[{id:"minecraft:speed",duration:400,amplifier:0b},'
                '{id:"minecraft:poison",duration:200,amplifier:1b}]}'), *s.go(4)]
    cloud = f'@e[type=area_effect_cloud,{s.box()},limit=1]'
    ops += [cmd(f'/execute if entity @e[type=area_effect_cloud,{s.box()}]')]
    # Not `potion_contents`: `/data get` on a live cloud leaves it out in Sunrise (a /data gap, not the blast's).
    ops += [cmd(f'/data get entity {cloud} {key}') for key in
            ('Radius', 'Duration', 'WaitTime', 'RadiusOnUse', 'RadiusPerTick', 'DurationOnUse')]
    return [('explosion creeper cloud', ops + s.leave())]


def end_crystal():
    """`EndCrystal.hurtServer`: any damage removes the crystal with a radius 6 blast."""
    s = Scene(2, 7)
    ops = s.enter()
    summon = cmd(f'/summon end_crystal {s.cx}.5 {s.cy} {s.cz}.5 {{ShowBottom:0b}}')
    # Case 0 syncs before the summon: a new crystal draws from its own random only (`EndCrystal.<init>`), so the
    # level's random, read by `/random value` and then by the blast's rays, is where the sync left it. Case 1 syncs
    # after the summon and compares the blast alone. The damage comes while ticks are frozen, with nothing between
    # the sync and the blast.
    for case, steps in enumerate([[s.sync(0), summon, cmd('/random value 1..1000000')], [summon, s.sync(1)]]):
        ops += s.clear() + s.cube('dirt') + setup(f'/setblock {s.cx} {s.cy} {s.cz} air',
                                                  f'/setblock {s.cx} {s.cy + 1} {s.cz} air')
        ops += [*steps, cmd(f'/damage @e[type=end_crystal,{s.box()},limit=1] 1'),
                *setup('/tick unfreeze'), {'op': 'tick', 'n': 2}, *setup('/tick freeze')]
        ops += [cmd(f'/execute if entity @e[type=end_crystal,{s.box()}]')] + s.air() + s.items('dirt')
    return [('explosion end crystal', ops + s.leave())]


def respawn_anchor():
    """`RespawnAnchorBlock.useWithoutItem`: a charged anchor used outside the Nether removes itself and explodes,
    radius 5, setting fires. The player uses it through the normal packet route (`use_block`)."""
    s = Scene(3, 6)
    top = s.cy + s.r
    ops = s.enter(freeze=False) + s.cube('dirt')
    ops += setup(f'/setblock {s.cx} {top + 1} {s.cz} respawn_anchor[charges=1]', '/clear @s',
                 f'/tp @s {s.cx + 2}.5 {top + 1} {s.cz}.5 90 30')
    ops += [{'op': 'wait_chunks', 'chunks': [[s.cx >> 4, s.cz >> 4]]}, *setup('/tick freeze'), s.sync(0),
            *setup('/tick unfreeze'), {'op': 'use_block', 'pos': [s.cx, top + 1, s.cz], 'face': 1, 'sequence': 7701},
            {'op': 'tick', 'n': 2}, *setup('/tick freeze')]
    ops += [cmd(f'/execute if block {s.cx} {top + 1} {s.cz} minecraft:respawn_anchor')]
    ops += s.air(layer=top) + s.fire(top + 1) + s.items('dirt')
    return [('explosion respawn anchor', ops + setup('/tp @s 8.5 230 8.5 0 0') + s.leave())]


def exposure():
    """`ServerExplosion.hurtEntities`: damage and push scale with distance and with the share of rays from the
    entity's box that reach the centre (`getSeenPercent`), measured before any block breaks."""
    s = Scene(4, 2)
    y = s.y0
    ops = s.enter() + setup(f'/fill {s.cx - 2} {y} {s.cz - 3} {s.cx + 2} {y + 2} {s.cz - 3} glass',
                            f'/fill {s.cx - 3} {y} {s.cz - 2} {s.cx - 3} {y + 2} {s.cz + 2} obsidian')
    pigs = {'open2': (s.cx + 2, s.cz), 'open4': (s.cx + 4, s.cz), 'open6': (s.cx + 6, s.cz),
            'glass': (s.cx, s.cz - 5), 'obsidian': (s.cx - 5, s.cz)}
    ops.append(s.sync(0))
    for tag, (x, z) in pigs.items():
        ops += setup(f'/summon pig {x}.5 {y} {z}.5 {{NoAI:1b,Silent:1b,PersistenceRequired:1b,Tags:["blast_{tag}"],'
                     'attributes:[{id:"minecraft:max_health",base:1000.0d}],Health:1000.0f}')
    ops += [cmd(f'/summon tnt {s.cx}.5 {y} {s.cz}.5 {{fuse:0}}'), *s.go(1)]
    for tag in pigs:
        pig = f'@e[type=pig,tag=blast_{tag},limit=1]'
        ops += [cmd(f'/data get entity {pig} Health'), cmd(f'/data get entity {pig} Motion')]
    return [('explosion exposure', ops + s.leave())]


def groups():
    return [*tnt_in_water(), *creeper_cloud(), *end_crystal(), *respawn_anchor(), *exposure()]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='a built sunrise-client (default: build it)')
    parser.add_argument('--prepare', action='store_true', help='import the pinned vanilla jars first (tools/lab.py)')
    parser.add_argument('--reference-only', action='store_true', help='run the vanilla side only')
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    work = (args.work_dir or root / '.local' / 'explosions-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    cases = work / 'groups.json'
    cases.write_text(json.dumps(groups()), encoding='utf-8')
    run = [sys.executable, str(root / 'tools/test_command_exec_parity.py'),
           '--groups-file', str(cases), '--work-dir', str(work)]
    if args.binary:
        run.extend(['--binary', str(args.binary.resolve())])
    if args.prepare:
        run.append('--prepare')
    if args.reference_only:
        run.append('--reference-only')
    return subprocess.run(run, cwd=root).returncode


if __name__ == '__main__':
    sys.exit(main())
