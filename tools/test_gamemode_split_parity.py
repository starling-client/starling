#!/usr/bin/env python3
"""Compare the client's local-game-mode vs PlayerInfo split and unified client abilities with vanilla 26.3.

Compiles and runs VanillaClientGameModeSplitOracle against the pinned 26.3 client jar on JDK 25,
writing the witness and meta.json into .local/witness/<version>/gamemode_split/, then runs each
Rust test in game::remote::abilities::gamemode_split_tests in its own fresh process against it via
SUNRISE_GAMEMODE_SPLIT_WITNESS."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

TESTS = [
    'game::remote::abilities::gamemode_split_tests::change_game_mode_updates_only_local_mode_and_abilities_26_3',
    'game::remote::abilities::gamemode_split_tests::genpvp_packet_sequence_matches_26_3_oracle_step_by_step',
    'game::remote::abilities::gamemode_split_tests::hud_camera_mounts_match_26_3_oracle',
    'game::remote::abilities::gamemode_split_tests::live_player_info_updates_toggle_no_physics_while_local_spectator_26_3',
    'game::remote::abilities::gamemode_split_tests::login_and_respawn_set_previous_mode_and_fresh_abilities_26_3',
    'game::remote::abilities::gamemode_split_tests::resolved_player_info_retains_identity_across_remove_and_readd_26_3',
    'game::remote::abilities::gamemode_split_tests::respawn_keep_data_spectator_info_clears_pending_y_26_3',
    'game::remote::abilities::gamemode_split_tests::retained_player_login_preserves_custom_ability_speeds_26_3',
    'game::remote::abilities::gamemode_split_tests::split_local_and_player_info_physics_matrix_26_3',
    'game::remote::abilities::gamemode_split_tests::split_mode_hud_rendering_reach_miss_inventory_and_input_rules',
]


def record(lock, jars, libraries, build, witness_dir=None):
    version = lock['minecraft']['version']
    witness_dir = witness_dir.resolve() if witness_dir is not None else lab.LOCAL / 'witness' / version / 'gamemode_split'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'witness.json'
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, 'VanillaClientGameModeSplitOracle', jars, libraries, lock['minecraft']['java_major']
    )
    harness.java_run(build, classes, dependencies, 'VanillaClientGameModeSplitOracle', [witness], 'client')
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java_ver,
        command='python3 tools/test_gamemode_split_parity.py',
        oracle='mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaClientGameModeSplitOracle.java',
        inputs_sha256={
            'oracle': lab.digest(oracle_path, 'sha256'),
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
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record-only', action='store_true')
    parser.add_argument('--prepare-witness', type=Path,
                        help='record only the vanilla witness into this private directory')
    parser.add_argument('--env-file', type=Path,
                        help='append SUNRISE_GAMEMODE_SPLIT_WITNESS to an environment file')
    args = parser.parse_args()
    if args.prepare_witness and not args.env_file:
        parser.error('--prepare-witness requires --env-file')
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'gamemode-split-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, libraries, build, witness_dir=args.prepare_witness)
    if args.prepare_witness:
        with args.env_file.open('a', encoding='utf-8') as env_file:
            env_file.write(f'SUNRISE_GAMEMODE_SPLIT_WITNESS={witness.resolve()}\n')
        print(f'wrote {witness} ({witness.stat().st_size} bytes)')
        return
    if args.record_only:
        print(f'wrote {witness} ({witness.stat().st_size} bytes)')
        return
    with tempfile.TemporaryDirectory(prefix='sunrise-gamemode-split-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        binary = lab.parity_fixtures()
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_GAMEMODE_SPLIT_WITNESS'] = str(witness.resolve())
        for idx, test in enumerate(TESTS):
            test_env = env.copy()
            test_env['SUNRISE_TEST_OWN_PROCESS'] = test
            log = run_logged(
                [binary, '--ignored', '--exact', test, '--test-threads=1'],
                build, f'game-{idx}', cwd=scratch, env=test_env,
            )
            assert '1 passed; 0 failed' in log.read_text(encoding='utf-8'), (
                f'expected "1 passed; 0 failed" in {log}'
            )
    print(f'Game-mode split parity passed ({len(TESTS)} tests against {witness})')


if __name__ == '__main__':
    main()
