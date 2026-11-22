#!/usr/bin/env python3
"""A connected player that moves up while it falls ends its fall.

`ServerGamePacketListenerImpl.handlePlayerPositionChange` calls `ServerPlayer.resetFallDistance` for a position
above the last one: the fall distance goes back to 0, and `fall_from_height` is awarded for the distance fallen
until then.

A real protocol client joins a vanilla 26.3 dedicated server and Sunrise's as a survival player in the air. The
world is frozen and stepped by hand. One position down, then one up, establish the single-report behavior.
A down/up/down batch ends the old fall at the middle report and starts a new one at the last. A landing
followed by a rise must damage the player and grant the landing criterion, not use the position of the rise.
The two servers must give the same fall distance, position, health and `fall_from_height` advancements.

    python3 tools/test_guest_upward_fall_reset_parity.py
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

import lab
import test_locator_bar_parity as net
from test_dedicated_server_parity import game_time
from test_disconnect_crafting_parity import generate
from test_machine_recipe_placement_parity import Commands

PLAYER = 'FallGuest'
ADVANCEMENTS = {'any': None, 'short': {'max': 1.0}, 'five': {'min': 4.9, 'max': 5.1}, 'landing': 3.5}
MOVES = [('down', 109.0), ('up', 109.5)]


def pack(world):
    root = world / 'datapacks/fall'
    root.mkdir(parents=True)
    (root / 'pack.mcmeta').write_text(json.dumps({'pack': {
        'description': 'fall reset', 'pack_format': 121, 'min_format': [121, 0], 'max_format': [121, 0]}}))
    for name, distance in ADVANCEMENTS.items():
        criterion = {'trigger': 'minecraft:fall_from_height'}
        if distance:
            criterion['conditions'] = {'distance': {'y': distance}}
        path = root / 'data/fall/advancement' / f'{name}.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({'criteria': {name: criterion}}))


def step(server):
    """One world step; the player's own ticks go on while the world is frozen."""
    before = game_time(server)
    server.console('tick step 1')
    deadline = time.monotonic() + 30
    while game_time(server) < before + 1:
        assert time.monotonic() < deadline, 'the world step did not happen'
        time.sleep(.02)


def observe(commands):
    """The player's fall distance, position, health and advancements from the console."""
    def data(path):
        text = commands.reply(f'data get entity {PLAYER} {path}')
        return re.search(r'has the following entity data: (.*)', text).group(1).strip()
    done = {name: 'Test passed' in commands.reply(
        f'execute if entity @a[name={PLAYER},advancements={{fall:{name}=true}}]') for name in ADVANCEMENTS}
    return {'fall_distance': data('fall_distance'), 'Pos': data('Pos'), 'Health': data('Health'),
            'advancements': done}


