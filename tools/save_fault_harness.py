"""Shared scratch-world setup for the daily save-fault checks."""
from contextlib import contextmanager
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

try:
    from . import anvil_dump, lab
    from .integration_env import game_environment, require_server_reports
    from .test_dedicated_crash_save import PLAYER, offline_uuid
    from .test_dedicated_server_parity import Server, free_port, write_server_dir
except ImportError:
    import anvil_dump
    import lab
    from integration_env import game_environment, require_server_reports
    from test_dedicated_crash_save import PLAYER, offline_uuid
    from test_dedicated_server_parity import Server, free_port, write_server_dir

CHUNK = (32, 32)
TARGET = '512 100 512'
NEIGHBOR = '528 100 512'
OLD_SPAWN = (0, 5, 0)
BASE_SPAWN = (1, 5, 0)
BASE_ITEM = 'diamond'
OLD_ITEM = 'iron_ingot'


def build():
    require_server_reports(lab.RESOURCES)
    subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client'],
                   cwd=lab.ROOT, check=True)
    return lab.cargo_target() / 'fast' / 'sunrise-client'


def run(check):
    if sys.platform != 'linux':
        print('Save-fault checks run on Linux only')
        return
    binary = build()
    with tempfile.TemporaryDirectory(prefix='sunrise-save-faults-') as scratch:
        root = Path(scratch)
        reference = root / 'reference'
        make_reference(binary, reference)
        check(binary, root, reference / 'world')


def prepare(directory, world=None, *, seed=None):
    port = free_port()
    write_server_dir(directory, port, world=world)
    with (directory / 'server.properties').open('a', encoding='iso-8859-1') as props:
        props.write('level-type=minecraft:flat\ngenerate-structures=false\n')
        if seed is not None:
            props.write(f'level-seed={seed}\n')
    return port


@contextmanager
def dedicated(binary, directory, label, *, point=None, ready=True):
    env = os.environ.copy()
    env.pop('SUNRISE_CRASH_FIXTURE', None)
    if point is not None:
        env['SUNRISE_CRASH_FIXTURE'] = point
    server = Server([binary, '--resources', lab.RESOURCES, '--game-dir', directory, '--dedicated', '--nogui'],
                    directory, label, env=env)
    try:
        if ready:
            server.wait_for(r'Done \(', timeout=300)
        yield server
    finally:
        # A broken test may leave a writer parked; don't ask it to save again.
        if point is not None and server.process.poll() is None:
            server.process.kill()
            server.process.wait(timeout=30)
        server.stop()


@contextmanager
def joined(binary, server, port):
    with tempfile.TemporaryDirectory(prefix='sunrise-save-player-') as work, tempfile.TemporaryFile() as log:
        env = game_environment(os.environ, work)
        client = subprocess.Popen([binary, '--headless', '--game-dir', work, '--resources', lab.RESOURCES,
                                   '--render-distance', '2', '--server', f'127.0.0.1:{port}', '--name', PLAYER],
                                  cwd=work, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        try:
            try:
                server.wait_for(rf'{PLAYER} joined the game', timeout=120)
            except AssertionError as why:
                log.seek(0)
                tail = log.read().decode(errors='replace').splitlines()[-20:]
                raise AssertionError(f'{why}\nClient log:\n' + '\n'.join(tail)) from why
            yield
        finally:
            client.kill()
            client.wait(timeout=30)


def save(server):
    server.console('save-all flush')
    server.wait_for(r'Saved the game', timeout=120)


def check_choices(server, condition, values, label):
    """Check an exact alternative with console commands, not a parsed save file."""
    start = len(server.lines)
    for i, value in enumerate(values):
        server.console(f'execute if {condition} minecraft:{value} run say {label}-{i}')
    server.console(f'say {label}-done')
    server.wait_for(rf'\[Server\] {re.escape(label)}-done$', timeout=60)
    replies = server.lines[start:]
    hits = [value for i, value in enumerate(values)
            if any(line.endswith(f'[Server] {label}-{i}') for line in replies)]
    assert len(hits) == 1, f'{label}: expected one of {values}\n' + '\n'.join(replies[-20:])


def wait_for_chunk(server, pos, label):
    # The console is FIFO, but chunk generation is not. Require a positive
    # answer after the forceload, not an arbitrary number of sleeps.
    deadline = time.monotonic() + 120
    for poll in range(1000):
        tag = f'{label}-{poll}'
        server.console(f'execute if loaded {pos} run say {tag}-loaded')
        server.console(f'say {tag}-done')
        server.wait_for(rf'\[Server\] {tag}-done$', timeout=max(1, deadline - time.monotonic()))
        if any(line.endswith(f'[Server] {tag}-loaded') for line in server.lines):
            return
        if time.monotonic() >= deadline:
            break
    raise AssertionError(f'{label}: chunk at {pos} did not load')


def player_file(world):
    return world / 'players' / 'data' / f'{offline_uuid(PLAYER)}.dat'


def items(path):
    return {stack['id'] for stack in anvil_dump.plain(anvil_dump.read_dat(path), full=True)['Inventory']}


def spawn(path):
    return tuple(anvil_dump.plain(anvil_dump.read_dat(path), full=True)['Data']['spawn']['pos'])


def generator_settings(world):
    path = world / 'data' / 'minecraft' / 'world_gen_settings.dat'
    return anvil_dump.plain(anvil_dump.read_dat(path), full=True, typed=True)


def region_file(world):
    x, z = CHUNK
    return world / 'dimensions' / 'minecraft' / 'overworld' / 'region' / f'r.{x // 32}.{z // 32}.mca'


def make_reference(binary, directory):
    """Create current and older, distinguishable metadata and player snapshots through real saves."""
    port = prepare(directory)
    world = directory / 'world'
    with dedicated(binary, directory, 'make-reference') as server, joined(binary, server, port):
        server.console('forceload add 512 512 528 512')
        wait_for_chunk(server, TARGET, 'baseline-target')
        wait_for_chunk(server, NEIGHBOR, 'baseline-neighbor')
        server.console(f'setblock {TARGET} minecraft:gold_block')
        server.console(f'setblock {NEIGHBOR} minecraft:emerald_block')
        server.console('setworldspawn ' + ' '.join(map(str, OLD_SPAWN)))
        server.console(f'item replace entity {PLAYER} hotbar.0 with minecraft:{OLD_ITEM}')
        save(server)
        assert spawn(world / 'level.dat') == OLD_SPAWN
        assert items(player_file(world)) == {f'minecraft:{OLD_ITEM}'}
        old_player = player_file(world).read_bytes()
        server.console('setworldspawn ' + ' '.join(map(str, BASE_SPAWN)))
        server.console(f'item replace entity {PLAYER} hotbar.0 with minecraft:{BASE_ITEM}')
        server.console('stop')
        assert server.process.wait(timeout=120) == 0, 'reference server did not stop cleanly'
    assert spawn(world / 'level.dat') == BASE_SPAWN
    assert spawn(world / 'level.dat_old') == OLD_SPAWN
    assert items(player_file(world)) == {f'minecraft:{BASE_ITEM}'}
    # A shutdown may save the same player again during logout. Use the
    # actual earlier snapshot as the backup so it cannot equal the primary.
    player_file(world).with_suffix('.dat_old').write_bytes(old_player)
    assert region_file(world).is_file(), 'reference server did not write the marked region'
