#!/usr/bin/env python3
"""Compare the anvil's name checks with vanilla's StringUtil.

The oracle runs the unmodified StringUtil.filterText and StringUtil.isBlank (what AnvilMenu.setItemName and
createResult call) on a set of names, and StringUtil.isWhitespace, isAllowedChatCharacter and isBlank on every
UTF-16 unit. The names: empty, spaces, every space and line separator alone and mixed, the no-break spaces,
U+0085, controls, zero-width characters, the section sign, surrogate pairs, and seeded random names. The menu,
its cost and the item's name component are outside scope.
"""
import os
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# Characters the names are drawn from: every Java space char and whitespace, their neighbours, and others.
POOL = ([chr(c) for c in range(0x00, 0x21)] + ['\x7f', '\x85', '\xa0', '\xa7', ' ', '᠎', '​',
        '‌', '‍', ' ', ' ', ' ', ' ', '⁠', '　', '﻿', 'a', 'Z', '7',
        '\U0001f600', '\U00010000', '\U0010ffff'] + [chr(c) for c in range(0x2000, 0x200b)])
NAMES = ['', ' ', '   ', '\xa0', ' ', ' ', '\x85', ' \x85 ', '\x1f', '\t', '　  ', 'a',
         ' a ', '\xa0x\xa0', '\U0001f600', ' \U0001f600 ', ' ', '᠎', '​', '﻿', '\xa7 ', '\x7f',
         ' \xa0  　', '\xa7\xa7', '\n', ' \n ']


def names():
    rng = random.Random(127)
    out = list(NAMES)
    for _ in range(20_000):
        out.append(''.join(rng.choice(POOL) if rng.random() < 0.8 else chr(rng.choice(
            [rng.randrange(0x20, 0xd800), rng.randrange(0xe000, 0x10000), rng.randrange(0x10000, 0x110000)]))
            for _ in range(rng.randrange(1, 9))))
    return out


def encode(name):
    units = name.encode('utf-16-le')
    return ' '.join(f'{int.from_bytes(units[i:i + 2], "little"):04x}' for i in range(0, len(units), 2)) or '-'


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'anvil-name-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    classpath = os.pathsep.join(map(str, [jars['server'], *libraries]))
    inputs = build / 'names.txt'
    inputs.write_text(''.join(encode(n) + '\n' for n in names()))
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaAnvilNameOracle.java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-cp', classpath, '-d', str(classes), str(oracle)], check=True)
    subprocess.run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + classpath,
                    'dev.sunriselab.parity.VanillaAnvilNameOracle', str(inputs), str(build / 'vanilla.txt')],
                   check=True)
    with (build / 'rust.txt').open('w') as out:
        subprocess.run(['cargo', 'run', '--quiet', '-p', 'sunrise-client-lib', '--example', 'client_probe', '--',
                        'anvil_name_fixture', str(inputs)], cwd=lab.ROOT, stdout=out, check=True)
    matched = base.compare(build / 'vanilla.txt', build / 'rust.txt')
    print(f'Anvil name parity passed: {matched} names and units give vanilla\'s filter and blank results')


if __name__ == '__main__':
    main()
