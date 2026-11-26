#!/usr/bin/env python3
"""Compare terrain section vertical render-distance clamping against real Minecraft 26.3.

Compiles and runs VanillaSectionVisibilityOracle against the pinned 26.3 client jar on JDK 25,
writing the witness and meta.json into .local/witness/<version>/visibility/, then runs the
Rust client_probe visibility_fixture against it."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

ORACLE = 'VanillaSectionVisibilityOracle'
GENPVP_NON_EMPTY_SY = [0, 1, 2, 3, 8, 9, 10, 11]
ALL_VISIBLE = (1 << 36) - 1


def build_cases():
    cases = [
        # 1-5: Genpvp menu geometry (min_y=-64, 24 sections sy=-4..19, non-empty at sy={0..3,8..11}, empty at 12..19).
        {
            'name': 'genpvp_menu_level',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [0.5, 301.620015, 0.5], 'smart_cull': True,
            'loaded_chunk_range': [-3, 3, -3, 3],
            'non_empty_world_sy': GENPVP_NON_EMPTY_SY,
            'frustum': {'yaw': 0.0, 'pitch': 0.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'genpvp_menu_looking_down',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [0.5, 301.620015, 0.5], 'smart_cull': True,
            'loaded_chunk_range': [-3, 3, -3, 3],
            'non_empty_world_sy': GENPVP_NON_EMPTY_SY,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'genpvp_menu_looking_down_no_smart_cull',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [0.5, 301.620015, 0.5], 'smart_cull': False,
            'loaded_chunk_range': [-3, 3, -3, 3],
            'non_empty_world_sy': GENPVP_NON_EMPTY_SY,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'genpvp_menu_rd6_misses_lobby',
            'min_y': -64, 'sections': 24, 'render_distance': 6,
            'camera': [0.5, 301.620015, 0.5], 'smart_cull': True,
            'non_empty_world_sy': GENPVP_NON_EMPTY_SY,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'genpvp_menu_rd7_reaches_lobby',
            'min_y': -64, 'sections': 24, 'render_distance': 7,
            'camera': [0.5, 301.620015, 0.5], 'smart_cull': True,
            'non_empty_world_sy': GENPVP_NON_EMPTY_SY,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        # 6-11: Exact vertical clamp across heights, render distances, smart_cull on/off, and Nether height.
        {
            'name': 'vertical_clamp_overworld_rd2_mid',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': True,
        },
        {
            'name': 'vertical_clamp_overworld_rd2_mid_no_smart_cull',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': False,
        },
        {
            'name': 'vertical_clamp_overworld_rd3_mid',
            'min_y': -64, 'sections': 24, 'render_distance': 3,
            'camera': [8.5, 72.5, 8.5], 'smart_cull': True,
        },
        {
            'name': 'vertical_clamp_overworld_rd4_high',
            'min_y': -64, 'sections': 24, 'render_distance': 4,
            'camera': [8.5, 296.5, 8.5], 'smart_cull': True,
        },
        {
            'name': 'vertical_clamp_overworld_rd4_low',
            'min_y': -64, 'sections': 24, 'render_distance': 4,
            'camera': [8.5, -56.5, 8.5], 'smart_cull': True,
        },
        {
            'name': 'vertical_clamp_overworld_rd8_mid',
            'min_y': -64, 'sections': 24, 'render_distance': 8,
            'camera': [8.5, 72.5, 8.5], 'smart_cull': True,
        },
        {
            'name': 'vertical_clamp_nether_rd3_top',
            'min_y': 0, 'sections': 16, 'render_distance': 3,
            'camera': [8.5, 248.5, 8.5], 'smart_cull': True,
        },
        # 12-17: Camera outside world build height (above maxSectionY and below minSectionY).
        {
            'name': 'camera_above_world_rd2_near',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 328.5, 8.5], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'camera_above_world_rd2_far',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 408.5, 8.5], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'camera_above_world_rd2_far_no_smart_cull',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 408.5, 8.5], 'smart_cull': False,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'camera_above_world_rd4_mid',
            'min_y': -64, 'sections': 24, 'render_distance': 4,
            'camera': [8.5, 360.5, 8.5], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'camera_below_world_rd2_near',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, -72.5, 8.5], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': -90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        {
            'name': 'camera_below_world_rd2_far',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, -152.5, 8.5], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': -90.0, 'fov': 70.0, 'aspect': 1.6},
        },
        # 18-19: World boundary sealing (no stepping into sy < minSectionY or sy > maxSectionY).
        {
            'name': 'cave_wall_at_bottom_world_boundary',
            'min_y': -64, 'sections': 24, 'render_distance': 3,
            'camera': [8.5, -56.5, 8.5], 'smart_cull': True,
            'vis_overrides': [
                {'section': [1, -4, 0], 'mask': 0},
                {'section': [-1, -4, 0], 'mask': 0},
                {'section': [0, -4, 1], 'mask': 0},
                {'section': [0, -4, -1], 'mask': 0},
                {'section': [0, -3, 0], 'mask': 0},
            ],
        },
        {
            'name': 'cave_wall_at_top_world_boundary',
            'min_y': -64, 'sections': 24, 'render_distance': 3,
            'camera': [8.5, 312.5, 8.5], 'smart_cull': True,
            'vis_overrides': [
                {'section': [1, 19, 0], 'mask': 0},
                {'section': [-1, 19, 0], 'mask': 0},
                {'section': [0, 19, 1], 'mask': 0},
                {'section': [0, 19, -1], 'mask': 0},
                {'section': [0, 18, 0], 'mask': 0},
            ],
        },
        # 20: Solid floor at sy=8 with single open shaft at (1,8,0), camera at sy=10, rd=4 (reaches down to sy=6, not 5).
        {
            'name': 'cave_floor_with_shaft_and_vertical_clamp',
            'min_y': -64, 'sections': 24, 'render_distance': 4,
            'camera': [8.5, 168.5, 8.5], 'smart_cull': True,
            'vis_overrides': [
                {'section': [x, 8, z], 'mask': 0}
                for x in range(-4, 5) for z in range(-4, 5)
                if not (x == 1 and z == 0)
            ],
        },
        # 21: Distant ray culling (>60 blocks) combined with vertical clamp at rd=8.
        {
            'name': 'distant_ray_and_vertical_clamp_rd8',
            'min_y': -64, 'sections': 24, 'render_distance': 8,
            'camera': [3.3, 134.3, -5.6], 'smart_cull': True,
            'vis_overrides': [
                {'section': [2, y, z], 'mask': 0}
                for z in range(-2, 3) for y in range(0, 17)
            ],
        },
        # 22: Solid roof at sy=4 with single hole at (2,4,0): vanilla never turns back -X above the roof.
        {
            'name': 'cave_roof_hole_no_sky_band_overdraw',
            'min_y': -64, 'sections': 24, 'render_distance': 4,
            'camera': [8.5, 40.5, 8.5], 'smart_cull': True,
            'vis_overrides': [
                {'section': [x, 4, z], 'mask': 0}
                for x in range(-4, 5) for z in range(-4, 5)
                if not (x == 2 and z == 0)
            ],
        },
        # 23-24: Outside-height Octree branch culling on single-direction frusta (above and below build height).
        {
            'name': 'outside_height_octree_above_rd2_square70',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [0.0, 400.0, 0.0], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': 90.0, 'fov': 70.0, 'aspect': 1.0},
        },
        {
            'name': 'outside_height_octree_below_rd2_square35',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [0.0, -160.0, 0.0], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': -90.0, 'fov': 35.0, 'aspect': 1.0},
        },
        {
            'name': 'outside_height_octree_below_rd2_square39_tight_bounds',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [0.0, -160.0, 0.0], 'smart_cull': True,
            'frustum': {'yaw': 0.0, 'pitch': -90.0, 'fov': 39.0, 'aspect': 1.0},
            'tight_bounds': [
                {'section': [2, -4, 0], 'min': [14.0, 0.0, 0.0], 'max': [15.0, 16.0, 16.0]},
            ],
        },
        # 25-26: Face-closed barrier with smart_cull on vs off (bounded graph still clamps when off).
        {
            'name': 'barrier_wall_smart_cull_on',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': True,
            'vis_overrides': [
                {'section': [1, 6, 0], 'mask': 0},
                {'section': [-1, 6, 0], 'mask': 0},
                {'section': [0, 7, 0], 'mask': 0},
                {'section': [0, 5, 0], 'mask': 0},
                {'section': [0, 6, 1], 'mask': 0},
                {'section': [0, 6, -1], 'mask': 0},
            ],
        },
        {
            'name': 'barrier_wall_smart_cull_off',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': False,
            'vis_overrides': [
                {'section': [1, 6, 0], 'mask': 0},
                {'section': [-1, 6, 0], 'mask': 0},
                {'section': [0, 7, 0], 'mask': 0},
                {'section': [0, 5, 0], 'mask': 0},
                {'section': [0, 6, 1], 'mask': 0},
                {'section': [0, 6, -1], 'mask': 0},
            ],
        },
        # 27-28: Unloaded chunk stripe at X=1 blocks propagation into loaded X=2, then resumes on load.
        {
            'name': 'unloaded_stripe_smart_cull_on',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': True,
            'excluded_loaded_chunks': [[1, z] for z in range(-3, 4)],
            'partial_step': {'add_loaded_chunks': [[1, 0]]},
        },
        {
            'name': 'unloaded_stripe_smart_cull_off',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': False,
            'excluded_loaded_chunks': [[1, z] for z in range(-3, 4)],
            'partial_step': {'add_loaded_chunks': [[1, 0]]},
        },
        # 29: Partial propagation uses the current eye across a shifted vertical band before a full update.
        {
            'name': 'partial_update_crosses_shifted_vertical_band',
            'min_y': -64, 'sections': 24, 'render_distance': 2,
            'camera': [8.5, 104.5, 8.5], 'smart_cull': True,
            'vis_overrides': [{'section': [0, 8, 0], 'mask': 0}],
            'partial_step': {
                'camera': [8.5, 120.5, 8.5],
                'vis_overrides': [{'section': [0, 8, 0], 'mask': ALL_VISIBLE}],
            },
        },
        # 30: Taller 32-section world layout with visible rows above default 24 sections and rejected (4, sy, 4) corner.
        {
            'name': 'vertical_clamp_tall_layout_32_rd4',
            'min_y': -64, 'sections': 32, 'render_distance': 4,
            'camera': [8.5, 360.5, 8.5], 'smart_cull': True,
        },
    ]
    return {'cases': cases}


def record(lock, jars, libraries, build):
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / 'visibility'
    witness_dir.mkdir(parents=True, exist_ok=True)
    cases_path = build / 'cases.json'
    cases_path.write_text(json.dumps(build_cases(), indent=2) + '\n', encoding='utf-8')
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, ORACLE, jars, libraries, lock['minecraft']['java_major']
    )
    raw_out = build / 'witness.json'
    harness.java_run(build, classes, dependencies, ORACLE, [cases_path, raw_out], 'client')
    data = json.loads(raw_out.read_text(encoding='utf-8'))
    lines = [
        '{',
        f'  "meta": {json.dumps(data["meta"], separators=(",", ":"))},',
        '  "cases": [',
    ]
    for i, case in enumerate(data['cases']):
        comma = ',' if i + 1 < len(data['cases']) else ''
        lines.append(f'    {json.dumps(case, separators=(",", ":"))}{comma}')
    lines.extend(['  ]', '}'])
    witness = witness_dir / 'witness.json'
    witness.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java_ver,
        command='python3 tools/test_visibility_parity.py',
        oracle=f'mods/sunrise/src/test/java/dev/sunriselab/parity/{ORACLE}.java',
        inputs_sha256={
            'oracle': lab.digest(oracle_path, 'sha256'),
            'cases': lab.digest(cases_path, 'sha256'),
        },
    ))
    return witness


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run([str(c) for c in command], cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record-only', action='store_true')
    parser.add_argument('--profile', default='fast', help='Cargo build profile')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'visibility-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, libraries, build)
    if args.record_only:
        print(f'wrote {witness} ({witness.stat().st_size} bytes)')
        return
    log = run_logged(
        [
            'cargo', 'run', '--quiet', '--locked', '--profile', args.profile,
            '-p', 'sunrise-client-lib', '--example', 'client_probe', '--',
            'visibility_fixture', witness.resolve(),
        ],
        build, 'visibility-probe',
    )
    print(log.read_text(encoding='utf-8').strip())


if __name__ == '__main__':
    main()