def run(side, command, client_command, out, world):
    directory = out / side
    port = net.server_dir(directory, world)
    server = net.Server(command, directory, side)
    commands = Commands(server, out / f'{side}-commands.json')
    client = None
    seen = {}
    with (out / f'{side}-client.log').open('w') as log:
        try:
            server.wait_for(r'Done \(')
            commands.reply('tick freeze')
            client = net.Client(client_command, port, out / f'{side}-wire.txt', log)
            for option in ['chat on', 'flat-chat on', 'state on', 'players on']:
                client.tell(option)
            client.tell('join ' + PLAYER)
            client.sync([PLAYER])
            for text in ['gamerule show_advancement_messages false', 'gamerule natural_health_regeneration false',
                         f'gamemode survival {PLAYER}', f'tp {PLAYER} 12.5 110 12.5 0 0']:
                commands.reply(text)
            client.sync([PLAYER])
            step(server)
            seen['start'] = observe(commands)
            for label, y in MOVES:
                client.tell(f'fallmove {PLAYER} 12.5 {y} 12.5 false')
                client.sync([PLAYER])
                step(server)
                client.sync([PLAYER])
                seen[label] = observe(commands)

            for name in ('any', 'short'):
                commands.reply(f'advancement revoke {PLAYER} only fall:{name}')
            client.tell(f'fallmove {PLAYER} 12.5 108.5 12.5 false')
            client.sync([PLAYER])
            step(server)

            # All three positions go in one write, with no world step or sync between them.
            client.tell(f'fallmove {PLAYER} 12.5 104 12.5 false 12.5 104.5 12.5 false 12.5 103.5 12.5 false')
            client.sync([PLAYER])
            seen['down-up-down'] = observe(commands)
            step(server)  # Track the new fall from the final position.

            client.tell(f'fallmove {PLAYER} 12.5 100 12.5 true 12.5 100.5 12.5 false')
            client.sync([PLAYER])
            seen['landing-then-rise'] = observe(commands)
            client.end()
            client = None
        finally:
            if client:
                client.kill()
            server.stop()
    return seen


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Sunrise game (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'guest-upward-fall-reset-parity')
    args = parser.parse_args()
    lock, jars, libraries = net.base.verify_artifacts()
    out = args.out.resolve()
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    classes = out / 'classes'
    classes.mkdir()
    source = lab.ROOT / 'mods/sunrise/src/test/java'
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn',
                    '-sourcepath', source, '-cp', cp, '-d', classes,
                    source / 'dev/sunriselab/parity/LocatorBarClient.java'], check=True)
    client = ['java', '-Xmx512m', '-cp', str(classes) + os.pathsep + cp, 'dev.sunriselab.parity.LocatorBarClient']
    vanilla = ['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', cp, 'net.minecraft.server.Main', 'nogui']
    if args.binary:
        binary = args.binary.resolve(strict=True)
    else:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client'],
                       cwd=lab.ROOT, check=True)
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
        binary = Path(metadata['target_directory']) / 'fast' / (
            'sunrise-client.exe' if os.name == 'nt' else 'sunrise-client')
    sunrise = [binary, '--resources', lab.RESOURCES.resolve(), '--dedicated', '--nogui']
    net.PLAYERS = [PLAYER]
    net.PROPERTIES.update({'force-gamemode': 'false'})
    world = generate(vanilla, out)
    pack(world)
    seen = {side: run(side, command, client, out, world)
            for side, command in [('vanilla', vanilla), ('sunrise', sunrise)]}
    lab.write_json(out / 'report.json', seen)
    # Admission: on vanilla the player was falling before it moved up, and the move up ended that fall.
    falling = seen['vanilla']['down']
    assert falling['fall_distance'] == '1.0d' and not any(falling['advancements'].values()), falling
    risen = seen['vanilla']['up']
    assert risen['fall_distance'] == '0.0d', risen
    assert risen['advancements'] == {'any': True, 'short': True, 'five': False, 'landing': False}, risen
    batch = seen['vanilla']['down-up-down']
    assert batch['fall_distance'] == '1.0d' and batch['Pos'] == '[12.5d, 103.5d, 12.5d]', batch
    assert batch['advancements'] == {'any': True, 'short': False, 'five': False, 'landing': False}, batch
    landed = seen['vanilla']['landing-then-rise']
    assert landed['fall_distance'] == '0.0d' and landed['Pos'] == '[12.5d, 100.5d, 12.5d]', landed
    assert landed['Health'] == '19.0f', landed
    assert landed['advancements'] == {'any': True, 'short': False, 'five': False, 'landing': True}, landed
    differences = [label for label in seen['vanilla'] if seen['vanilla'][label] != seen['sunrise'][label]]
    for label in differences:
        print(f"difference after {label}: vanilla {seen['vanilla'][label]}, sunrise {seen['sunrise'][label]}",
              flush=True)
    print(f"{len(seen['vanilla']) - len(differences)}/{len(seen['vanilla'])} observations match", flush=True)
    if differences:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
