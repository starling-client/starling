#!/usr/bin/env python3
"""Compare how far block entities are drawn with the vanilla 26.3 client renderers.

The Java oracle (VanillaBlockEntityReachOracle) lists every block whose block entity has a
renderer, then asks that renderer BlockEntityRenderDispatcher's questions for each case:
shouldRenderOffScreen, getViewDistance and shouldRender (camera position and render distance).
`examples/probe/block_entity_render_fixture reach` answers the same cases with Sunrise's `Reach`, which
every block entity draw asks (the BlockEntityRenderer path, beacon and test instance beams,
structure and test instance outlines, moving pistons, campfire, brushable, shelf, vault and
spawner items). Cameras sit on each view distance's edge, one float step either side of it,
and half a block inside, around block positions near the origin, below zero and near the
world border. Every value must match exactly. The output names the harness and its outcome;
which cases differ goes to mismatches.jsonl in the build folder, kept local.

  test_block_entity_reach_parity.py [--filter SUBSTRING]
"""
import argparse
import json
import math
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
POSITIONS = [[0, 64, 0], [-37, -20, 1205], [29_999_983, 100, -29_999_983], [5, 319, -8]]
# The fixed view distances (getViewDistance), seen at a short and a long render distance,
# and the beacon's, 16 blocks per chunk of render distance, at its own.
FIXED = [64, 68, 96, 256]
RENDER_DISTANCES = [2, 6, 12, 32]
EDGES = [(r, d) for r in FIXED for d in (2, 32)] + [(16 * d, d) for d in RENDER_DISTANCES]
# A server that sent a smaller view distance caps the render distance (Options.getEffectiveRenderDistance):
# (edge, render distance, server view distance), asked of the game's own draws only.
SERVER_EDGES = [(16 * 3, d, 3) for d in (6, 32)] + [(16 * 7, 12, 7), (16 * 6, 6, 40)]
# The blocks whose draws the game test makes, one per draw path (campfire, shelf and brushable
# items, beacon beams, moving pistons, and the frame's off-screen selection for the End gateway).
GAME_BLOCKS = ['minecraft:campfire', 'minecraft:soul_campfire', 'minecraft:oak_shelf', 'minecraft:suspicious_sand',
               'minecraft:beacon', 'minecraft:moving_piston', 'minecraft:end_gateway']
GAME_TEST = 'game::block_entity_reach_draw_tests::block_entity_draws_match_the_vanilla_cases'


