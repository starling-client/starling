#!/usr/bin/env python3
"""Bounced arrows and continuing wind charges keep their place in the 26.3 tick order. Dedicated servers.

`ServerLevel` ticks its entities in the order they joined (`EntityTickList`). An arrow that hits an entity it can't
hurt bounces off (`AbstractArrow.onHitEntity`, `deflect(REVERSE)`) without ever leaving the level, and a wind charge
that flies on after earlier charges break a boat in the same tick also stays in its original slot. Both still tick
before every entity that joined after them.

A vanilla 26.3 dedicated server and Sunrise's each load a copy of one flat world vanilla generated first. Ticking is
frozen. Four isolated lanes run together:
- Arrow bounce (`arrow`, `c1`, `c2`): falling arrow bounces off `c1` on tick 1 and meets rising `c2` on tick 3.
- Wind-charge continuation (`w6_main`, `a2_main`): 5 wind charges break an oak boat on tick 1; `w6_main` flies on and
  clears `x = 8.0` before falling `a2_main` ticks on tick 4.
- Four-charge control (`w6_ctrl4`): 4 wind charges leave the boat intact, so `w6_ctrl4` bursts on tick 1.
- Reversed-order control (`w6_rev`, `a2_rev`): `a2_rev` joins before `w6_rev`, so on tick 4 `a2_rev` bounces off and
  redirects `w6_rev`.

The vanilla 26.3 wind-charge continuation witness and its `meta.json` are written into
`.local/witness/<version>/wind_charge_continuation_tick_order/` (gitignored). Normal runs compare the dedicated servers
and then run `game::creatures::projectile_batch_tests::continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass`
via `lab.parity_fixtures()` with `SUNRISE_WIND_CONTINUATION_WITNESS` pointing at that witness. Pass `--record` (or
`--record-wind-witness`) to record only the vanilla 26.3 witness without building Sunrise.
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_locator_bar_parity as locator, test_rust_parity as base
from tools.test_dedicated_server_parity import Server

NAME = 'bounced-projectile-tick-order-parity'
TICKS = 5
ARROW_TAGS = ['arrow', 'c2']
WIND_TAGS = ['w6_main', 'a2_main', 'w6_ctrl4', 'w6_rev', 'a2_rev']
BOAT_SPECS = [
    ('boat_main', (4.5, -60.0, 8.5)),
    ('boat_ctrl4', (4.5, -60.0, 24.5)),
    ('boat_rev', (4.5, -60.0, 40.5)),
]
TAGS = [*ARROW_TAGS, *WIND_TAGS]
ARROW_SETUP = [
    'summon minecraft:arrow 4.5 -50.0 4.5 {Motion:[0.0d,-0.2d,0.0d],pickup:0b,Tags:["arrow"]}',
    'summon minecraft:breeze_wind_charge 4.5 -50.2625 4.5 {Motion:[0.6d,0.0d,0.0d],acceleration_power:0.0d,Tags:["c1"]}',
    'summon minecraft:breeze_wind_charge 4.5 -51.33 4.5 {Motion:[0.0d,0.5d,0.0d],acceleration_power:0.0d,Tags:["c2"]}',
]
WIND_SETUP = [
    'summon minecraft:oak_boat 4.5 -60.0 8.5 {Tags:["boat_main"]}',
    *['summon minecraft:wind_charge 4.5 -59.2 8.5 {Motion:[0.0d,-0.5d,0.0d],acceleration_power:0.0d}'] * 5,
    'summon minecraft:breeze_wind_charge 3.5 -59.7 8.5 {Motion:[1.5d,0.0d,0.0d],acceleration_power:0.0d,Tags:["w6_main"]}',
    'summon minecraft:arrow 8.0 -58.6 8.5 {Motion:[0.0d,-0.2d,0.0d],pickup:0b,Tags:["a2_main"]}',
    'summon minecraft:oak_boat 4.5 -60.0 24.5 {Tags:["boat_ctrl4"]}',
    *['summon minecraft:wind_charge 4.5 -59.2 24.5 {Motion:[0.0d,-0.5d,0.0d],acceleration_power:0.0d}'] * 4,
    'summon minecraft:breeze_wind_charge 3.5 -59.7 24.5 {Motion:[1.5d,0.0d,0.0d],acceleration_power:0.0d,Tags:["w6_ctrl4"]}',
    'summon minecraft:oak_boat 4.5 -60.0 40.5 {Tags:["boat_rev"]}',
    *['summon minecraft:wind_charge 4.5 -59.2 40.5 {Motion:[0.0d,-0.5d,0.0d],acceleration_power:0.0d}'] * 5,
    'summon minecraft:arrow 8.0 -58.6 40.5 {Motion:[0.0d,-0.2d,0.0d],pickup:0b,Tags:["a2_rev"]}',
    'summon minecraft:breeze_wind_charge 3.5 -59.7 40.5 {Motion:[1.5d,0.0d,0.0d],acceleration_power:0.0d,Tags:["w6_rev"]}',
]
SETUP = ['time set 6000', *ARROW_SETUP, *WIND_SETUP]
WIND_WITNESS_NAME = 'wind_charge_continuation_tick_order'
WIND_WITNESS_FILE = f'{WIND_WITNESS_NAME}.txt'
WIND_WITNESS_TAGS = {*WIND_TAGS, *(boat_tag for boat_tag, _pos in BOAT_SPECS)}
WIND_TEST = (
    'game::creatures::projectile_batch_tests::'
    'continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass'
)
NUMBER = re.compile(r'(-?[0-9][0-9.E-]*)d')


def compile_client(lock, jars, libraries, build):
    """Compile LocatorBarClient without building Sunrise (used by --record)."""
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/LocatorBarClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/sunrise/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                   cwd=lab.ROOT, check=True, capture_output=True)
    return ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies, 'dev.sunriselab.parity.LocatorBarClient']


def run(label, command, client_command, world, build, *, setup=SETUP, tags=TAGS, boats=BOAT_SPECS, ticks=TICKS):
    directory = build / label
    port = locator.server_dir(directory, world)
    out = build / f'{label}-boats.txt'
    out.unlink(missing_ok=True)
    server = Server(command, directory, label)
    tick_proj_lines = {}
    tick_boat_console_alive = {}
    try:
        server.wait_for(r'Done \(', timeout=600)
        server.console('forceload add 0 0 31 63')
        with (build / f'{label}-client.log').open('w', encoding='utf-8') as log:
            client = locator.Client(client_command, port, out, log)
            try:
                client.tell('boat-damage on')
                client.tell('step setup')
                client.tell('join Alice')
                server.wait_for(r'Alice joined the game', timeout=120)
                client.tell('cmd Alice gamemode spectator')
                client.tell('cmd Alice tp @s 4.5 -40 24.5')
                client.sync(['Alice'])
                server.console('tick freeze')
                server.console('say READY')
                server.wait_for(r'READY', timeout=120)
                time.sleep(3)
                for command_line in setup:
                    server.console(command_line)
                time.sleep(1)
                client.sync(['Alice'])
                for tick in range(ticks + 1):
                    client.tell(f'step tick_{tick}')
                    if tick:
                        server.console('tick step 1')
                        time.sleep(1.0)
                    client.sync(['Alice'])
                    start = len(server.lines)
                    for tag in tags:
                        for path in ('Pos', 'Motion'):
                            server.console(f'data get entity @e[tag={tag},limit=1] {path}')
                    for boat_tag, _pos in boats:
                        server.console(f'data get entity @e[tag={boat_tag},limit=1] Pos')
                    server.console(f'say END {tick}')
                    server.wait_for(rf'END {tick}\b', timeout=120)
                    answers = [l.split('INFO]: ', 1)[-1] for l in server.lines[start:]
                               if 'entity data:' in l or 'No entity was found' in l or 'Found no' in l]
                    proj_lines = []
                    for i, tag in enumerate(tags):
                        pos, motion = answers[2 * i:2 * i + 2] if len(answers) >= 2 * i + 2 else ('', '')
                        if 'entity data:' not in pos:
                            proj_lines.append(f'tick {tick} {tag} gone')
                        else:
                            proj_lines.append(f'tick {tick} {tag} pos {" ".join(NUMBER.findall(pos))}'
                                              f' motion {" ".join(NUMBER.findall(motion))}')
                    tick_proj_lines[tick] = proj_lines
                    boat_offset = 2 * len(tags)
                    console_alive = {}
                    for i, (boat_tag, _pos) in enumerate(boats):
                        ans = answers[boat_offset + i] if len(answers) > boat_offset + i else ''
                        console_alive[boat_tag] = 'entity data:' in ans
                    tick_boat_console_alive[tick] = console_alive
                client.end()
            finally:
                client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no boat transcript (see {build}/{label}-client.log)')
    client_lines = out.read_text(encoding='utf-8').splitlines()
    errors = [l for l in client_lines if ' error ' in f' {l} ' or 'disconnected' in l]
    if errors:
        raise AssertionError(f'{label}: client errors:\n' + '\n'.join(errors[:10]))
    pos_to_tag = {pos: tag for tag, pos in boats}
    events_by_step = {}
    for line in client_lines:
        parts = line.split()
        if len(parts) < 3:
            continue
        step = parts[1].removeprefix('step=')
        events_by_step.setdefault(step, []).append(parts[2:])
    boat_alive = {tag: False for tag, _pos in boats}
    boat_damage = {tag: '0.0' for tag, _pos in boats}

    def apply_step(step_name):
        for event in events_by_step.get(step_name, ()):
            kind = event[0]
            if kind in ('boat-add', 'boat-damage', 'boat-remove'):
                pos = (float(event[1]), float(event[2]), float(event[3]))
                tag = pos_to_tag.get(pos)
                if tag is None:
                    continue
                if kind == 'boat-add':
                    boat_alive[tag] = True
                    boat_damage[tag] = '0.0'
                elif kind == 'boat-damage':
                    boat_damage[tag] = event[4]
                elif kind == 'boat-remove':
                    boat_alive[tag] = False

    apply_step('setup')
    lines = []
    for tick in range(ticks + 1):
        apply_step(f'tick_{tick}')
        lines.extend(tick_proj_lines[tick])
        for boat_tag, _pos in boats:
            console_is_alive = tick_boat_console_alive[tick][boat_tag]
            if boat_alive[boat_tag] != console_is_alive:
                raise AssertionError(
                    f'{label}: tick {tick} {boat_tag} client alive={boat_alive[boat_tag]} '
                    f'disagrees with server console alive={console_is_alive}'
                )
            if console_is_alive:
                lines.append(f'tick {tick} {boat_tag} alive damage {boat_damage[boat_tag]}')
            else:
                lines.append(f'tick {tick} {boat_tag} gone')
    return lines


def motion(lines, tick, tag):
    line = next(l for l in lines if l.startswith(f'tick {tick} {tag} '))
    return [float(x) for x in line.split('motion ')[1].split()] if ' motion ' in line else None


def motion_y(lines, tick, tag):
    vel = motion(lines, tick, tag)
    return vel[1] if vel is not None else None


def proves_wind(vanilla):
    """Why vanilla's wind-charge continuation lanes do not prove tick order, or None."""
    for boat_tag, _pos in BOAT_SPECS:
        if f'tick 0 {boat_tag} alive damage 0.0' not in vanilla:
            return f'{boat_tag} was not recorded alive with 0.0 damage at tick 0'
    if motion(vanilla, 1, 'w6_ctrl4') is not None:
        return 'the four-charge control did not burst on the intact boat'
    main_v = motion(vanilla, 4, 'w6_main')
    rev_v = motion(vanilla, 4, 'w6_rev')
    a2_main = motion_y(vanilla, 4, 'a2_main')
    a2_rev = motion_y(vanilla, 4, 'a2_rev')
    if main_v is None or rev_v is None or a2_main is None or a2_rev is None:
        return 'a wind-charge continuation entity disappeared unexpectedly'
    if not rev_v[0] > main_v[0]:
        return 'the reversed-order control did not redirect w6_rev on tick 4'
    if not a2_rev > a2_main:
        return 'the reversed-order control did not bounce a2_rev on tick 4'
    return None


