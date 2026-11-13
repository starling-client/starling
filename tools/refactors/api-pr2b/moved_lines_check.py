#!/usr/bin/env python3
"""Check the multiset of moved lines, after declared module-layout edits.

Usage: moved_lines_check.py BASE
Normalize paths/visibility before prettyplease formatting, so a longer absolute
path cannot create an apparent changed line just by wrapping. Ignore imports,
module/trait/impl envelopes and repeated envelope docs; preserve every leaf
item's attributes, signature and body. compare_items.py separately checks whole
items and reports their destination. Comments/docs are not behavior assertions.
"""
from collections import Counter
from pathlib import Path
import re
import subprocess
import sys
from compare_items import ROOT, API
from move import layout


def lines(source):
    # Declared edits: paths, sibling visibility, one disambiguated local name,
    # imports and outer trait/impl/module envelopes, rustfmt whitespace.
    source = re.sub(r'(?m)^\s*//[^\n]*', '', source)
    source = re.sub(r'\bpub(?:\([^)]*\))?\s+', '', source)
    source = re.sub(r'\b(?:crate|super)::', '', source)
    source = re.sub(r'\b(?:parts|shared|capabilities|items|tests)::', '', source)
    source = source.replace('installed_parts', 'installed')
    source = subprocess.check_output([str(ROOT / 'target/debug/part-table-layout'), '--canonical'], input=source, text=True)
    source_lines = source.splitlines()
    result = Counter()
    def walk(entries):
        for item in entries:
            start, end = item['start']-1, item['end']
            text = '\n'.join(source_lines[start:end])
            if 'children' in item:
                walk(item['children'])
            elif 'members' in item:
                for member in item['members']:
                    result.update(line.strip() for line in source_lines[member['start']-1:member['end']] if line.strip())
            elif not any(item.get(key) for key in ('import', 'module_declaration', 'generated_modules')):
                result.update(line.strip() for line in source_lines[start:end] if line.strip())
    walk(layout(source))
    return result


def main():
    base = sys.argv[1]
    old = Counter()
    names = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', base, 'crates/horizon-mod-api/src/part_table/'], text=True).splitlines()
    for name in names:
        old.update(lines(subprocess.check_output(['git', 'show', f'{base}:{name}'], text=True)))
    new = Counter()
    for path in (API / 'parts').glob('*.rs'):
        new.update(lines(path.read_text()))
    if old != new:
        for label, delta in [('removed', old-new), ('added', new-old)]:
            for line, n in delta.items():
                print(label, n, line)
        raise SystemExit(1)
    print(f'PASS: multiset of {sum(old.values())} canonical moved lines is equal (14 fragments)')


if __name__ == '__main__':
    main()
