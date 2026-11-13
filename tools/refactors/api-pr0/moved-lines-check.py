#!/usr/bin/env python3
"""Moved-lines check for PR 0.

Compares, over every changed .rs file, the multiset of lines of the old tree against the new
tree after dropping `use` statements (whole, even when rustfmt wraps them) and comments-only
whitespace. Lines that only differ by the declared path rewrites are normalised first:
`horizon_world::`/`crate::`/`super::` prefixes of the moved items become `horizon_core::`.
What is left over is printed: it must be the script's declared edits and nothing else.
"""
import collections, re, subprocess, sys

base = sys.argv[1] if len(sys.argv) > 1 else 'origin/main'
git = lambda *a: subprocess.run(['git', *a], capture_output=True, text=True).stdout
files = git('diff', '--cached', '--name-only', '-M', '--no-renames', base).split()
USE = re.compile(r'^[ \t]*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([a-z]+\))? )?use [^;]*;', re.M | re.S)
MOD = re.compile(r'^[ \t]*(?:pub(?:\([a-z]+\))? )?mod \w+;\n', re.M)
PATHS = re.compile(r'\b(?:horizon_world|crate|super|self)::(?:\w+::)*?(nbt|identifier|java_hash_map|dimension|equipment_slot|use_animation|connection_kind|neighbour_update|bounding_box)\b')


def lines(text):
    text = USE.sub('', text)
    text = MOD.sub('', text)
    text = re.sub(r'\b(?:horizon_world|horizon_core)::', 'ROOT::', text)
    text = re.sub(r'\b(?:crate|super)::(?=(?:nbt|identifier|java_hash_map|level::(?:Dimension|ConnectionKind|NeighbourUpdate)|worldgen::structure::BoundingBox|player_inventory::EquipmentSlot|consumable::UseAnimation))', 'ROOT::', text)
    text = re.sub(r'ROOT::(?:level|worldgen::structure|player_inventory|consumable)::', 'ROOT::', text)
    text = re.sub(r'ROOT::(?:dimension|equipment_slot|use_animation|connection_kind|neighbour_update|bounding_box)::', 'ROOT::', text)
    text = re.sub(r'\b(?:super|self)::(?=(?:Dimension|ConnectionKind|NeighbourUpdate|BoundingBox)\b)', 'ROOT::', text)
    return [re.sub(r'\s+', '', l) for l in text.split('\n') if l.strip()]


rem, add = collections.Counter(), collections.Counter()
for f in files:
    if not f.endswith('.rs'):
        continue
    old = git('show', f'{base}:{f}')
    new = git('show', f':{f}')
    rem.update(lines(old))
    add.update(lines(new))
# rustfmt may split one statement over lines: compare on a per-character basis too
only_rem, only_add = rem - add, add - rem
print('removed-only:', sum(only_rem.values()), ' added-only:', sum(only_add.values()))
for k, v in list(only_rem.items())[:60]:
    print('  -', v, k[:110])
for k, v in list(only_add.items())[:60]:
    print('  +', v, k[:110])
