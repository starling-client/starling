#!/usr/bin/env python3
"""Compare what a GameEvent's parameter does with ClientPacketListener.handleGameEvent.

The oracle runs the unmodified handler for CHANGE_GAME_MODE and GUARDIAN_ELDER_EFFECT on each parameter and
records the game mode it passes to MultiPlayerGameMode.setLocalMode and whether it plays the elder guardian's
curse sound. The parameters: whole numbers and the halves around them, the floats next to each (where the float
sum with 0.5 rounds), signed zeros, infinities, NaNs, values past the int range, and seeded random bit patterns
and small values. The apparition, the screens of the other events and sound playback are outside scope.
"""
import os
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def neighbours(value):
    b = bits(value)
    return [b - 1, b, b + 1] if b & 0x7fffffff else [b, b + 1]


def inputs():
    out = []
    for n in range(-6, 8):
        for v in (n, n + 0.5):
            out += neighbours(float(v))
    out += [bits(v) for v in (0.0, -0.0, float('inf'), float('-inf'), 2.0 ** 31, -(2.0 ** 31), 3e9, -3e9,
                              8388607.5, 8388608.0, 16777216.0, 3.4028235e38, -3.4028235e38)]
    out += [0x7fc00000, 0xffc00000, 0x7f800001, 0x00000001, 0x80000001]
    rng = random.Random(1101)
    out += [rng.getrandbits(32) for _ in range(100_000)]
    out += [bits(rng.uniform(-8.0, 8.0)) for _ in range(100_000)]
    return [b & 0xffffffff for b in out]


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'game-event-param-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    # The packet listener links client-only libraries.
    _, game_dir = lab.config()
    libraries += sorted((game_dir / 'libraries/client').rglob('*.jar'))
    classpath = os.pathsep.join(map(str, [jars['client'], *libraries]))
    params = build / 'inputs.txt'
    params.write_text(''.join(f'{b:08x}\n' for b in inputs()))
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaGameEventParamOracle.java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-cp', classpath, '-d', str(classes), str(oracle)], check=True)
    subprocess.run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + classpath,
                    'dev.sunriselab.parity.VanillaGameEventParamOracle', str(params), str(build / 'vanilla.txt')],
                   check=True)
    with (build / 'rust.txt').open('w') as out:
        subprocess.run(['cargo', 'run', '--quiet', '-p', 'sunrise-client-lib', '--example', 'client_probe', '--',
                        'game_event_param_fixture', str(params)], cwd=lab.ROOT, stdout=out, check=True)
    matched = base.compare(build / 'vanilla.txt', build / 'rust.txt')
    print(f'Game event parameter parity passed: {matched} parameters give vanilla\'s game mode and curse sound')


if __name__ == '__main__':
    main()
