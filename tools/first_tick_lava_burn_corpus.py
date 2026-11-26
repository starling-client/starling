#!/usr/bin/env python3
"""Minecraft 26.3 first-tick lava burn corpus and vanilla witness recorder."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

GROUP = 'mob first-tick lava burn'
# (in_lava, initial_fire, initial_absorption)
CASES = (
    (True, -1, 0.0),
    (True, 0, 0.0),
    (True, 1, 0.0),
    (True, 19, 0.0),
    (True, 20, 0.0),
    (True, 21, 0.0),
    (True, 40, 0.0),
    (True, 320, 0.0),
    (True, 321, 0.0),
    (True, 330, 0.0),
    (False, 20, 0.0),
    (False, 21, 0.0),
    (True, 20, 2.5),
    (True, 21, 2.5),
    (True, 40, 2.5),
)
TICKS = (1, 2, 10, 11)
SPOTS = (
    (2, 2), (6, 2), (10, 2), (14, 2),
    (2, 6), (6, 6), (10, 6), (14, 6),
    (2, 10), (6, 10), (10, 10), (14, 10),
    (2, 14), (6, 14), (10, 14),
)
WITNESS_DIR = lab.LOCAL / 'witness' / lab.VERSION / 'first_tick_lava_burn'
WITNESS = WITNESS_DIR / 'first_tick_lava_burn.tsv'
WITNESS_ENV = 'SUNRISE_FIRST_TICK_LAVA_BURN_WITNESS'


def first_tick_lava_burn(cmd, setup):
    """Summon NoAI pigs in lava or dry air with initial Fire and AbsorptionAmount and read HurtTime, Fire, Health and AbsorptionAmount."""
    x0, y0, z0 = 3760, 260, 3776
    # Entities tick once the forced chunk's 2-column neighbourhood is loaded.
    chunks = [[(x0 >> 4) + i, (z0 >> 4) + k] for i in range(-2, 3) for k in range(-2, 3)]
    ops = [
        cmd(f'/forceload add {x0} {z0}', False),
        {'op': 'wait_chunks', 'chunks': chunks},
        cmd('/tick freeze', False),
        cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 + 2} {z0 + 15} air', False),
        cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0} {z0 + 15} stone', False),
    ]
    for i, ((dx, dz), (in_lava, fire, absorption)) in enumerate(zip(SPOTS, CASES), 1):
        x, z = x0 + dx, z0 + dz
        block = 'minecraft:lava' if in_lava else 'minecraft:air'
        ops.append(cmd(f'/setblock {x} {y0} {z} {block}', False))
        extra = f',AbsorptionAmount:{absorption}f' if absorption != 0.0 else ''
        ops.append(cmd(
            f'/summon minecraft:pig {x}.5 {y0} {z}.5 '
            f'{{UUID:[I;2674,0,0,{i}],NoAI:1b,PersistenceRequired:1b,Fire:{fire}s{extra}}}',
            expect_result=1,
        ))
    done = 0
    for target in TICKS:
        ops += [cmd('/tick unfreeze', False), {'op': 'tick', 'n': target - done}, cmd('/tick freeze', False)]
        done = target
        for i in range(1, len(CASES) + 1):
            uid = f'00000a72-0000-0000-0000-{i:012x}'
            for field in ('HurtTime', 'Fire', 'Health', 'AbsorptionAmount'):
                ops.append(cmd(f'/data get entity {uid} {field}'))
    return ops + setup(
        '/tick unfreeze',
        f'/kill @e[type=!player,x={x0},y={y0 - 4},z={z0},dx=16,dy=12,dz=16]',
        f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 + 2} {z0 + 15} air',
        f'/forceload remove {x0} {z0}',
    )


def _snbt_value(row):
    text = ''.join(span[0] for span in row['messages'][0])
    return text.split(': ', 1)[1]


def _java_version():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def record_witness(work_dir=None, out=None, prepare=False, command=None):
    """Run VanillaCommandExecOracle on the pinned server jar and write the TSV witness and meta.json into .local."""
    root = lab.ROOT
    scratch = Path(work_dir).resolve() if work_dir else lab.LOCAL / 'first-tick-lava-burn-parity'
    cmd = [
        sys.executable, str(root / 'tools/test_command_exec_parity.py'),
        '--only', GROUP, '--reference-only', '--work-dir', str(scratch),
    ]
    if prepare:
        cmd.append('--prepare')
    subprocess.run(cmd, cwd=root, check=True)
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']
    rows = [json.loads(line) for line in (scratch / 'vanilla.jsonl').read_text(encoding='utf-8').splitlines()]
    expected_count = len(CASES) + len(TICKS) * len(CASES) * 4
    if len(rows) != expected_count:
        raise RuntimeError(f'expected {expected_count} vanilla records, got {len(rows)}')
    idx = len(CASES)
    lines = [
        f'# vanilla {version} witness recorded via VanillaCommandExecOracle (python3 tools/test_first_tick_lava_burn_parity.py)',
        'in_lava\tinitial_fire\tinitial_absorption_bits\tticks\thurt_time\tfire\thealth_bits\tabsorption_bits',
    ]
    for target in TICKS:
        for i, (in_lava, initial_fire, initial_absorption) in enumerate(CASES, 1):
            uid = f'00000a72-0000-0000-0000-{i:012x}'
            values = {}
            for field in ('HurtTime', 'Fire', 'Health', 'AbsorptionAmount'):
                row = rows[idx]
                idx += 1
                expected_cmd = f'/data get entity {uid} {field}'
                if row['cmd'] != expected_cmd:
                    raise RuntimeError(f'unexpected record {row["cmd"]!r}, want {expected_cmd!r}')
                values[field] = _snbt_value(row)
            hurt_time = int(values['HurtTime'].removesuffix('s'))
            fire = int(values['Fire'].removesuffix('s'))
            health = float(values['Health'].removesuffix('f'))
            absorption = float(values['AbsorptionAmount'].removesuffix('f'))
            init_abs_bits = struct.pack('>f', initial_absorption).hex()
            health_bits = struct.pack('>f', health).hex()
            abs_bits = struct.pack('>f', absorption).hex()
            lava_flag = 1 if in_lava else 0
            lines.append(
                f'{lava_flag}\t{initial_fire}\t{init_abs_bits}\t{target}\t'
                f'{hurt_time}\t{fire}\t{health_bits}\t{abs_bits}'
            )
    out = Path(out) if out is not None else lab.LOCAL / 'witness' / version / 'first_tick_lava_burn' / 'first_tick_lava_burn.tsv'
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    oracle_path = root / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
    input_sources = (
        oracle_path,
        root / 'tools/first_tick_lava_burn_corpus.py',
        root / 'tools/test_command_exec_parity.py',
        root / 'tools/test_first_tick_lava_burn_parity.py',
    )
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'server.jar': lab.digest(jars['server'], 'sha256'),
    }
    for src in input_sources:
        input_hashes[src.relative_to(root).as_posix()] = lab.digest(src, 'sha256')
    lab.write_json(out.parent / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': _java_version(),
        'harness': 'tools/test_first_tick_lava_burn_parity.py',
        'witness_env': WITNESS_ENV,
        'command': command or 'python3 tools/first_tick_lava_burn_corpus.py --record',
        'oracle': oracle_path.relative_to(root).as_posix(),
        'input_hashes': input_hashes,
        'outputs': {
            out.name: {'sha256': lab.digest(out, 'sha256'), 'bytes': out.stat().st_size},
        },
    })
    print(f'Wrote {len(lines) - 2} {version} witness rows to {out}')
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true', help='record the witness into .local via VanillaCommandExecOracle')
    parser.add_argument('--prepare', action='store_true', help='prepare pinned vanilla artifacts first')
    parser.add_argument('--work-dir', type=Path, help='scratch directory for the Java oracle run')
    parser.add_argument('--out', type=Path, default=WITNESS, help='output TSV path (default under .local/witness/)')
    args = parser.parse_args()
    if not args.record:
        parser.error('pass --record to generate the witness')
    record_witness(args.work_dir, args.out, prepare=args.prepare)


if __name__ == '__main__':
    main()
