#!/usr/bin/env python3
"""Audit the forwarder rename before changing any source.

Refuses existing Game methods, including trait impl methods: adding an inherent
method could change method lookup at callers. No rewrite is allowed on failure.
"""
import collections
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GAME = ROOT / "crates/horizon-client/src"


def mask(source):
    # Preserve offsets and braces in code, but not comments or string literals.
    pattern = r'''//[^\n]*|/\*[\s\S]*?\*/|r(\#*)"[\s\S]*?"\1|'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^'\\])'|"(?:\\.|[^"\\])*"'''
    return re.sub(pattern, lambda m: " " * len(m[0]), source)


def blocks(source, pattern):
    code = mask(source)
    for match in re.finditer(pattern, code):
        start = match.end()
        depth = 1
        end = start
        while depth:
            if code[end] == "{":
                depth += 1
            elif code[end] == "}":
                depth -= 1
            end += 1
        yield match, source[start:end - 1], start


def audit():
    targets = {}
    classes = collections.Counter()
    base = sys.argv[sys.argv.index('--base') + 1] if '--base' in sys.argv else None
    if base:
        names = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', base,
                                         'crates/horizon-client/src/game/host'], cwd=ROOT, text=True).splitlines()
        sources = [subprocess.check_output(['git', 'show', base + ':' + name], cwd=ROOT, text=True) for name in names]
    else:
        sources = [path.read_text() for path in sorted((GAME / 'game/host').glob('*.rs'))]
    for source in sources:
        for impl, body, _ in blocks(source, r"impl (\w+) for Game<'_>\s*\{"):
            if not re.search(r'(?:Host|Backend|BackendMut)$', impl[1]):
                continue  # Draw contexts and service implementations are not area traits.
            for method, body, _ in blocks(body, r'\bfn\s+(\w+)\s*\([^{}]*?\)[^{}]*\{'):
                call = re.search(r'(self\.|Self::)(native_\w+)\(([^;]*)\)\s*$', body)
                if not call:
                    classes["union_accessors" if impl[1] in {'Host', 'ServerHost', 'ClientHost'} else "service_accessors"] += 1
                    continue
                name = method[1]
                target = call[2]
                assert target not in targets, target
                targets[target] = name
                classes["static" if call[1] == "Self::" else "mechanical" if target == "native_" + name else "odd_named"] += 1
    definitions = collections.defaultdict(list)
    parsed = subprocess.check_output(
        ["cargo", "run", "--quiet", "-p", "horizon-mod-api-gen", "--example", "audit_game_methods"],
        cwd=ROOT, text=True,
    )
    for row in parsed.splitlines():
        name, path, line, kind = row.split("\t")
        if "host" in Path(path).parts:
            continue  # The replaced forwarders themselves are not clashes.
        definitions[name].append((path, int(line), kind))
    print("Base:", subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip())
    print("Forwarder classes:", dict(classes))
    clashes = []
    pending = {}
    for old, new in sorted(targets.items()):
        inherent = [d for d in definitions[new] if d[2] == 'inherent']
        if not definitions[old] and len(inherent) == 1:
            continue  # This target was already converted before a main merge.
        pending[old] = new
        if len(definitions[old]) != 1:
            clashes.append(f"{old} -> {new}: expected one Game definition, found {definitions[old]}")
        for path, line, header in definitions[new]:
            if new == 'attachments' and header == 'trait':
                continue  # Public AttachmentHost method stays; inspect its calls separately.
            clashes.append(f"{old} -> {new}: {path}:{line}: {header}")
    if clashes:
        print("REFUSED: rename collisions / unresolved definitions:")
        print("\n".join(clashes))
        return 1
    print("No rename clashes (public AttachmentHost method retained).")
    if '--apply' in sys.argv:
        table = Path(__file__).with_name('forwarder-renames.tsv')
        recorded = dict(row.split('\t') for row in table.read_text().splitlines()) if table.exists() else {}
        for old, new in targets.items():
            assert old not in recorded or recorded[old] == new, (old, new)
            recorded[old] = new
        table.write_text(''.join(f'{old}\t{new}\n' for old, new in sorted(recorded.items())))
        pattern = re.compile(r'\b(' + '|'.join(map(re.escape, pending)) + r')\b') if pending else None
        for path in ROOT.rglob('*.rs'):
            if any(part in {'target', '.git', '.local'} for part in path.relative_to(ROOT).parts):
                continue
            relative = path.relative_to(ROOT).as_posix()
            if relative.startswith(('crates/horizon-mod-api/src/host/', 'crates/horizon-mod-api/src/hooks/')):
                continue  # API declarations and their frozen rustdoc contain no Game implementation calls.
            source = path.read_text()
            new = pattern.sub(lambda m: pending[m[0]], source) if pattern else source
            if new != source:
                path.write_text(new)
        for path in (GAME / 'game/host').glob('*.rs'):
            if path.stem not in {'draw', 'services', 'mod'}:
                path.unlink()  # Every removed impl is now generated from the API declarations.
    return 0


if __name__ == "__main__":
    raise SystemExit(audit())
