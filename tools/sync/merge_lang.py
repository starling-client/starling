#!/usr/bin/env python3
"""Git JSON translation merge driver. Conflicts leave ours valid and report keys."""
import json
import sys
from pathlib import Path

MISSING = object()


def merge(base, ours, theirs):
    result, conflicts = {}, []
    for key in sorted(base.keys() | ours.keys() | theirs.keys()):
        b, o, t = (obj.get(key, MISSING) for obj in (base, ours, theirs))
        if o is MISSING and b is not MISSING:
            # A fork removal is intentional even when upstream retranslates it.
            value = MISSING
        elif o == t or t == b:
            value = o
        elif o == b:
            value = t
        else:
            conflicts.append(key)
            value = o
        if value is not MISSING:
            result[key] = value
    return result, conflicts


def main():
    base, ours, theirs = map(Path, sys.argv[1:4])
    result, conflicts = merge(*(json.loads(p.read_text()) for p in (base, ours, theirs)))
    ours.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    if conflicts:
        print(f'{sys.argv[-1]}: conflicting translation keys: {conflicts}', file=sys.stderr)
    return bool(conflicts)


if __name__ == '__main__':
    sys.exit(main())