def proves(vanilla):
    """Why vanilla's transcript does not show what this harness is for, or None."""
    ys = [motion_y(vanilla, t, 'arrow') for t in range(TICKS + 1)]
    if None in ys:
        return 'the arrow is gone'
    if not ys[2] < 0:
        return 'the arrow was not falling again after its first bounce'
    # A bounce turns the fall to a tenth of it, upwards, before inertia and gravity.
    if not ys[3] > ys[2]:
        return 'the arrow did not bounce off the second wind charge on the third tick'
    return proves_wind(vanilla)


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def wind_witness_lines(lines):
    """The tick 0..=4 wind-charge continuation and boat rows from a vanilla transcript."""
    selected = []
    for line in lines:
        parts = line.split()
        if (len(parts) >= 3 and parts[0] == 'tick' and parts[1].isdigit()
                and int(parts[1]) <= 4 and parts[2] in WIND_WITNESS_TAGS):
            selected.append(line)
    return selected


def write_wind_witness(lock, jars, vanilla, command):
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / WIND_WITNESS_NAME
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_file = witness_dir / WIND_WITNESS_FILE
    rows = wind_witness_lines(vanilla)
    header = [
        f'# Vanilla {version} dedicated-server witness for wind-charge continuation tick order.',
        '# Recorded by tools/test_bounced_projectile_tick_order_parity.py.',
    ]
    witness_file.write_text('\n'.join([*header, *rows]) + '\n', encoding='utf-8')
    oracle_src = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/LocatorBarClient.java'
    harness_src = Path(__file__).resolve()
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_bounced_projectile_tick_order_parity.py',
        'witness_env': 'SUNRISE_WIND_CONTINUATION_WITNESS',
        'command': command,
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            'mods/sunrise/src/test/java/dev/sunriselab/parity/LocatorBarClient.java': lab.digest(oracle_src, 'sha256'),
            'tools/test_bounced_projectile_tick_order_parity.py': lab.digest(harness_src, 'sha256'),
        },
        'outputs': {
            witness_file.name: {
                'sha256': lab.digest(witness_file, 'sha256'),
                'bytes': witness_file.stat().st_size,
            },
        },
    })
    return witness_dir, witness_file, len(rows)


