#!/usr/bin/env python3
"""Check teleport hooks through real Game actors and the owning mob tick loop.

This is a Sunrise hook integration proof, not an independent vanilla oracle.
The teleport_consume, pearl_death, pearl_wind_charge and mob_ai harnesses
separately cover the shared vanilla algorithms and gameplay consumers.
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import integration_env, lab, test_rust_parity as base


CASES = (
    'consumption_hook_keeps_the_stack_before_use_and_cancellation_stops_attempts',
    'player_pearl_keeps_acting_context_cancellation_and_mutable_damage',
    'mob_pearls_use_the_hook_and_recheck_listener_writes_without_player_side_effects',
    'fox_consumption_adapter_uses_full_host_and_preserves_listener_writes',
    'fox_tick_pauses_before_movement_and_next_actor_and_preserves_full_host_writes',
)


def main():
    build = lab.LOCAL / 'mob-item-teleports'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    sources = [Path(__file__), lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py']
    sources += sorted((lab.ROOT / 'crates').rglob('*.rs'))
    hashes = base.source_hashes(sources)
    executable = lab.parity_fixtures(default_features=False)
    listed = subprocess.check_output([executable, '--list', '--ignored'], text=True)
    passed = []
    for case in CASES:
        matches = [line.removesuffix(': test') for line in listed.splitlines()
                   if line.endswith('::' + case + ': test')]
        assert len(matches) == 1, (case, matches)
        with tempfile.TemporaryDirectory(prefix='sunrise-mob-item-teleports-') as scratch:
            env = integration_env.game_environment(os.environ, scratch)
            env['SUNRISE_TEST_RESOURCES'] = str(lab.RESOURCES)
            with (build / (case + '.log')).open('w') as output:
                subprocess.run([executable, matches[0], '--exact', '--ignored',
                                '--test-threads=1', '--nocapture'], cwd=scratch, env=env,
                               stdout=output, stderr=subprocess.STDOUT, check=True, timeout=900)
        result = (build / (case + '.log')).read_text()
        assert re.search(r'^test result: ok\. 1 passed;', result, re.MULTILINE), result
        passed.append(matches[0])
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, sunrise_only=True, resource_tests=len(passed),
        cases=passed, source_hashes=hashes,
        rust_test_executable_sha256=lab.digest(Path(executable), 'sha256'),
        scope='Player captured API stack count, damage and components, cancellation RNG prefix '
              'and listener move preservation; player pearl acting context, cancellation, '
              'mutable damage and listener move preservation; mob pearl cancellation, '
              'redirect, stale decision refusal, rotations and no player side effects. '
              'Fox full-host cancellation, redirect, item/move/removal writes and actor RNG. '
              'Real Game mob batch pauses before fox movement and the next actor, resumes '
              'once, preserves other-actor writes/removal and defers callback-spawned mobs. '
              'Does not drive a multiplayer guest transport or draw a GUI.'))
    print(f'{len(passed)} mob/item teleport hook resource regressions passed: {report}', flush=True)


if __name__ == '__main__':
    main()
