#!/usr/bin/env python3
"""Record the vanilla 26.3 fireball hit-turn witness into .local and run the Rust parity test.

Compiles and runs VanillaFireballHitTurnOracle against the pinned 26.3 server jar,
writing fireball_hit_turn.json and meta.json into .local/witness/<version>/fireball_hit_turn/
(gitignored), then runs projectiles::tests::a_fireball_turns_on_the_tick_it_hits against it
via SUNRISE_FIREBALL_HIT_TURN_WITNESS.
"""
import argparse
import os
import subprocess
import sys

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

ORACLE = 'VanillaFireballHitTurnOracle'
SOURCE = lab.ROOT / f'mods/sunrise/src/test/java/dev/sunriselab/parity/{ORACLE}.java'
TEST = 'projectiles::tests::a_fireball_turns_on_the_tick_it_hits'
WITNESS_ENV = 'SUNRISE_FIREBALL_HIT_TURN_WITNESS'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record_only', action='store_true',
                        help='record the vanilla witness into .local/witness/<version>/fireball_hit_turn/ and stop')
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    version = lock['minecraft']['version']
    java_major = str(lock['minecraft']['java_major'])

    build = lab.LOCAL / 'fireball-hit-turn-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'fireball_hit_turn'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_path = (witness_dir / 'fireball_hit_turn.json').resolve()

    deps = os.pathsep.join(map(str, libraries))
    cp = str(jars['server']) + os.pathsep + deps

    def run(command, name, env=None):
        log_path = build / f'{name}.log'
        with log_path.open('w', encoding='utf-8') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, env=env, timeout=3600,
                                    stdout=log, stderr=subprocess.STDOUT)
        if result.returncode != 0:
            tail = log_path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr, flush=True)
            sys.exit(f'{name} failed (exit {result.returncode}): see {log_path}')
        return log_path

    compile_cmd = ['javac', '--release', java_major, '-proc:none', '-nowarn',
                   '-cp', cp, '-d', str(classes), str(SOURCE)]
    run(compile_cmd, 'compile-java')

    # Passing "-" makes the oracle create and remove its own temp world so parity_shim tracks only the JSON output.
    run_cmd = ['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + cp,
               f'dev.sunriselab.parity.{ORACLE}', '-', str(witness_path)]
    run(run_cmd, f'oracle-{ORACLE}')
    if not witness_path.is_file():
        sys.exit(f'{ORACLE} did not write {witness_path}')

    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_fireball_hit_turn_parity.py',
        'witness_env': WITNESS_ENV,
        'oracle': SOURCE.relative_to(lab.ROOT).as_posix(),
        'command': [str(part) for part in run_cmd],
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            SOURCE.relative_to(lab.ROOT).as_posix(): lab.digest(SOURCE, 'sha256'),
        },
        'output': {
            'path': witness_path.name,
            'sha256': lab.digest(witness_path, 'sha256'),
            'bytes': witness_path.stat().st_size,
        },
    })

    if args.record_only:
        print(f'Recorded fireball hit-turn witness into {witness_dir}')
        return 0

    env = {**os.environ, WITNESS_ENV: str(witness_path)}
    log = run(['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
               '--', '--ignored', '--exact', TEST], 'rust-test', env=env)
    if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
        sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log}')
    print(f'Fireball hit-turn parity passed ({TEST} against {witness_path})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
