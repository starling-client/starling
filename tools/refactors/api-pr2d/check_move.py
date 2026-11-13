#!/usr/bin/env python3
"""Check moved methods token-for-token and the aggregate removed/added line multiset.

Declared structural edits: trait/impl wrapper lines and braces, the new area-trait
headings, doc(hidden) and its reason on provider area traits. Imports are excluded, not methods.
Union documentation moves to unions.rs.in unchanged. No method is excluded.
"""
import collections
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
PREFIX = 'crates/horizon-mod-api/src/parts/'
BASE = sys.argv[1]
LAYOUT = ROOT / 'target/debug/part-table-layout'

def methods(source):
    lines = source.splitlines(keepends=True)
    out = collections.Counter()
    layout = json.loads(subprocess.check_output([LAYOUT], input=source, text=True))
    for item in layout:
        text = ''.join(lines[item['start']-1:item['end']])
        trait = re.search(r'pub trait \w*(Registrar|CapabilityProviders) \{', text)
        impl = re.search(r'impl \w*(Registrar|CapabilityProviders) for (PartRegistrar|InstalledProviders) \{', text)
        if trait or impl:
            owner = (trait or impl)[1]
            for member in item['members']:
                out[(bool(impl), owner, member['text'])] += 1
    return out

def content(path):
    return subprocess.check_output(['git', 'show', f'{BASE}:{path}'], cwd=ROOT, text=True)

paths = subprocess.check_output(['git','ls-tree','-r','--name-only',BASE,'--',PREFIX], cwd=ROOT, text=True).splitlines()
before = {p: content(p) for p in paths if p.endswith('.rs')}
after = {str(p.relative_to(ROOT)): p.read_text() for p in (ROOT / PREFIX).glob('*.rs')}
old_methods = methods(before[PREFIX+'mod.rs'])
new_methods = sum((methods(s) for s in after.values()), collections.Counter())
assert old_methods == new_methods, (old_methods-new_methods, new_methods-old_methods)

def lines(sources):
    result = collections.Counter()
    for source in sources:
        for line in source.splitlines():
            line = line.strip()
            if (not line or line == '}' or line == '#[doc(hidden)]'
                or line.startswith(('use ', 'pub trait ', 'impl ', '/// Registrar operations for ', '/// CapabilityProviders operations for ', '// hidden: per-kind lookup implementation,'))):
                continue
            result[line] += 1
    return result

old_lines = lines(before.values())
new_lines = lines([*after.values(), (ROOT / PREFIX / 'unions.rs.in').read_text()])
assert old_lines == new_lines, (old_lines-new_lines, new_lines-old_lines)
print(f'Moved {sum(old_methods.values())} declarations/implementations token-identically; aggregate line multiset identical across {len(after)} Rust files plus union documentation template.')
print('Declared exclusions: imports, trait/impl wrapper lines and closing braces, area-trait headings, doc(hidden) attributes and their per-kind reason.')
