#!/usr/bin/env python3
"""Check client placement and redstone update guards against vanilla.

The Java oracle runs shipped block and ClientLevel methods without a game
window. The Rust tests run placement and neighbor updates through DryLevel,
with server-side controls. This does not replay a flying machine.
"""
import os
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def main():
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES / 'block-behaviours.json'
    if not resources.is_file():
        raise FileNotFoundError(resources)
    work = lab.LOCAL / 'client-redstone-preview-parity'
    work.mkdir(parents=True, exist_ok=True)
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaClientRedstonePreviewOracle.java'
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    base.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
              '-cp', dependencies, '-d', str(work), str(oracle)])
    base.run(['java', '-cp', str(work) + os.pathsep + dependencies,
              'dev.sunriselab.parity.VanillaClientRedstonePreviewOracle'])
    for case in ('placement_preview_checks_obstruction_without_changing_the_level',
                 'redstone_updates_respect_the_preview_level_side'):
        test = f'game::remote::tests::placement_preview_tests::{case}'
        recording = work / f'{case}.log'
        with recording.open('w', encoding='utf-8') as output:
            base.run(['cargo', 'test', '--locked', '-p', 'sunrise-client', '--lib',
                      test, '--', '--exact', '--ignored'], stdout=output,
                     env={**os.environ, "SUNRISE_TEST_RESOURCES": str(lab.RESOURCES)})
        if f'test {test} ... ok' not in recording.read_text(encoding='utf-8').splitlines():
            raise AssertionError(f'{test} did not run')
    print('Client placement and redstone update guards match vanilla; server controls pass.')


if __name__ == '__main__':
    main()