def run_wind_batch_test(build, witness_file, resources):
    with tempfile.TemporaryDirectory(prefix='sunrise-wind-continuation-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(resources.resolve())
        env['SUNRISE_WIND_CONTINUATION_WITNESS'] = str(witness_file.resolve())
        binary = lab.parity_fixtures()
        log_path = build / 'wind-continuation-test.log'
        with log_path.open('w', encoding='utf-8') as log:
            result = subprocess.run(
                [str(binary), WIND_TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch,
                env=env,
                timeout=3600,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        if result.returncode != 0:
            raise AssertionError(f'{WIND_TEST} failed against {witness_file}; see {log_path}')
        output = log_path.read_text(encoding='utf-8')
        if '1 passed; 0 failed' not in output:
            raise AssertionError(f'expected 1 passed; 0 failed for {WIND_TEST}; see {log_path}')


def record_wind_witness():
    lock, jars, libraries = base.verify_artifacts()
    deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
    build = lab.LOCAL / f'{NAME}-record'
    build.mkdir(parents=True, exist_ok=True)
    client = compile_client(lock, jars, libraries, build)
    world = locator.generate_world(vanilla_server, build)
    vanilla = run('vanilla', vanilla_server, client, world, build,
                  setup=['time set 6000', *WIND_SETUP], tags=WIND_TAGS, boats=BOAT_SPECS, ticks=4)
    why = proves_wind(vanilla)
    if why:
        raise AssertionError(f'vanilla is no proof: {why}:\n' + '\n'.join(vanilla))
    _witness_dir, witness_file, count = write_wind_witness(
        lock,
        jars,
        vanilla,
        ['python3', 'tools/test_bounced_projectile_tick_order_parity.py', '--record'],
    )
    print(f'recorded {count} lines to {witness_file}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument(
        '--record', '--record-only', '--record-wind-witness',
        dest='record',
        action='store_true',
        help='run only the vanilla 26.3 server and write the local wind-charge continuation witness into .local',
    )
    parser.add_argument(
        '--resources',
        type=Path,
        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)),
    )
    args = parser.parse_args()
    if args.prepare:
        lab.prepare()
    if args.record:
        record_wind_witness()
        return
    lock, jars, _libraries = base.verify_artifacts()
    lock, build, vanilla_server, sunrise_server, client = locator.sides(NAME)
    world = locator.generate_world(vanilla_server, build)
    vanilla = run('vanilla', vanilla_server, client, world, build)
    (build / 'vanilla.txt').write_text('\n'.join(vanilla) + '\n', encoding='utf-8')
    why = proves(vanilla)
    if why:
        raise AssertionError(f'vanilla is no proof: {why}:\n' + '\n'.join(vanilla))
    _witness_dir, witness_file, _count = write_wind_witness(
        lock,
        jars,
        vanilla,
        ['python3', 'tools/test_bounced_projectile_tick_order_parity.py'],
    )
    sunrise = run('sunrise', sunrise_server, client, world, build)
    (build / 'sunrise.txt').write_text('\n'.join(sunrise) + '\n', encoding='utf-8')
    if vanilla != sunrise:
        differ = [f'- {v}\n+ {h}' for v, h in zip(vanilla, sunrise) if v != h]
        raise AssertionError(f'{len(differ)} lines differ from vanilla:\n' + '\n'.join(differ[:20]))
    run_wind_batch_test(build, witness_file, args.resources)
    print(f'bounced projectile tick order: {len(vanilla)} lines identical, batched witness test passed against {witness_file}')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
