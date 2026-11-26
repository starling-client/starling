#!/usr/bin/env python3
"""Record the vanilla 26.3 armor stand low-health/DeathTime witness into .local and run the Rust fixture.

Compiles and runs VanillaCommandExecOracle against the pinned 26.3 server jar on JDK 25 using
the stand_low_health corpus (tools/stand_command_hurt_corpus.py), writes stand_low_health.tsv
and meta.json into .local/witness/<version>/stand_low_health/, and then runs the sunrise-client
witness fixture through lab.parity_fixtures() with SUNRISE_STAND_LOW_HEALTH_WITNESS.

  test_stand_low_health_parity.py [--prepare] [--record-only] [--resources DIR] [--witness-dir DIR]
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base
from tools.stand_command_hurt_corpus import LOW_HEALTH_CASES, stand_low_health

SEED = 1
TEST_NAME = 'game::stand_low_health_tests::a_stand_breaks_on_low_health_only_through_damage_and_a_dead_one_goes_at_twenty'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def plain_message(row):
    return ''.join(span[0] for line in row['messages'] for span in line)


def parse_low_health_witness(ops, rows, version):
    grouped = {}
    order = []
    for op, row in zip(ops, rows):
        meta = op.get('witness')
        if meta is None:
            continue
        name, nbt, tag, step_name, field = meta
        key = (name, tag)
        if key not in grouped:
            grouped[key] = {'case': name, 'nbt': nbt, 'step': step_name}
            order.append(key)
        entry = grouped[key]
        text = plain_message(row)
        results = row['results']
        if field == 'alive':
            assert len(results) == 1 and results[0][0] in (True, False), (op, row)
            entry['alive'] = 1 if results[0][0] else 0
        elif field == 'health':
            if results and results[0][0]:
                match = re.search(r'has the following entity data: ([-0-9.eE+]+)f$', text)
                assert match, (op, text)
                bits = struct.unpack('>I', struct.pack('>f', float(match.group(1))))[0]
                entry['health_bits'] = f'{bits:08x}'
            else:
                entry['health_bits'] = '-'
        elif field == 'deathtime':
            if results and results[0][0]:
                match = re.search(r'has the following entity data: (-?\d+)s$', text)
                assert match, (op, text)
                entry['death_time'] = match.group(1)
            else:
                entry['death_time'] = '-'
        elif field == 'drops':
            assert len(results) == 1, (op, row)
            entry['drop_count'] = results[0][1] if results[0][0] else 0
        elif field == 'drop_item':
            if results and results[0][0]:
                match = re.search(r'id: "([^"]+)"', text)
                assert match, (op, text)
                entry['drop_item'] = match.group(1)
            else:
                entry['drop_item'] = '-'
    case_order = {name: i for i, (name, *_) in enumerate(LOW_HEALTH_CASES)}
    by_case = {name: [] for name, *_ in LOW_HEALTH_CASES}
    for key in order:
        by_case[key[0]].append(grouped[key])
    lines = [
        f'# Minecraft {version} vanilla armor stand low-health and DeathTime witness (recorded by tools/test_stand_low_health_parity.py)',
        'case\tnbt\tstep\talive\thealth_bits\tdeath_time\tdrop_count\tdrop_item',
    ]
    for name in sorted(by_case, key=case_order.__getitem__):
        for entry in by_case[name]:
            lines.append('\t'.join([
                entry['case'],
                entry['nbt'],
                entry['step'],
                str(entry['alive']),
                entry['health_bits'],
                entry['death_time'],
                str(entry['drop_count']),
                entry['drop_item'],
            ]))
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record-only', '--record', action='store_true', dest='record_only',
                        help='record the vanilla witness into .local without running the Rust fixture')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--witness-dir', type=Path, default=None,
                        help='override the witness output directory (default: .local/witness/<version>/stand_low_health)')
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    version = lock['minecraft']['version']
    java_major = str(lock['minecraft']['java_major'])

    build = lab.LOCAL / 'stand-low-health-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    witness_dir = args.witness_dir.resolve() if args.witness_dir else (lab.LOCAL / 'witness' / version / 'stand_low_health')
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'stand_low_health.tsv'

    ops = stand_low_health(full=False)
    script_path = build / 'script.jsonl'
    script_path.write_text(
        ''.join(
            json.dumps({k: v for k, v in op.items() if k != 'expect_result'}, ensure_ascii=False) + '\n'
            for op in ops
        ),
        encoding='utf-8',
    )

    def run(command, name, cwd=lab.ROOT, env=None):
        path = build / f'{name}.log'
        with path.open('w', encoding='utf-8') as log:
            result = subprocess.run([str(c) for c in command], cwd=cwd, env=env, timeout=3600,
                                    stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            tail = path.read_text(encoding='utf-8', errors='replace')[-3000:]
            sys.exit(f'{name} failed ({result.returncode}): see {path}\n{tail}')
        return path

    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
    deps = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', java_major, '-proc:none', '-cp', str(jars['server']) + os.pathsep + deps,
         '-d', classes, oracle], 'compile-java')

    vanilla_dir = build / 'vanilla'
    shutil.rmtree(vanilla_dir, ignore_errors=True)
    vanilla_dir.mkdir()
    raw_output = build / 'vanilla.jsonl'
    oracle_cmd = [
        'java', '-Xmx3G', '-Duser.timezone=UTC',
        '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
        'dev.sunriselab.parity.VanillaCommandExecOracle',
        vanilla_dir / 'universe', script_path, raw_output, SEED,
    ]
    run(oracle_cmd, 'java', cwd=vanilla_dir)

    recorded = [op for op in ops if op['op'] == 'cmd' and op.get('record', True)]
    java_rows = [json.loads(line) for line in raw_output.read_text(encoding='utf-8').splitlines()]
    if len(recorded) != len(java_rows):
        sys.exit(f'Java oracle record count differs: expected {len(recorded)}, got {len(java_rows)}')
    for op, row in zip(recorded, java_rows):
        if 'expect_result' in op and row.get('results') != [[True, op['expect_result']]]:
            sys.exit(f'Java fixture guard failed: {op["cmd"]}: {row.get("results")}')

    witness.write_text(parse_low_health_witness(recorded, java_rows, version), encoding='utf-8')
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_stand_low_health_parity.py',
        'witness_env': 'SUNRISE_STAND_LOW_HEALTH_WITNESS',
        'command': [str(part) for part in oracle_cmd],
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java': lab.digest(oracle, 'sha256'),
            'tools/stand_command_hurt_corpus.py': lab.digest(lab.ROOT / 'tools/stand_command_hurt_corpus.py', 'sha256'),
            'script.jsonl': lab.digest(script_path, 'sha256'),
        },
        'output': {
            'file': witness.name,
            'sha256': lab.digest(witness, 'sha256'),
            'bytes': witness.stat().st_size,
        },
    })

    if args.record_only:
        print(f'Recorded vanilla {version} stand low-health witness into {witness}')
        return

    binary = lab.parity_fixtures()
    with tempfile.TemporaryDirectory(prefix='sunrise-stand-low-health-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_STAND_LOW_HEALTH_WITNESS'] = str(witness.resolve())
        log = run([binary, TEST_NAME, '--exact', '--ignored', '--nocapture'], 'rust-test', cwd=scratch, env=env)
        if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST_NAME} in {log}')

    print(f'Stand low-health parity passed ({TEST_NAME} against {witness})')


if __name__ == '__main__':
    main()
