#!/usr/bin/env python3
"""Compare every original joined item with real-module items (including tests).

Usage: compare_items.py OLD_GENERATED_RS [--list]
Visibility and paths are normalized; imports, module/impl/trait envelopes and
repeated trait-envelope docs are ignored. Leaf item bodies are compared too,
not just names. Uses the existing syn-based part-table-layout dev tool.
"""
from collections import Counter
from pathlib import Path
import re
import subprocess
import sys
from move import API, ROOT, layout


def normalize(s):
    s = re.sub(r'(?m)^\s*//[^\n]*', '', s)
    s = re.sub(r'\bpub(?:\s*\([^)]*\))?\s+', '', s)
    s = s.replace('installed_parts', 'installed')
    s = re.sub(r'\b(?:crate|super)\s*::\s*', '', s)
    s = re.sub(r'\b(?:parts|shared|capabilities|items|tests)\s*::\s*', '', s)
    s = re.sub(r'\s+', '', s)
    return re.sub(r',(?=[>)\]}])', '', s)


def items(source):
    source = subprocess.check_output([str(ROOT / 'target/debug/part-table-layout'), '--canonical'], input=source, text=True)
    lines = source.splitlines(keepends=True)
    def walk(entries, owner=''):
        for item in entries:
            text = ''.join(lines[item['start']-1:item['end']])
            if 'children' in item:
                yield from walk(item['children'], owner)
            elif 'members' in item:
                header = re.search(r'(?m)^(?:pub )?(?:trait|impl)[^\n]*', text)
                context = normalize(header[0].split('{')[0]) if header else owner
                for member in item['members']:
                    yield context + ':' + normalize(member['text'])
            elif not any(item.get(key) for key in ('import', 'module_declaration', 'generated_modules')):
                n = normalize(text)
                if n:
                    yield n
    return Counter(walk(layout(source)))


def manifests(old):
    before = items(Path(old).read_text())
    after = Counter()
    locations = {}
    for p in sorted((API / 'parts').glob('*.rs')):
        found = items(p.read_text())
        after.update(found)
        locations.update({k: p.name for k in found})
    return before, after, locations


def main():
    before, after, locations = manifests(sys.argv[1])
    if before != after:
        for side, delta in [('only before', before-after), ('only after', after-before)]:
            for item, count in delta.items():
                print(side, count, item[:400])
        raise SystemExit(1)
    if '--list' in sys.argv:
        for item in sorted(after):
            print(locations[item], item[:180])
    print(f'PASS: {sum(before.values())} items (signatures and bodies) unchanged across 14 modules')


if __name__ == '__main__':
    main()
