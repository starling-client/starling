#!/usr/bin/env python3
"""Fixed scene on a `state-digest` dedicated server: BINARY RESOURCES TEMPLATE OUT [--detail] [--ticks N] [--sprint] [--fast-step] [--random-ticks] [--no-digest] [--generate] [--coverage]; random ticks are off unless --random-ticks; --coverage adds a village, foxes, dolphins, allays, looting mobs, tridents, a furnace, a brewing stand and the custom spawners, and defaults --ticks to 2500."""
import gzip
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import lab

GATE = os.environ.get('SUNRISE_SCENE_GATE')  # optional script that exits 0 when the machine is free to run
PROPERTIES = {
    'online-mode': 'false', 'level-seed': '12345', 'level-type': 'minecraft\\:flat', 'generate-structures': 'false',
    'gamemode': 'survival', 'difficulty': 'normal', 'view-distance': '4', 'simulation-distance': '4',
    'spawn-protection': '0', 'pause-when-empty-seconds': '-1', 'spawn-monsters': 'false', 'spawn-animals': 'false',
    'spawn-npcs': 'false', 'max-tick-time': '-1',
}
SPECIES = ['pig', 'cow', 'sheep', 'chicken', 'rabbit', 'zombie', 'skeleton', 'spider', 'villager', 'wolf']
Y = -60  # the flat world's surface
AREA = (0, 0, 95, 95)  # forceloaded block area: 6 x 6 chunks
SYNC = (424242, 100000, 987654321)  # uniquifier, entity id, level seed


