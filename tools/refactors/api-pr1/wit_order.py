"""Canonicalize only the name-resolved sets affected by host-area regrouping.

No production ordering changes: all non-set bytes, hook interfaces and the world
remain ordered. Type blocks retain their exact internal text. This deliberately
understands only the WIT formatting emitted by the generator; anything else stays
byte-ordered and therefore cannot silently weaken the gate.
"""
from collections import Counter
import re

HOST_IMPORTS = {'host', 'server-host', 'client-host', 'backend', 'backend-mut', 'client-backend-mut'}


def canonical_wit(text):
    groups = {}

    def interface(match):
        name, body = match['name'], match['body']
        if name not in HOST_IMPORTS | {'types'}:
            return match[0]
        changes = []

        def slots(kind, spans):
            values = [body[start:end] for start, end in spans]
            groups[(name, kind)] = values
            changes.extend((start, end, value) for (start, end), value in zip(spans, sorted(values)))

        funcs = list(re.finditer(r'^  [\w%-]+: func\([^\n]*;\n', body, re.M))
        slots('functions', [m.span() for m in funcs])
        if name == 'types':
            spans, start, depth, offset = [], None, 0, 0
            for line in body.splitlines(keepends=True):
                if start is None and re.match(r'^  (?:type|record|variant|enum|flags|resource) [\w%-]+\b', line):
                    start = offset
                if start is not None:
                    depth += line.count('{') - line.count('}')
                    if depth == 0:
                        spans.append((start, offset + len(line)))
                        start = None
                offset += len(line)
            if start is not None:
                raise ValueError('unterminated WIT type definition')
            slots('types', spans)
        for index, use in enumerate(re.finditer(r'^  use types\.\{(?P<items>[^\n}]+)\};\n', body, re.M)):
            values = use['items'].split(', ')
            groups[(name, f'use-items:{index}')] = values
            changes.append((*use.span('items'), ', '.join(sorted(values))))
        for start, end, value in sorted(changes, reverse=True):
            body = body[:start] + value + body[end:]
        return match['head'] + body + match['tail']

    normalized = re.sub(r'(?P<head>^interface (?P<name>[\w-]+) \{\n)(?P<body>.*?)(?P<tail>^\}\n)', interface, text, flags=re.M | re.S)
    return normalized, groups


def compare_wit(old, new):
    before, old_groups = canonical_wit(old)
    after, new_groups = canonical_wit(new)
    moved = Counter()
    for key in old_groups.keys() | new_groups.keys():
        left, right = old_groups.get(key, []), new_groups.get(key, [])
        if Counter(left) != Counter(right):
            print(f'gate failed: WIT set differs in {key[0]} {key[1]}')
            return False
        kind = key[1].split(':')[0]
        moved[kind] += sum(a != b for a, b in zip(left, right))
    if before != after:
        print('gate failed: WIT bytes outside host functions, whole types and use-items differ')
        return False
    print(f"WIT sets identical; moved {moved['functions']} host functions, {moved['types']} type blocks, {moved['use-items']} use-items; all other bytes identical")
    return True
