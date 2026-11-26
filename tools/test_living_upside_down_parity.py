#!/usr/bin/env python3
"""Compare the living model's upright and upside-down Y transform bit-for-bit."""
from itertools import product

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base


def main():
    build = lab.LOCAL / 'living-upside-down-parity'
    build.mkdir(exist_ok=True)
    # Non-power-of-two scale also catches canceling the division against the pose scale.
    cases = [' '.join(map(str, case)) for case in product(
        ('zombie', 'pig'), ('adult', 'baby'), (0.25, 0.34, 0.5, 1.0, 2.0), ('Dinnerbone', 'Grumm'))]
    cases += ['zombie adult 2.0 Alex', 'magma_cube adult 0.5 Grumm', 'magma_cube adult 2.0 Dinnerbone']
    inputs = build / 'cases.txt'
    inputs.write_text('\n'.join(cases) + '\n', encoding='utf-8')
    lock, jars, _ = base.verify_artifacts()
    oracle = 'VanillaLivingUpsideDownOracle'
    classes, dependencies, _ = harness.compile_oracle(
        build, oracle, jars, baking.client_libraries(), lock['minecraft']['java_major'])
    java_out = build / 'java.txt'
    harness.java_run(build, classes, dependencies, oracle, [inputs, java_out], 'java')

    rust_out = build / 'rust.txt'
    probe = harness.build_probe(build, 'entity_anim_fixture')
    harness.run_logged([*probe, '--living-upside-down', lab.RESOURCES, inputs, rust_out], build, 'rust')

    vanilla = java_out.read_text().splitlines()
    sunrise = rust_out.read_text().splitlines()
    if len(vanilla) != len(cases) or len(sunrise) != len(cases):
        raise AssertionError('Incomplete living render output')
    diff = [case for case, expected, actual in zip(cases, vanilla, sunrise) if expected != actual]
    if diff:
        raise AssertionError(f'{len(diff)} living render cases differ: {", ".join(diff)}')
    print(f'Living upside-down transform: {len(cases)} cases match bit-for-bit')


if __name__ == '__main__':
    main()