def cameras(pos, edges=None):
    """(camera, render distance) around every edge: on it, one float step either side, half a block inside.

    The float steps are taken on the camera's coordinate, after the block centre is added, so the
    camera really sits one representable double inside and outside the edge.
    """
    centre = [c + 0.5 for c in pos]

    def around(x, outward):
        return (x, math.nextafter(x, -outward * math.inf), math.nextafter(x, outward * math.inf))

    out = []
    for r, render_distance, *server in edges or EDGES:
        cams = []
        # Along each axis, and straight above (BeaconRenderer measures across the ground only).
        cams += [[x, centre[1], centre[2]] for x in around(centre[0] + r, 1)]
        cams += [[centre[0], centre[1], z] for z in around(centre[2] - r, -1)]
        cams += [[centre[0], y, centre[2]] for y in around(centre[1] + r, 1)]
        # From the block's corner, not its centre.
        cams += [[x, pos[1], pos[2]] for x in around(pos[0] - r, -1)]
        for d in (r - 0.5, r + 0.5):
            s = d / math.sqrt(2)
            cams += [
                [centre[0] + d, centre[1], centre[2]],
                [centre[0], centre[1], centre[2] - d],
                [centre[0] + s, centre[1], centre[2] - s],
                [centre[0], centre[1] + d, centre[2]],
                [centre[0] - s, centre[1] + 37.25, centre[2] + s],
                [pos[0] - d, pos[1], pos[2]],
            ]
        out += [(camera, render_distance, *server) for camera in cams]
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-block-entity-reach-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name, cwd=lab.ROOT, env=None):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:])
            sys.exit(f'{name} failed')

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaTextDisplayRenderOracle',
                                          'VanillaBlockEntityReachOracle'))], 'compile-java')
    java = ['java', '-Xmx2G', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
            'dev.sunriselab.parity.VanillaBlockEntityReachOracle']
    run([*java, 'list', build / 'blocks.txt'], 'vanilla-list')
    blocks = [b for b in (build / 'blocks.txt').read_text().split() if args.filter in b]
    assert blocks, 'no block entity renderers listed'
    chosen = [{'block': block, 'pos': pos, 'camera': camera, 'renderDistance': d}
              for block in blocks for pos in POSITIONS for camera, d in cameras(pos)]
    drawn = [dict(c) for c in chosen if c['block'] in GAME_BLOCKS] + [
        {'block': block, 'pos': pos, 'camera': camera, 'renderDistance': d, 'serverDistance': server}
        for block in ('minecraft:beacon',) if block in blocks
        for pos in POSITIONS for camera, d, server in cameras(pos, SERVER_EDGES)]
    inputs, game_inputs = build / 'inputs.jsonl', build / 'game-inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    game_inputs.write_text(''.join(json.dumps(c) + '\n' for c in drawn))
    run([*java, 'cases', inputs, build / 'vanilla.jsonl'], 'vanilla')
    run([*java, 'cases', game_inputs, build / 'vanilla-game.jsonl'], 'vanilla-game')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'sunrise-client-lib', '--example', 'client_probe', '--',
         'block_entity_render_fixture', 'reach', inputs, build / 'rust.jsonl'], 'rust')
    with tempfile.TemporaryDirectory(prefix='sunrise-reach-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        run(['cargo', 'test', '--locked', '-p', 'sunrise-client', '--lib', '--no-run', '--message-format=json'],
            'compile-game', env=env)
        binaries = [Path(row['executable']) for row in map(json.loads, filter(
            lambda l: l.startswith('{'), (build / 'compile-game.log').read_text(encoding='utf-8').splitlines()))
            if row.get('reason') == 'compiler-artifact' and row.get('executable')
            and row.get('target', {}).get('name') == 'sunrise_client' and row.get('profile', {}).get('test')]
        assert len(binaries) == 1, 'expected one Game test binary'
        env.update(SUNRISE_TEST_RESOURCES=str(Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)).resolve()),
                   SUNRISE_REACH_DRAW_CASES=str(game_inputs), SUNRISE_REACH_DRAW_OUT=str(build / 'game.jsonl'))
        run([binaries[0], GAME_TEST, '--exact', '--ignored'], 'game', cwd=scratch, env=env)
        assert '1 passed; 0 failed' in (build / 'game.log').read_text(encoding='utf-8'), 'the game test did not run'
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen), (len(vanilla), len(rust), len(chosen))
    failed, errors = 0, 0
    with (build / 'mismatches.jsonl').open('w') as details:
        for case, v, r in zip(chosen, vanilla, rust):
            differ = ['error'] if 'error' in v else [
                k for k in ('offScreen', 'viewDistance', 'shouldRender') if v[k] != r[k]]
            if differ:
                errors += differ == ['error']
                failed += 1
                details.write(json.dumps({'case': case, 'differ': differ}) + '\n')
        vanilla_game = [json.loads(l) for l in (build / 'vanilla-game.jsonl').read_text().splitlines()]
        game = [json.loads(l) for l in (build / 'game.jsonl').read_text().splitlines()]
        assert len(vanilla_game) == len(game) == len(drawn), (len(vanilla_game), len(game), len(drawn))
        game_failed = 0
        for case, v, r in zip(drawn, vanilla_game, game):
            (field, answer), = r.items()
            if 'error' in v or v[field] != answer:
                game_failed += 1
                details.write(json.dumps({'case': case, 'differ': ['game ' + field]}) + '\n')
    print(f'{len(chosen) - failed} of {len(chosen)} cases match over {len(blocks)} blocks; {errors} oracle errors')
    print(f'{len(drawn) - game_failed} of {len(drawn)} game draw cases match')
    if failed or game_failed:
        sys.exit(f'block entity reach parity failed (details: {build / "mismatches.jsonl"})')
    print('block entity reach parity ok')


if __name__ == '__main__':
    main()
