#!/usr/bin/env python3
"""Record the vanilla shulker motion/yaw witness into .local and run the Rust witness tests.

Runs VanillaCommandExecOracle (via tools/test_command_exec_parity.py --record-shulker-motion)
on the `shulker motion and yaw` group, writing shulker_motion.json and meta.json under
.local/witness/<version>/shulker_motion/ (gitignored). Unless --record / --record-only is
given, runs the two shulker motion witness tests in sunrise-client-lib against that witness
through SUNRISE_SHULKER_MOTION_WITNESS.

  test_shulker_motion_parity.py [--prepare] [--record] [--work-dir DIR] [--resources DIR]
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab

TESTS = [
    'mobs::shulker::saved_bytes_tests::a_loaded_shulker_has_no_motion_and_keeps_its_yaw',
    'mobs::shulker::saved_bytes_tests::a_noai_shulker_in_flowing_water_keeps_zero_motion',
]


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run([str(part) for part in command], cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def build_client_lib_test_binary(build, env):
    compiled = run_logged(
        ['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib', '--no-run', '--message-format=json'],
        build, 'compile-sunrise-client-lib', env=env,
    )
    binaries = []
    for line in compiled.read_text(encoding='utf-8').splitlines():
        if not line.startswith('{'):
            continue
        row = json.loads(line)
        if (row.get('reason') == 'compiler-artifact' and row.get('executable')
                and row.get('target', {}).get('name') == 'sunrise_client_lib'
                and 'lib' in row.get('target', {}).get('kind', ())
                and row.get('profile', {}).get('test')):
            binaries.append(Path(row['executable']))
    if len(binaries) != 1:
        sys.exit(f'expected one sunrise_client_lib test binary, found {binaries}')
    return binaries[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', action='store_true', dest='record_only',
                        help='record the vanilla witness and meta.json into .local without running Rust tests')
    parser.add_argument('--work-dir', type=Path, default=None,
                        help='scratch directory for the oracle run (default: .local/shulker-motion-parity)')
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    lock, _ = lab.config()
    version = lock['minecraft']['version']
    build = args.work_dir.resolve() if args.work_dir else lab.LOCAL / 'shulker-motion-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'shulker_motion'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_path = witness_dir / 'shulker_motion.json'

    record_cmd = [
        sys.executable,
        str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--work-dir', str(build),
        '--record-shulker-motion', str(witness_path),
    ]
    if args.prepare:
        record_cmd.append('--prepare')
    result = subprocess.run(record_cmd, cwd=lab.ROOT)
    if result.returncode:
        sys.exit(result.returncode)
    if not witness_path.is_file() or not (witness_dir / 'meta.json').is_file():
        sys.exit(f'expected witness and meta.json under {witness_dir}')

    if args.record_only:
        print(f'Recorded shulker motion witness into {witness_dir}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-shulker-motion-parity-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_SHULKER_MOTION_WITNESS'] = str(witness_path.resolve())
        binary = build_client_lib_test_binary(build, env)
        log = run_logged([binary, '--ignored', '--exact', *TESTS], build, 'rust-tests', cwd=scratch, env=env)
        expected = f'{len(TESTS)} passed; 0 failed'
        if expected not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected "{expected}" in {log}')

    print(f'Shulker motion parity passed ({len(TESTS)} tests against {witness_path})')


if __name__ == '__main__':
    main()
