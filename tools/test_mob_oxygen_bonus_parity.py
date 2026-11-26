#!/usr/bin/env python3
"""Record the vanilla 26.3 underwater oxygen_bonus save witness into .local and run the Rust witness test.

Runs VanillaCommandExecOracle through tools/test_command_exec_parity.py
(--mob-attribute-save-fixture --reference-only --record-oxygen-bonus-witness),
writing mob_oxygen_bonus_save.json and meta.json into
.local/witness/<version>/mob_oxygen_bonus/ (gitignored). Unless --record or
--record-only is given, runs sunrise-client-lib's
mobs::ai::oxygen_bonus_tests::a_mob_losing_air_under_water_saves_oxygen_bonus
against that witness via SUNRISE_MOB_OXYGEN_BONUS_WITNESS.
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

ORACLE = 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
CORPUS = 'tools/mob_attribute_save_corpus.py'
DRIVER = 'tools/test_command_exec_parity.py'
TEST = 'mobs::ai::oxygen_bonus_tests::a_mob_losing_air_under_water_saves_oxygen_bonus'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run(
            [str(part) for part in command],
            cwd=cwd,
            env=env,
            timeout=3600,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument(
        '--record',
        '--record-only',
        action='store_true',
        dest='record_only',
        help='only record the vanilla witness and meta.json into .local/witness/<version>/mob_oxygen_bonus/',
    )
    parser.add_argument(
        '--resources',
        type=Path,
        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)),
    )
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']

    build = lab.LOCAL / 'mob-oxygen-bonus-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'mob_oxygen_bonus'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'mob_oxygen_bonus_save.json'

    record_cmd = [
        sys.executable,
        str(lab.ROOT / DRIVER),
        '--mob-attribute-save-fixture',
        '--reference-only',
        '--record-oxygen-bonus-witness',
        str(witness),
        '--work-dir',
        str(build),
    ]
    run_logged(record_cmd, build, 'record', cwd=lab.ROOT)
    if not witness.is_file():
        sys.exit(f'VanillaCommandExecOracle did not write {witness}')

    oracle_path = lab.ROOT / ORACLE
    corpus_path = lab.ROOT / CORPUS
    driver_path = lab.ROOT / DRIVER
    harness_path = Path(__file__).resolve()
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'server.jar': lab.digest(jars['server'], 'sha256'),
        ORACLE: lab.digest(oracle_path, 'sha256'),
        CORPUS: lab.digest(corpus_path, 'sha256'),
        DRIVER: lab.digest(driver_path, 'sha256'),
        harness_path.relative_to(lab.ROOT).as_posix(): lab.digest(harness_path, 'sha256'),
    }
    lab.write_json(witness_dir / 'meta.json', {
        'minecraft': version,
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_mob_oxygen_bonus_parity.py',
        'oracle': ORACLE,
        'witness_env': 'SUNRISE_MOB_OXYGEN_BONUS_WITNESS',
        'command': [str(part) for part in record_cmd],
        'input_hashes': input_hashes,
        'outputs': {
            witness.name: {
                'sha256': lab.digest(witness, 'sha256'),
                'bytes': witness.stat().st_size,
            },
        },
    })

    if args.record_only:
        print(f'Recorded mob oxygen_bonus witness into {witness} ({witness.stat().st_size} bytes)')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-mob-oxygen-bonus-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_MOB_OXYGEN_BONUS_WITNESS'] = str(witness.resolve())
        log = run_logged(
            ['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib', '--', '--ignored', '--exact', TEST],
            build,
            'rust-test',
            cwd=lab.ROOT,
            env=env,
        )
        if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log}')

    print(f'Mob oxygen_bonus parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
