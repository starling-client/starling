#!/usr/bin/env python3
"""Compare zero-range entity pairing with the pinned vanilla dedicated server."""
import argparse
from pathlib import Path
import sys

try:
    from . import test_server_knockback_parity as net
except ImportError:
    import test_server_knockback_parity as net

NAME = 'server-entity-zero-range-parity'
X, Y, Z = net.A
CASES = {
    'zero-range': [
        # Same x/z as Alice, and a positive-range control. The server-side
        # query distinguishes a summon without pairing from a failed summon.
        f'cmd Alice summon minecraft:armor_stand {X} {Y} {Z} {{NoGravity:1b}}',
        f'cmd Alice summon minecraft:marker {X} {Y} {Z}',
        'present marker', 'ticks 4',
    ],
}


def capture(lines):
    spawns = []
    for line in lines:
        who, step, event = line.split(' ', 2)
        if event.startswith(('error', 'disconnected')):
            raise AssertionError(f'{who} failed while recording {step}')
        if step == 'step=zero-range' and event in ('spawn armor_stand', 'spawn marker'):
            spawns.append((who, event))
    return spawns


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sunrise-binary', type=Path, help='use an existing dedicated game build')
    parser.add_argument('--recording-dir', type=Path, default=net.lab.LOCAL / NAME)
    args = parser.parse_args()
    lock, jars, libraries = net.base.verify_artifacts()
    build = args.recording_dir.resolve()
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    vanilla_server, sunrise_server, client = net.harness_commands(lock, jars, libraries, build, args.sunrise_binary)
    world = net.generate_world(vanilla_server, build)
    vanilla = capture(net.run_side('vanilla', vanilla_server, client, build, world, CASES, tracking=True))
    if vanilla != [(who, 'spawn armor_stand') for who in net.PLAYERS]:
        raise AssertionError('vanilla did not record the positive-range control without the marker')
    sunrise = capture(net.run_side('sunrise', sunrise_server, client, build, world, CASES, tracking=True))
    result = 'pass' if sunrise == vanilla else 'fail'
    version = lock['minecraft']['version']
    net.lab.write_json(report, dict(schema=1, minecraft=version, recording=NAME, cases={'zero-range': result},
                                    server_jar_sha256=net.lab.digest(jars['server'], 'sha256'),
                                    vanilla_sha256=net.lab.digest(build / 'vanilla.txt', 'sha256'),
                                    sunrise_sha256=net.lab.digest(build / 'sunrise.txt', 'sha256')))
    print(f'{NAME} ({version}): zero-range: {result}')
    if result == 'fail':
        raise AssertionError('pairing packets differ (see the private recordings)')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
