#!/usr/bin/env python3
"""World entity-type tags at open and after /reload, integrated and over TCP.

The integrated command fixture observes daylight ignition of the same mobs
across pack changes. A separate headless Sunrise client then connects first to
pinned vanilla and then to Sunrise, and reads the tag the server replaces in play.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

from test_dedicated_server_parity import Server, free_port, write_server_dir
import lab
import oracle_harness as oh
import test_command_exec_parity as commands
import test_rust_parity as base

TAG = 'minecraft:burn_in_daylight'
PROBE_TYPES = ('minecraft:zombie', 'minecraft:husk')
READY = 'entity tag fixture ready'


def write_pack(world, jar, name, *, replace, values):
    with zipfile.ZipFile(jar) as archive:
        version = json.loads(archive.read('version.json'))['pack_version']
    fmt = [version['data_major'], version['data_minor']]
    pack = world / 'datapacks' / name
    tag = pack / 'data/minecraft/tags/entity_type/burn_in_daylight.json'
    tag.parent.mkdir(parents=True, exist_ok=True)
    (pack / 'pack.mcmeta').write_text(json.dumps({'pack': {
        'description': 'Entity type tag fixture', 'pack_format': fmt[0],
        'min_format': fmt, 'max_format': fmt}}), encoding='utf-8')
    tag.write_text(json.dumps({'replace': replace, 'values': values}), encoding='utf-8')
    return tag


def integrated_ops():
    ops = []

    def command(text, **options):
        ops.append(dict(op='cmd', cmd='/' + text, record=False, **options))

    def advance(n):
        ops.append(dict(op='tick', n=n))

    def check(label, kind, expected_tag, expected_fire):
        selector = f'@e[tag=fixture_{kind},limit=1]'
        command(f'execute store result score fire entity_fixture run data get entity {selector} Fire')
        command('execute store success score burning entity_fixture run execute if score fire entity_fixture matches 1..')
        command(f'execute store success score member entity_fixture run execute if entity @e[tag=fixture_{kind},type=#{TAG}]')
        for subject, expected in [('member', expected_tag), ('burning', expected_fire)]:
            ops.append(dict(op='cmd', cmd=f'/scoreboard players get {subject} entity_fixture',
                            label=f'{label}: {kind} {subject}', expect_result=int(expected)))

    command('gamerule spawn_mobs false')
    command('gamerule advance_time false')
    command('gamerule advance_weather false')
    command('weather clear')
    command('time set noon')
    command('forceload add 0 0')
    ops.append(dict(op='settle', required_chunks=[[0, 0]]))
    command('fill 0 199 0 15 199 15 stone')
    command('tp @s 8.5 200 8.5')
    command('scoreboard objectives add entity_fixture dummy')
    command('datapack disable "file/add-husk"')
    advance(40)
    ops.append(dict(op='sync_randoms', uniquifier=3839, entity_id=3839000, level_seed=3839))
    for i, kind in enumerate(('zombie', 'husk'), start=4):
        command(f'summon {kind} {i}.5 200 5.5 {{NoAI:1b,Invulnerable:1b,Tags:["fixture_{kind}"]}}')
    advance(160)
    check('replace', 'zombie', False, False)
    check('replace', 'husk', False, False)
    command('datapack disable "file/no-daylight"')
    command('reload')
    advance(160)
    check('disable replacement', 'zombie', True, True)
    check('disable replacement', 'husk', False, False)
    command('datapack enable "file/add-husk"')
    advance(40)
    command('reload')
    advance(160)
    check('add husk', 'husk', True, True)
    return ops


def integrated(build, binary, reference_only):
    def install(world, _predicates, _functions, jar, *_args, **_kwargs):
        write_pack(world, jar, 'add-husk', replace=False, values=[PROBE_TYPES[1]])
        write_pack(world, jar, 'no-daylight', replace=True, values=[])

    commands.install_datapack_fixtures = install
    commands.corpus_module.groups = lambda: [('Entity type tags', integrated_ops())]
    args = ['--work-dir', str(build / 'integrated'), '--limit', '0']
    sys.argv[1:] = args + (['--reference-only'] if reference_only else ['--binary', str(binary)])
    with (build / 'integrated.log').open('w', encoding='utf-8') as log:
        with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
            try:
                commands.main()
            except SystemExit as error:
                print(error, file=log)
                raise RuntimeError('integrated command fixture failed') from None


def remote(build, label, command, binary, jar):
    directory = build / label
    port = free_port()
    write_server_dir(directory, port)
    with (directory / 'server.properties').open('a', encoding='utf-8') as properties:
        properties.write('level-type=minecraft:flat\n')
    tag = write_pack(directory / 'world', jar, 'daylight', replace=True, values=[])
    server = Server(command, directory, label)
    try:
        server.wait_for(r'Done \(')
        report = build / f'{label}-client.json'
        env = lab.game_env(SUNRISE_NET_REPORT=report, SUNRISE_NET_CHAT=READY,
                           SUNRISE_NET_ENTITY_TAG=TAG, SUNRISE_PLAYER_NAME='TagTester')
        with (build / f'{label}-client.log').open('w', encoding='utf-8') as log:
            with subprocess.Popen([binary, '--server', f'127.0.0.1:{port}', '--resources', lab.RESOURCES],
                                  cwd=directory, env=env, stdout=log, stderr=subprocess.STDOUT) as client:
                try:
                    server.wait_for(READY, timeout=75)
                    tag.write_text(json.dumps({'values': [PROBE_TYPES[1]]}), encoding='utf-8')
                    server.console('reload')
                    if client.wait(timeout=125) != 0:
                        raise RuntimeError('client exited unsuccessfully')
                finally:
                    if client.poll() is None:
                        client.kill()
        current = json.loads(report.read_text(encoding='utf-8'))
        snapshots = current['entity_tag_snapshots']
        # Independently guard both sides: an unbound client cannot match just
        # because it is wrong in the same way against both servers.
        if current['error'] or len(snapshots) != 2 or snapshots[0] != [] or snapshots[1] is None:
            raise RuntimeError('client did not receive one complete play replacement')
        result = [kind in snapshots[1] for kind in PROBE_TYPES]
        if result != [True, True]:
            raise RuntimeError('reloaded daylight membership is incorrect')
        return snapshots
    finally:
        server.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--reference-only', action='store_true', help='run only the integrated vanilla oracle')
    parser.add_argument('--only', choices=['integrated', 'remote'])
    args = parser.parse_args()
    if args.reference_only and args.only == 'remote':
        parser.error('--reference-only selects integrated')
    build = Path(tempfile.mkdtemp(prefix='entity-type-tags-', dir=lab.LOCAL))
    lock, jars, libraries = base.verify_artifacts()
    binary = args.binary
    if not args.reference_only:
        if binary is None:
            oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client'],
                          build, 'compile-rust')
            metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                          cwd=lab.ROOT, text=True))
            binary = Path(metadata['target_directory']) / 'fast' / ('sunrise-client.exe' if os.name == 'nt' else 'sunrise-client')
        binary = binary.resolve(strict=True)
    if args.only != 'remote':
        try:
            integrated(build, binary, args.reference_only)
        except Exception as error:
            (build / 'integrated-error.txt').write_text(str(error), encoding='utf-8')
            sys.exit('Entity type tags (integrated): FAIL')
        print('Entity type tags (integrated): pass', flush=True)
    if not args.reference_only and args.only != 'integrated':
        java = ['java', '-Xmx2g', '-cp', os.pathsep.join(map(str, [jars['server'], *libraries])),
                'net.minecraft.server.Main', 'nogui']
        native = [binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui']
        try:
            expected = remote(build, 'vanilla', java, binary, jars['server'])
            actual = remote(build, 'sunrise', native, binary, jars['server'])
            if [set(members) for members in actual] != [set(members) for members in expected]:
                raise RuntimeError('remote entity type tag differs from vanilla')
        except Exception as error:
            (build / 'remote-error.txt').write_text(str(error), encoding='utf-8')
            sys.exit('Entity type tags (remote): FAIL')
        print('Entity type tags (remote): pass', flush=True)


if __name__ == '__main__':
    main()
