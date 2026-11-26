#!/usr/bin/env python3
"""Compare first-tick lava burn, dry-air burn, and absorption damage with vanilla 26.3.

Records the vanilla 26.3 witness and meta.json into .local/witness/<version>/first_tick_lava_burn/
through VanillaCommandExecOracle (reused via tools/parity_shim.py when run under tools/parity.py),
then runs mobs::ai::burn_and_rose_tests::a_mob_loaded_in_lava_burns_on_its_first_tick in
sunrise-client-lib against that file via SUNRISE_FIRST_TICK_LAVA_BURN_WITNESS.
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import first_tick_lava_burn_corpus as corpus, lab

TEST = 'mobs::ai::burn_and_rose_tests::a_mob_loaded_in_lava_burns_on_its_first_tick'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument(
        '--record', '--record-only', dest='record_only', action='store_true',
        help='record the witness and meta.json into .local without running the Rust test',
    )
    parser.add_argument('--work-dir', type=Path, help='scratch directory for the Java oracle run')
    parser.add_argument(
        '--resources', type=Path,
        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)),
    )
    args = parser.parse_args()
    build = args.work_dir.resolve() if args.work_dir else lab.LOCAL / 'first-tick-lava-burn-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = corpus.record_witness(
        work_dir=build,
        prepare=args.prepare,
        command='python3 tools/test_first_tick_lava_burn_parity.py',
    )
    if args.record_only:
        print(f'Recorded first-tick lava burn witness to {witness}')
        return
    env = lab.game_env(
        os.environ,
        SUNRISE_FIRST_TICK_LAVA_BURN_WITNESS=str(witness.resolve()),
        SUNRISE_TEST_RESOURCES=str(args.resources.resolve()),
    )
    log_path = build / 'first-tick-lava-burn-test.log'
    with log_path.open('w', encoding='utf-8') as log:
        result = subprocess.run(
            ['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
             '--', '--ignored', '--exact', TEST],
            cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT, env=env,
        )
    log_text = log_path.read_text(encoding='utf-8', errors='replace')
    if result.returncode != 0 or '1 passed; 0 failed' not in log_text:
        tail = log_text.splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        raise SystemExit(f'{TEST} failed against the recorded witness; see {log_path}')
    print(f'First-tick lava burn parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
