#!/usr/bin/env python3
"""Compare real 26.3's dialog codec with the answers Sunrise's tests pin.

The fixture lists JSON dialogs, packet NBT dialogs and small data packs,
with the codec's acceptance, selected round-trip values and diagnostics. The oracle runs the real codecs over the same cases and this
harness fails on any answer that differs. Sunrise's side is the unit test
`dialog_admission_answers_as_the_real_codecs_do` (`sunrise-client`, `lan/data_pack_registries.rs`), which
reads the same file."""
import json
import os
import subprocess

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

FIXTURE = lab.ROOT / 'crates/sunrise-client/tests/fixtures/dialog_codec_cases.json'


def main():
    build = lab.LOCAL / 'dialog-codec-parity'
    build.mkdir(parents=True, exist_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaDialogCodecOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath',
         lab.ROOT / 'mods/sunrise/src/test/java', '-cp', str(jars['server']) + os.pathsep + dependencies,
         '-d', classes, oracle], 'compile-java')
    answers = build / 'answers.jsonl'
    # Vanilla's recursive DependencySorter needs a larger stack for the deep tag chain.
    run(['java', '-Xmx1g', '-Xss32m', '-Djava.awt.headless=true', '-cp',
         os.pathsep.join([str(classes), str(jars['server']), dependencies]),
         'dev.sunriselab.parity.VanillaDialogCodecOracle', FIXTURE, answers], 'oracle')
    recorded = json.loads(FIXTURE.read_text(encoding='utf-8'))
    recorded = {case['name']: case for case in recorded['cases'] + recorded['packets'] + recorded['packs']}
    seen = set()
    for line in answers.read_text(encoding='utf-8').splitlines():
        answer = json.loads(line)
        case = recorded[answer['name']]
        seen.add(answer['name'])
        if answer['accept'] != case['accept']:
            raise AssertionError(f"{answer['name']}: fixture says {case['accept']} {case.get('vanilla_error')!r}, "
                                 f"real codec says {answer['accept']} {answer.get('error')!r}")
        if answer.get('round_trip') != case.get('round_trip'):
            raise AssertionError(f"{answer['name']}: expected {case.get('round_trip')!r}, "
                                 f"real round-trip yielded {answer.get('round_trip')!r}")
    if seen != set(recorded):
        raise AssertionError('the oracle skipped cases: ' + ', '.join(sorted(set(recorded) - seen)))
    print(f'{len(seen)} dialog codec answers match the fixture')


if __name__ == '__main__':
    main()
