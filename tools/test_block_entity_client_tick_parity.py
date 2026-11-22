#!/usr/bin/env python3
"""Compare the client ticks of spawners, trial spawners and vaults with vanilla 26.3.

The Java oracle (VanillaBlockEntityClientTickOracle) runs BaseSpawner.clientTick,
TrialSpawner.tickClient and VaultBlockEntity.Client.tick as ClientLevel.tickBlockEntities does:
every loaded one, each tick, in the level's ticker order, sharing the level random. Columns arrive
as chunk packets carrying update tags, whose order the tickers follow (LevelChunk.replaceWithPacketData),
and level events 3011 to 3021 run through LevelEventHandler. The game test TEST runs the same cases on a
server's client: chunk packets (`remote_chunk`), forgotten columns, level events and
`tick_client_block_entities`. Each tick compares one ordered stream of particles and local sounds
(with each sound's seed), each block entity's spin and the level random's state, bit for bit.

Cases: a spawner at its required range's edge (inside, on, outside), with a dead or spectating
player; every trial spawner state, ominous or not, around a spawn and a 20-tick boundary; vaults
with and without a display item and a connected player around the particle range; each kind at
Chebyshev distance 63, 64 and 65 and far beyond, several sharing the level random, a mixed column
forgotten and sent again in another order, and level events 3011 to 3021.

docs/parity/block-entity-client-ticks.jsonl holds the cases and a hash of vanilla's log of each, which
the game test checks alone in the daily run; `--write` makes it again, and a run fails while it is
stale. Diagnostics go to .local/block-entity-client-tick-parity/report.txt.

  test_block_entity_client_tick_parity.py [--filter SUBSTRING] [--write]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity'
TEST = 'game::spawner_client_tick_tests::client_ticks_match_the_vanilla_cases'
COMMITTED = lab.ROOT / 'docs/parity/block-entity-client-ticks.jsonl'
NAME = 'block_entity_client_tick'
TRIAL_STATES = ['inactive', 'waiting_for_players', 'active', 'waiting_for_reward_ejection', 'ejecting_reward', 'cooldown']
FACINGS = ['north', 'south', 'west', 'east']


def player(x, y, z, dead=False, spectator=False):
    return {'pos': [x, y, z], 'dead': dead, 'spectator': spectator}


def spawner(pos, range_=16, delay=20):
    return {'kind': 'spawner', 'pos': pos, 'range': range_, 'delay': delay}


def trial(pos, state, ominous=False, next_mob=0):
    return {'kind': 'trial_spawner', 'pos': pos, 'state': state, 'ominous': ominous, 'nextMobSpawnsAt': next_mob}


def vault(pos, facing='north', ominous=False, display=True, connected=True, state='active', particles_range=4.5):
    return {'kind': 'vault', 'pos': pos, 'facing': facing, 'ominous': ominous, 'display': display,
            'connected': connected, 'state': state, 'particlesRange': particles_range}


def cases():
    out = []

    def add(name, entities, who, ticks=24, game_time=1000, events=None):
        case = {'name': name, 'seed': 0x5EED0000 + len(out), 'gameTime': game_time, 'ticks': ticks,
                'player': who, 'entities': entities}
        if events:
            case['events'] = events
        out.append(case)

    # BaseSpawner.isNearPlayer: strictly closer than the range to the block's centre.
    for dx in (15.5, 16.0, 16.5, -15.25):
        add(f'spawner range {dx}', [spawner([8, 64, 8])], player(8.5 + dx, 64.5, 8.5))
    add('spawner diagonal', [spawner([8, 64, 8], range_=6)], player(8.5 + 4.2, 64.5, 8.5 - 4.2))
    add('spawner dead player', [spawner([8, 64, 8])], player(9.5, 64.0, 8.5, dead=True))
    add('spawner spectator', [spawner([8, 64, 8])], player(9.5, 64.0, 8.5, spectator=True))
    add('spawner counts down', [spawner([8, 64, 8], delay=3)], player(9.5, 64.0, 8.5), ticks=8)
    add('spawner any range', [spawner([8, 64, 8], range_=-1)], player(500.5, 64.0, 8.5))
    for state in TRIAL_STATES:
        for ominous in (False, True):
            add(f'trial {state} {ominous}', [trial([8, 64, 8], state, ominous, next_mob=1010)], player(9.5, 64.0, 8.5),
                ticks=24, game_time=995)
    for facing in FACINGS:
        add(f'vault {facing}', [vault([8, 64, 8], facing)], player(10.5, 64.0, 9.5), game_time=1000)
    add('vault ominous inactive', [vault([8, 64, 8], ominous=True, state='inactive', display=False)], player(9.5, 64.0, 8.5))
    # VaultBlockEntity.Client.isWithinConnectionRange: block positions, <= the range.
    for dx in (4, 5, -3):
        add(f'vault connection {dx}', [vault([8, 64, 8])], player(8.5 + dx, 64.0, 8.5), game_time=1000)
    add('vault not connected', [vault([8, 64, 8], connected=False)], player(9.5, 64.0, 8.5))
    # ClientLevel ticks every loaded block entity, however far from the player (the storage around
    # the view centre still holds their columns).
    far = [trial([8 + 70, 64, 8], 'cooldown'), vault([8 - 130, 64, 8 + 150]), trial([8, 64, 8 + 200], 'active')]
    add('far block entities', far, player(8.5, 64.0, 8.5), game_time=1000)
    # Each kind at Chebyshev distance 63, 64 and 65 from the player's block, along x, z and y, in
    # three columns: none of them stops at 64.
    for kind, make in (('spawner', lambda p: spawner(p, range_=-1)), ('trial', lambda p: trial(p, 'active')),
                       ('vault', vault)):
        edge = [make(p) for d in (63, 64, 65) for p in ([8 + d, 64, 8], [8, 64, 8 - d], [8, 64 + d, 8])]
        add(f'{kind} chebyshev 63 64 65', edge, player(8.5, 64.0, 8.5), game_time=1000)
    # Several draw from the one level random, in the level's ticker order.
    many = [trial([1, 64, 1], 'active'), vault([3, 64, 1]), spawner([5, 64, 1]), trial([1, 64, 5], 'waiting_for_players'),
            vault([20, 64, 3], 'east', ominous=True), trial([3, 64, 12], 'cooldown', True), spawner([12, 64, 12])]
    add('shared random', many, player(6.5, 64.0, 4.5), game_time=1000)
    # A mixed column the server forgets and sends again, then sends again while it is loaded, each
    # packet listing its block entities in another order: the tickers follow the packet, not the
    # column's map. Column (1, 0) goes and comes back within one tick.
    mixed = many + [spawner([9, 70, 14]), vault([14, 66, 2], 'south'), trial([18, 64, 2], 'active'),
                    vault([30, 64, 9], 'west'), spawner([17, 64, 14])]
    first = [i for i, e in enumerate(mixed) if e['pos'][0] < 16]
    second = [i for i, e in enumerate(mixed) if e['pos'][0] >= 16]
    add('chunk reload', mixed, player(6.5, 64.0, 4.5), ticks=30, game_time=990,
        events={'6': [['forget', 0, 0]], '9': [['send', 0, 0, first[::-1]]],
                '14': [['send', 0, 0, first[2:] + first[:2]]],
                '20': [['forget', 1, 0], ['send', 1, 0, second[::-1]]]})
    # LevelEventHandler.levelEvent 3011 to 3021 draws from the same level random as the ticks, a
    # sound's seed included. Within 10 blocks of the player: a farther sound's distance delay is not
    # compared here.
    at, vault_at = [10, 64, 8], [8, 64, 8]
    level_events = [[3011, at, 0], [3012, at, 1], [3013, at, 2], [3014, at, 0], [3015, vault_at, 0],
                    [3016, vault_at, 1], [3017, at, 0], [3018, at, 0], [3019, at, 3], [3020, at, 0],
                    [3020, at, 1], [3021, at, 0], [3015, vault_at, 1], [3012, at, 0]]
    add('level events', [vault(vault_at), trial(at, 'active')], player(9.5, 64.0, 10.5), ticks=len(level_events) + 2,
        game_time=1000, events={str(t): [['event', *e]] for t, e in enumerate(level_events)})
    return out


def canonical(log):
    """A log as the game test hashes its own: compact, keys sorted."""
    return json.dumps(log, separators=(',', ':'), sort_keys=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--write', action='store_true', help=f'write {COMMITTED.relative_to(lab.ROOT)} again')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    chosen = [c for c in cases() if args.filter in c['name']]
    if not chosen:
        sys.exit(f'{NAME}: --filter {args.filter!r} selects no case')
    if args.write and args.filter:
        sys.exit(f'{NAME}: --write takes every case, not a filter')
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'block-entity-client-tick-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.txt'
    report.write_text('')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))

    def run(command, name, cwd=lab.ROOT, env=None):
        path = build / (name + '.log')
        with path.open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT)
        if result.returncode:
            sys.exit(f'{NAME}: {name} failed (log: {path})')
        return path

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes,
         PARITY / 'ParityData.java', PARITY / 'VanillaBlockEntityClientTickOracle.java'], 'compile-java')
    run(['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
         'dev.sunriselab.parity.VanillaBlockEntityClientTickOracle', inputs, build / 'vanilla.jsonl'], 'vanilla')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(chosen), (len(vanilla), len(chosen))
    notes = []
    for case, v in zip(chosen, vanilla):
        if 'error' in v:
            notes.append(f'ORACLE ERROR {case["name"]}: {v["error"]}')
    if notes:
        report.write_text('\n'.join(notes) + '\n')
        sys.exit(f'{NAME}: the oracle failed on {len(notes)} cases (report: {report})')
    rows = [{'case': c, 'vanilla': hashlib.sha256(canonical(v).encode()).hexdigest()} for c, v in zip(chosen, vanilla)]
    if args.write:
        COMMITTED.write_text(''.join(json.dumps(r, separators=(',', ':')) + '\n' for r in rows))
    with tempfile.TemporaryDirectory(prefix='sunrise-client-tick-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        compiled = run(['cargo', 'test', '--locked', '-p', 'sunrise-client', '--lib', '--no-run', '--message-format=json'],
                       'compile', env=env)
        binaries = []
        for line in compiled.read_text(encoding='utf-8').splitlines():
            if not line.startswith('{'):
                continue
            row = json.loads(line)
            if (row.get('reason') == 'compiler-artifact' and row.get('executable')
                    and row.get('target', {}).get('name') == 'sunrise_client' and row.get('profile', {}).get('test')):
                binaries.append(Path(row['executable']))
        assert len(binaries) == 1, 'expected one Game test binary'
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_CLIENT_TICK_CASES'] = str(inputs)
        env['SUNRISE_CLIENT_TICK_OUT'] = str(build / 'rust.jsonl')
        log = run([binaries[0], TEST, '--exact', '--ignored', '--nocapture'], 'game', cwd=scratch, env=env)
        assert '1 passed; 0 failed' in log.read_text(encoding='utf-8'), 'the game test did not run'
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(rust) == len(chosen), (len(rust), len(chosen))
    failed = 0
    for case, v, r in zip(chosen, vanilla, rust):
        where = first_difference(v['ticks'], r['ticks'])
        if where:
            notes.append(f'MISMATCH {case["name"]}: {where}')
            failed += 1
    if not args.filter and not args.write:
        committed = [json.loads(l) for l in COMMITTED.read_text().splitlines()] if COMMITTED.exists() else []
        if committed != rows:
            notes.append(f'{COMMITTED.relative_to(lab.ROOT)} is not these cases and vanilla\'s logs: run with --write')
            failed += 1
    report.write_text(''.join(n + '\n' for n in notes) + f'{len(chosen) - failed} of {len(chosen)} cases match\n')
    if failed:
        sys.exit(f'{NAME}: FAILED, {failed} of {len(chosen)} cases differ (report: {report})')
    print(f'{NAME}: ok')


def first_difference(vanilla, rust):
    """Where two logs first part: the tick and field, and the entry, never vanilla's values."""
    for t, (vt, rt) in enumerate(zip(vanilla, rust)):
        for field in ('events', 'spins', 'random'):
            if vt[field] == rt[field]:
                continue
            if field == 'random':
                return f'tick {t} leaves the level random elsewhere'
            at = next((i for i, (a, b) in enumerate(zip(vt[field], rt[field])) if a != b),
                      min(len(vt[field]), len(rt[field])))
            return f'tick {t} {field} differ from entry {at} ({len(vt[field])} vanilla, {len(rt[field])} Sunrise)'
    if len(vanilla) != len(rust):
        return f'{len(vanilla)} vanilla ticks, {len(rust)} Sunrise ticks'
    return None


if __name__ == '__main__':
    main()
