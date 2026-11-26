#!/usr/bin/env python3
"""A player through a Nether portal on a dedicated server, standing still in the exit portal, against vanilla's,
seen by a vanilla client.

A vanilla dedicated server and Sunrise's (`sunrise-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival. LocatorBarClient (a player made of vanilla's protocol
code) builds a Nether portal in the Overworld and its partner in a cleared, force-loaded room of the Nether, then
`tp`s Alice into the Overworld one and leaves her there. After the portal's wait (`playersNetherPortalDefaultDelay`)
she goes to the Nether and lands in the partner, where she stays: every tick `Entity.handlePortal` counts her
`portalCooldown` down and `setAsInsidePortal`, from `LivingEntity.aiStep`'s block effects, sets it back to
`Player.getDimensionChangingDelay`, so she never goes back.

Alice's level changes, teleports and chat while she travels and stands, and the replies to her `data get` of
dimension, position and portal cooldown after it, are compared exactly.
"""
import sys

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import test_locator_bar_parity as locator

NAME = 'player-portal-cooldown-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')

NETHER = 'execute in minecraft:the_nether run'


def build(command):
    return f'cmd Alice {command}'


SCRIPT = [
    ('setup', [
        'chat on',
        # No natural spawns: a flat world's slimes attack Alice at their own random moments.
        build('gamerule spawn_monsters false'), build('gamerule spawn_mobs false'),
        build('forceload add -32 -32 32 32'),
        build('execute in minecraft:overworld run kill @e[type=minecraft:slime]'),
        # The Overworld's Nether portal along x at z 4: its inside x 0..1, y -60..-58.
        build('fill -1 -61 4 2 -57 4 minecraft:obsidian'),
        build('fill 0 -60 4 1 -58 4 minecraft:nether_portal[axis=x]'),
        # Its partner in a cleared Nether room on an obsidian floor (inside x 0..1, y 65..67, z 0).
        build(f'{NETHER} forceload add -32 -32 32 32'),
        build(f'{NETHER} fill -6 64 -6 6 72 6 minecraft:air'),
        build(f'{NETHER} fill -6 63 -6 6 63 6 minecraft:obsidian'),
        build(f'{NETHER} fill -1 64 0 2 68 0 minecraft:obsidian'),
        build(f'{NETHER} fill 0 65 0 1 67 0 minecraft:nether_portal[axis=x]'),
    ]),
    # Into the portal, standing still: she goes after the portal's 80 ticks, inside this step.
    ('enter', [build('tp @s 1.0 -60 4.5 0 0')]),
    # Standing in the exit portal, on her cooldown.
    ('stand', []),
    ('read', ['cmd Alice data get entity @s Dimension', 'cmd Alice data get entity @s Pos',
              'cmd Alice data get entity @s PortalCooldown']),
]

KEPT = ('teleport', 'respawn', 'error')


def shown(lines):
    """Alice's level changes and teleports from the `tp` into the portal on, and the replies to her reads. The
    advancement chat of the trip is left out: Sunrise sends it before the arrival's teleport (#4152)."""
    steps = {f'step={step}' for step, _ in SCRIPT if step != 'setup'}
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3 and parts[1] in steps
            and (parts[2] in KEPT or parts[2] == 'chat' and parts[1] == 'step=read')]


def check(vanilla, vanilla_all):
    respawns = [line.split(' ')[1] for line in vanilla if line.split(' ')[2] == 'respawn']
    if respawns != ['step=enter']:
        return f'vanilla sent Alice level changes in {respawns}, not one in step=enter'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'A player through a Nether portal on a dedicated server '
                               'stays in the exit portal on her cooldown.', step_seconds=6.0)
    print(f'player portal cooldown: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