def scene_commands(random_ticks, coverage=False):
    commands = [f'digest-sync {SYNC[0]} {SYNC[1]} {SYNC[2]}']
    if not random_ticks:
        commands.append('gamerule random_tick_speed 0')
    for s, species in enumerate(SPECIES):
        for i in range(20):
            x, z = 8 + s * 8 + (i % 4) * 1.5, 8 + (i // 4) * 1.5
            commands.append(f'summon minecraft:{species} {x:.1f} {Y} {z:.1f}')
    for i in range(20):
        x, z = 8 + i * 4, 60
        commands.append(f'setblock {x} {Y} {z} minecraft:chest')
        commands.append(f'setblock {x} {Y + 1} {z} minecraft:hopper[facing=down]'
                        '{Items:[{Slot:0b,id:"minecraft:cobblestone",count:64},{Slot:1b,id:"minecraft:dirt",count:32}]}')
    for i in range(5):
        commands.append(f'summon minecraft:tnt {10 + i * 15}.5 {Y} 85.5 {{fuse:{300 + i * 10}s}}')
    for i in range(10):
        commands.append(f'summon minecraft:item {8 + i * 8}.5 {Y + 3} 70.5 '
                        f'{{Item:{{id:"minecraft:{"diamond" if i % 2 else "oak_log"}",count:{i + 1}}}}}')
    if coverage:
        commands += coverage_commands()
    return commands


def item(x, y, z, item_id, count=1):
    return f'summon minecraft:item {x} {y} {z} {{Item:{{id:"minecraft:{item_id}",count:{count}}}}}'


def held(item_id):
    return f'equipment:{{mainhand:{{id:"minecraft:{item_id}",count:1}}}}'


VILLAGE = (64, 38)  # bell position (x, z); the players of --coverage stand beside it
VILLAGE_BEDS = (-8, -6, -4, 4, 6)  # bed x offsets from the bell: CatSpawner needs more than four
DAY_TIME = 17000  # the run crosses 18000, the siege roll, and the sky stays dark
SPIT_FOX = 'cov_spit'
SPIT_FOX_UUID = (24726, 0, 7, 1)
TRADER_DELAY = 1200  # the saved spawn delay of --coverage: the spawner's first pass, at tick 1200, rolls
TRADER_FRESH = 24000  # WanderingTraderSpawner's delay after a roll


def seed_coverage_world(world):
    """The saved wandering-trader data of --coverage, so its spawner rolls inside the window."""
    version = dat_ints(world / 'data' / 'minecraft' / 'weather.dat').get('DataVersion', 0)
    body = b'\x0a' + name(b'data') + int_tag(b'spawn_delay', TRADER_DELAY) + int_tag(b'spawn_chance', 25) + b'\x00'
    raw = b'\x0a' + name(b'') + body + int_tag(b'DataVersion', version) + b'\x00'
    (world / 'data' / 'minecraft' / 'wandering_trader.dat').write_bytes(gzip.compress(raw))


def name(text):
    return struct.pack('>H', len(text)) + text


def int_tag(key, value):
    return b'\x03' + name(key) + struct.pack('>i', value)


def dat_ints(path):
    """The int tags of a gzipped NBT file of ints and compounds (what the trader and weather data hold), flattened."""
    data, out, at = gzip.decompress(path.read_bytes()), {}, 3
    while at < len(data):
        kind = data[at]
        if kind == 0:
            at += 1
            continue
        size = struct.unpack('>H', data[at + 1:at + 3])[0]
        key, at = data[at + 3:at + 3 + size].decode(), at + 3 + size
        if kind == 3:
            out[key] = struct.unpack('>i', data[at:at + 4])[0]
            at += 4
        elif kind == 1:
            at += 1
        elif kind != 10:
            raise SystemExit(f'{path}: tag {kind} not read here')
    return out


def coverage_checks(run, world, owner=None):
    """After a --coverage run, through `run` (console commands to output; `owner` names the Loyalty trident's player): coverage.txt's lines, the spots missed."""
    def passed(command):
        return any('Test passed' in line for line in run([command]))
    lines, missed = [], []

    def need(label, ok):
        lines.append(f'{label} {"reached" if ok else "MISSED"}')
        if not ok:
            missed.append(label)
    fox = run([f'data get entity @e[type=fox,tag={SPIT_FOX},limit=1] equipment.mainhand.id'])
    spitter = ','.join(map(str, SPIT_FOX_UUID))
    need('fox_spit', passed(f'execute if entity @e[type=minecraft:item,nbt={{Thrower:[I;{spitter}]}}]')
         or any('has the following entity data' in line and 'minecraft:stick' not in line for line in fox))
    run(['save-all flush'])
    trader = dat_ints(world / 'data' / 'minecraft' / 'wandering_trader.dat')
    lines.append(f'trader spawn_delay={trader.get("spawn_delay")} spawn_chance={trader.get("spawn_chance")}')
    need('trader_roll', trader.get('spawn_delay') == TRADER_FRESH - TRADER_DELAY)
    if owner:
        need('village_cat', passed('execute if entity @e[type=minecraft:cat]'))
        need('loyalty_return', not passed('execute if entity @e[type=minecraft:trident,tag=cov_loyal]')
             and any('minecraft:trident' in line for line in run([f'data get entity {owner} Inventory'])))
    return lines, missed


def pickup_stat_check(world, name, uuid):
    """After the server stopped (a player's stats are written when it leaves): the emerald pickup stat of --coverage."""
    stats = world / 'players' / 'stats' / f'{uuid}.json'
    picked = json.loads(stats.read_text()).get('stats', {}).get('minecraft:picked_up', {}) if stats.exists() else {}
    ok = picked.get('minecraft:emerald') == 3
    return [f'{name} picked_up {json.dumps(picked, sort_keys=True)}', f'emerald_pickup {"reached" if ok else "MISSED"}'], [] if ok else ['emerald_pickup']


def uuid_text(ints):
    """A UUID's four ints (`data get entity <player> UUID`) as the dashed text stats files are named by."""
    h = ''.join(f'{v & 0xffffffff:08x}' for v in ints)
    return f'{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}'


def coverage_commands():
    """What wave 2's moves run and the default scene does not; only with --coverage, tick-counted like the rest; coverage_checks says each spot ran."""
    y = Y
    commands = []
    for k in range(4):  # foxes with items they accept; a summoned fox only loots with CanPickUpLoot
        x = 10 + k * 4
        commands.append(f'summon minecraft:fox {x}.5 {y} 22.5 {{CanPickUpLoot:1b}}')
        commands.append(item(f'{x + 1}.5', y, '22.5', ['sweet_berries', 'rabbit', 'sweet_berries', 'rabbit'][k], 3 if k == 0 else 1))
    # Food replaces the stick, which the fox spits out with itself as thrower: checked. Invulnerable, so wolves leave it.
    spitter = ','.join(map(str, SPIT_FOX_UUID))
    commands.append(f'summon minecraft:fox 26.5 {y} 22.5 {{CanPickUpLoot:1b,Invulnerable:1b,UUID:[I;{spitter}],'
                    f'Tags:["{SPIT_FOX}"],{held("stick")}}}')
    commands.append(item('27.5', y, '22.5', 'mushroom_stew'))
    commands.append(f'fill 30 {y - 3} 20 35 {y - 1} 25 minecraft:water')  # a 6x6x3 pool flush with the ground
    for k in range(3):
        commands.append(f'summon minecraft:dolphin {31 + k * 2}.5 {y - 2} 21.5')
        commands.append(item(f'{31 + k * 2}.5', y - 1.5, '22.5', ['cod', 'salmon', 'tropical_fish'][k]))
    for k in range(3):  # allays with a held item and the same item on the ground within 8 blocks
        x = 40 + k * 4
        thing = ['diamond', 'emerald', 'gold_ingot'][k]
        commands.append(f'summon minecraft:allay {x}.5 {y + 1} 22.5 {{{held(thing)}}}')
        commands.append(item(f'{x + 2}.5', y, '24.5', thing, 2))
    commands.append(f'setblock 44 {y} 27 minecraft:note_block')
    commands.append(f'setblock 45 {y} 27 minecraft:redstone_block')
    for k, (kind, thing) in enumerate([('zombie', 'iron_helmet'), ('zombie', 'diamond_sword'), ('skeleton', 'iron_helmet'),
                                       ('skeleton', 'diamond_sword'), ('zombie', 'iron_helmet')]):
        x = 8 + k * 5
        worse = ',equipment:{head:{id:"minecraft:leather_helmet",count:1}}' if k == 4 else ''
        commands.append(f'summon minecraft:{kind} {x}.5 {y} 32.5 {{CanPickUpLoot:1b,PersistenceRequired:1b{worse}}}')
        commands.append(item(f'{x + 1}.5', y, '32.5', thing))
    commands.append(f'fill 24 {y} 50 36 {y + 2} 50 minecraft:stone')  # a wall for the tridents
    for k in range(3):
        commands.append(f'summon minecraft:cow {12 + k * 2}.5 {y} 52.5')
        commands.append(f'summon minecraft:trident {12 + k * 2}.5 {y + 0.8} 46.5 {{Motion:[0.0d,0.0d,1.5d]}}')
        commands.append(f'summon minecraft:trident {26 + k * 3}.5 {y + 1.2} 44.5 {{Motion:[0.0d,0.05d,1.8d]}}')
    commands.append(f'setblock 46 {y} 44 minecraft:furnace{{Items:[{{Slot:0b,id:"minecraft:raw_iron",count:8}},'
                    '{Slot:1b,id:"minecraft:coal",count:8}]}')
    bottle = 'id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:water"}}'
    commands.append(f'setblock 50 {y} 44 minecraft:brewing_stand{{Fuel:20b,Items:[{{Slot:0b,{bottle}}},{{Slot:1b,{bottle}}},'
                    f'{{Slot:2b,{bottle}}},{{Slot:3b,id:"minecraft:nether_wart",count:4}},{{Slot:4b,id:"minecraft:blaze_powder",count:4}}]}}')
    vx, vz = VILLAGE  # a bell, five claimed beds and two workstations: a village where a cat can spawn
    commands.append(f'setblock {vx} {y} {vz} minecraft:bell[attachment=floor,facing=north]')
    for x in VILLAGE_BEDS:
        commands.append(f'setblock {vx + x} {y} {vz - 2} minecraft:red_bed[facing=north,part=foot]')
        commands.append(f'setblock {vx + x} {y} {vz - 3} minecraft:red_bed[facing=north,part=head]')
    commands.append(f'setblock {vx} {y} {vz + 4} minecraft:lectern[facing=south]')
    commands.append(f'setblock {vx + 2} {y} {vz + 4} minecraft:composter')
    for k, x in enumerate(VILLAGE_BEDS):  # CatSpawner counts beds that living villagers hold as home
        home = f'{{value:{{pos:[I;{vx + x},{y},{vz - 3}],dimension:"minecraft:overworld"}}}}'
        commands.append(f'summon minecraft:villager {vx - 4 + 2 * k}.5 {y} {vz + 2}.5 '
                        f'{{PersistenceRequired:1b,Brain:{{memories:{{"minecraft:home":{home}}}}}}}')
    commands += ['gamerule mob_griefing true', 'gamerule spawn_mobs true', 'gamerule spawn_monsters true',
                 'gamerule spawn_wandering_traders true', 'gamerule spawn_patrols true', f'time set {DAY_TIME}']
    return commands


def default_ticks():
    if '--ticks' in sys.argv:
        return int(sys.argv[sys.argv.index('--ticks') + 1])
    return 2500 if '--coverage' in sys.argv else 200


class Server:
    def __init__(self, binary, resources, directory, env, freeze=False):
        self.lines = []
        self.log = open(directory / 'server.log', 'w')
        self.process = subprocess.Popen(
            [binary, '--resources', resources, '--dedicated', '--nogui'], cwd=directory, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        threading.Thread(target=self._read, daemon=True).start()
        self.count = 0
        if freeze:
            # Queued before the first tick, so no unfrozen tick moves the game time by wall clock.
            self.process.stdin.write('tick freeze\n')
            self.process.stdin.flush()

    def _read(self):
        for line in self.process.stdout:
            self.lines.append(line.rstrip('\n'))
            self.log.write(line)
            self.log.flush()

    def wait_for(self, text, start=0, timeout=300):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(text in line for line in self.lines[start:]):
                return
            if self.process.poll() is not None:
                raise SystemExit('server exited:\n' + '\n'.join(self.lines[-30:]))
            time.sleep(0.01)
        raise SystemExit(f'no "{text}" in {timeout} s')

    def run(self, commands, timeout=300):
        self.count += 1
        mark = f'DIGESTMARK{self.count}X'
        start = len(self.lines)
        self.process.stdin.write(''.join(c + '\n' for c in commands) + f'say {mark}\n')
        self.process.stdin.flush()
        self.wait_for(mark, start, timeout)
        return self.lines[start:]

    def game_time(self):
        for line in reversed(self.run(['time query gametime'])):
            if 'time is ' in line:
                return int(line.split('time is ')[1].split()[0])
        raise SystemExit('no game time')

    def stop(self):
        if self.process.poll() is None:
            self.process.stdin.write('stop\n')
            self.process.stdin.flush()
            try:
                self.process.wait(120)
            except subprocess.TimeoutExpired:
                self.process.kill()


def hold_gate():
    if not GATE:
        return
    env = dict(os.environ, GATE_OWNER='state-digest')
    while subprocess.run([sys.executable, GATE], env=env, capture_output=True).returncode != 0:
        time.sleep(10)


def scratch_env(home, digest, detail):
    folders = {}
    for key in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'):
        path = home / key.lower()
        path.mkdir(parents=True, exist_ok=True)
        folders[key] = path
    env = lab.game_env(**folders, SUNRISE_TEST_SEEDS='1', DISPLAY=':99')
    if digest:
        env['SUNRISE_STATE_DIGEST'] = str(digest)
    if detail:
        env['SUNRISE_STATE_DIGEST_DETAIL'] = '1'
    return env


def prepare_dir(directory):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in PROPERTIES.items()))
    (directory / 'eula.txt').write_text('eula=true\n')


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    # Resolved here: the server runs in OUT/server, where relative paths would point elsewhere.
    binary, resources, template, out = (Path(a).resolve() for a in args[:4])
    ticks = default_ticks()
    out.mkdir(parents=True, exist_ok=True)
    home = Path(tempfile.mkdtemp(prefix='state-digest-home-'))
    hold_gate()
    if '--generate' in sys.argv:
        prepare_dir(template)
        server = Server(binary, resources, template, scratch_env(home, None, False))
        try:
            server.wait_for('Done (')
            server.run([f'forceload add {AREA[0]} {AREA[1]} {AREA[2]} {AREA[3]}'])
            time.sleep(5)
        finally:
            server.stop()
        return
    run_dir = out / 'server'
    if run_dir.exists():
        shutil.rmtree(run_dir)
    prepare_dir(run_dir)
    shutil.copytree(template / 'world', run_dir / 'world')
    (run_dir / 'world' / 'session.lock').unlink(missing_ok=True)
    coverage = '--coverage' in sys.argv
    if coverage:
        seed_coverage_world(run_dir / 'world')
    digest = out / 'digest.txt'
    server = Server(binary, resources, run_dir, scratch_env(home, None if '--no-digest' in sys.argv else digest, '--detail' in sys.argv), freeze=True)
    timings, missed = {}, []
    try:
        server.wait_for('Done (')
        server.run(['forceload query'])
        time.sleep(3)
        server.run(scene_commands('--random-ticks' in sys.argv, coverage))
        before = server.game_time()
        start = time.time()
        if '--sprint' in sys.argv:
            # The console prints no line when a sprint ends, so the game time says when it has.
            server.run(['tick unfreeze', f'tick sprint {ticks}'])
            while server.game_time() < before + ticks:
                time.sleep(0.05)
        else:
            server.run((['tick rate 10000'] if '--fast-step' in sys.argv else []) + [f'tick step {ticks}'])
            while server.game_time() < before + ticks:
                time.sleep(0.05)
        timings['step_seconds'] = time.time() - start
        if coverage:
            lines, missed = coverage_checks(server.run, run_dir / 'world')
            (out / 'coverage.txt').write_text('\n'.join(lines) + '\n')
    finally:
        server.stop()
        shutil.rmtree(home, ignore_errors=True)
    (out / 'timings.txt').write_text(f'{timings}\n')
    print(out, timings)
    if missed:
        raise SystemExit(f'--coverage missed: {", ".join(missed)} (see {out / "coverage.txt"})')


if __name__ == '__main__':
    main()
