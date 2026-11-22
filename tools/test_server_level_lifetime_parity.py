#!/usr/bin/env python3
"""The built-in levels' lifetime on a dedicated server with nobody in them, against vanilla's.

Vanilla keeps the Overworld, the Nether and the End in `MinecraftServer.levels` from `createLevels` to
`stopServer`; an idle level only unloads the chunks no ticket holds (`ChunkMap.processUnloads`). Sunrise saves and
closes an idle custom level after 300 ticks, but never a built-in one.

The pinned server JAR first generates a world (its chunk (0, 0) in the Nether and in the End saved); a vanilla
dedicated server and Sunrise's (`sunrise-client --dedicated --nogui`) then serve copies of it, with nobody joining,
and the console:

- idles past Sunrise's 300-tick idle close, then checks whether each built-in level answers (`execute in`) and
  whether chunk (0, 0) in the Nether and in the End is ready for entity ticking (`execute if loaded`);
- force-loads both chunks, waits until both are ready, and asks again;
- releases the Nether's ticket, idles again, and asks again: only the End chunk is still ready;
- stops the server, keeping the raw log for diagnosis.

The comparison uses only those command outcomes: dimension addressability and chunk readiness. An answer
shows a dimension can be addressed, not that the same level instance stayed open.
"""
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import CONSOLE_LINE, Server, free_port, wait_ticks, write_server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import CONSOLE_LINE, Server, free_port, wait_ticks, write_server_dir

NAME = 'server-level-lifetime-parity'
LEVELS = (('overworld', 'minecraft:overworld'), ('nether', 'minecraft:the_nether'), ('end', 'minecraft:the_end'))
# The levels whose chunk (0, 0) is forced and asked about (the Overworld's spawn is not this harness's business).
FORCED = ('minecraft:the_nether', 'minecraft:the_end')
# Past Sunrise's idle close of a custom level (`IDLE_TICKS`, 300).
IDLE_TICKS = 400


def say_payloads(server, start, barrier, timeout):
    """Read the console's `say` payloads only up to the command queue barrier."""
    # SayCommand broadcasts unsigned console chat; MinecraftServer.logChatMessage
    # writes the trust tag before the decorated sender. Sunrise logs it the same way.
    prefix = '[Not Secure] [Server] '
    server.console(f'say {barrier}')
    server.wait_for(rf': {re.escape(prefix + barrier)}$', timeout=timeout, after=start)
    payloads = []
    for line in server.lines[start:]:
        if (match := CONSOLE_LINE.match(line)) and match[2].startswith(prefix):
            payload = match[2].removeprefix(prefix)
            if payload == barrier:
                return payloads
            payloads.append(payload)
    raise AssertionError(f'{server.log_path.name}: missing {barrier} barrier')


def ask(server, step):
    """Require one addressability answer per built-in and one readiness answer per queried chunk."""
    start = len(server.lines)
    for short, level in LEVELS:
        server.console(f'execute in {level} run say {step} {short} answers')
        if level in FORCED:
            server.console(f'execute in {level} if loaded 0 64 0 run say {step} {short} 0 0 loaded')
            server.console(f'execute in {level} unless loaded 0 64 0 run say {step} {short} 0 0 unloaded')
    payloads = say_payloads(server, start, f'{step} asked', timeout=60)
    addressable, chunks = [], {}
    for short, level in LEVELS:
        if payloads.count(f'{step} {short} answers') != 1:
            raise AssertionError(f'{server.log_path.name}: {step}: expected one addressability marker for {short}')
        addressable.append(short)
        if level in FORCED:
            loaded, unloaded = (payloads.count(f'{step} {short} 0 0 {state}') for state in ('loaded', 'unloaded'))
            if loaded + unloaded != 1:
                raise AssertionError(f'{server.log_path.name}: {step}: expected one chunk readiness marker for {short}')
            chunks[short] = 'loaded' if loaded else 'unloaded'
    return dict(addressable=addressable, chunks=chunks)


def wait_loaded(server, levels, timeout=300):
    """Wait for a positive readiness marker for every queried chunk in one poll."""
    deadline = time.time() + timeout
    for poll in range(1_000_000):
        start = len(server.lines)
        markers = []
        for level in levels:
            marker = f'poll {poll} {level} ready'
            server.console(f'execute in {level} if loaded 0 64 0 run say {marker}')
            markers.append(marker)
        payloads = say_payloads(server, start, f'poll {poll} done', timeout=max(1, deadline - time.time()))
        if all(marker in payloads for marker in markers):
            return
        if time.time() > deadline:
            break
        time.sleep(1)
    raise AssertionError(f'{server.log_path.name}: chunk (0, 0) of {levels} not loaded in {timeout} s')


def generate_world(server_command, build):
    """The world both servers load, made by vanilla, with chunk (0, 0) of the Nether and of the End saved."""
    directory = build / 'generate'
    write_server_dir(directory, free_port())
    server = Server(server_command, directory, 'generate')
    try:
        server.wait_for(r'Done \(')
        for level in FORCED:
            server.console(f'execute in {level} run forceload add 0 0')
        wait_loaded(server, FORCED)
        for level in FORCED:
            server.console(f'execute in {level} run forceload remove all')
        server.console('save-all flush')
        server.wait_for(r'Saved the game', timeout=120)
    finally:
        server.stop()
    return directory / 'world'


def run_side(label, command, build, world):
    directory = build / label
    write_server_dir(directory, free_port(), world)
    server = Server(command, directory, label)
    try:
        server.wait_for(r'Done \(')
        wait_ticks(server, IDLE_TICKS)
        idle = ask(server, 'idle')
        for level in FORCED:
            server.console(f'execute in {level} run forceload add 0 0')
        wait_loaded(server, FORCED)
        forced = ask(server, 'forced')
        server.console('execute in minecraft:the_nether run forceload remove 0 0')
        wait_ticks(server, IDLE_TICKS)
        return dict(idle=idle, forced=forced, released=ask(server, 'released'))
    finally:
        server.stop()


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'sunrise-game', '--bin', 'sunrise-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('sunrise-client.exe' if os.name == 'nt' else 'sunrise-client')
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    sunrise_server = [binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui']

    world = generate_world(vanilla_server, build)
    results = {label: run_side(label, command, build, world)
               for label, command in (('vanilla', vanilla_server), ('sunrise', sunrise_server))}
    for label, phases in results.items():
        lab.write_json(build / f'{label}.json', phases)
    vanilla, sunrise = results['vanilla'], results['sunrise']
    expected = dict(idle=dict(nether='unloaded', end='unloaded'),
                    forced=dict(nether='loaded', end='loaded'),
                    released=dict(nether='unloaded', end='loaded'))
    for step, chunks in expected.items():
        if vanilla[step] != dict(addressable=['overworld', 'nether', 'end'], chunks=chunks):
            raise AssertionError(f'vanilla: unexpected {step} outcomes (see {build}/vanilla.json and vanilla.log)')
    if vanilla != sunrise:
        raise AssertionError(f'lifecycle outcomes differ (see vanilla.json, sunrise.json and raw logs in {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        phases=len(vanilla), output_sha256=lab.digest(build / 'sunrise.json', 'sha256'),
        scope='Built-in dimension addressability and entity-ticking readiness of the Nether and End chunks '
              'after idling, forcing both chunks, and releasing the Nether ticket.'))
    print('server level lifetime: pass')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
