#!/usr/bin/env python3
"""Prove the regroup is a pure move, against BASE (default: the recorded base).

Compare full source line multisets, not git's rename heuristics. Declared edits:
area trait/impl wrappers and their docs, module headers, use declarations, blank
lines, standalone block delimiters, hidden trait markers, repeated cfg(test) wrapper attributes and double-area macros.
Area trait names in qualified calls/imports normalize to their tier. The generator
checkout smoke test names config instead of the now-empty base area. Rustfmt
reflow caused by shorter qualified trait names is normalized with rustfmt itself.

Independently compare every complete method (signature, docs and body bytes) in
both host folders. This covers delimiters and docs ignored as wrapper scaffolding
by the line comparison. draw/services non-area traits are included in this check.
The generator's host.rs is excluded from line counting: its separate small
trait-validation follow-up is hand-written and reviewed, not a pure move. Its area
smoke test is the only scripted edit there. Phase example imports are declared
import edits. All declaration and game-forwarder methods remain covered.
"""
from collections import Counter
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
BASE = 'ba916ffc30d3b67eafb7124ebad78308d3f6bb91'
FOLDERS = ('crates/horizon-mod-api/src/host/', 'crates/horizon-client/src/game/host/')
TIERS = 'ClientBackendMut|ClientBackend|BackendMut|Backend|ServerHost|ClientHost|Host'


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True)


def files(ref):
    return {p: git('show', f'{ref}:{p}') for p in git('ls-tree', '-r', '--name-only', ref).splitlines()
            if p.endswith('.rs') and (p.startswith(FOLDERS) or p in changed)}


def lines(sources):
    result = Counter()
    for path, text in sources.items():
        if path == 'crates/horizon-mod-api-gen/src/host.rs':
            continue
        text = re.sub(rf'\b\w+?({TIERS})\b', r'Area\1', text)
        if not path.startswith(FOLDERS):
            text = subprocess.run(['rustfmt', '--edition', '2024', '--config', 'skip_children=true'],
                                  input=text, capture_output=True, text=True, check=True).stdout
        text = re.sub(r'(?:mod_api::|crate::)?impl_(?:host|server_host|client_host|backend|backend_mut|client_backend_mut)_areas!\([^;]*\);', '', text)
        text = re.sub(r'(?m)^\s*use [^;]+;\n', '', text)
        for line in text.splitlines():
            stripped = line.strip()
            if not stripped or stripped == '}' or stripped.startswith(('//!', '/// use ', '/// The host\'s methods ',
                    '/// The server host\'s methods ', '/// The client host\'s methods ',
                    '/// The handles\'', '// hidden: a handle\'s backend', '#![allow(unused_variables)]')):
                continue
            if re.match(r'(?:pub trait \w+ \{|impl (?:\w+::)*\w+ for [^{]+ \{\}?)$', stripped):
                continue
            if stripped in ('#[doc(hidden)]', '#[cfg(test)]'):
                continue
            line = re.sub(rf'\b\w+?({TIERS})\b', r'Area\1', line)
            if path == 'crates/horizon-mod-api-gen/src/host.rs':
                line = line.replace('area.stem == "base"', 'area.stem == "config"')
            result[line] += 1
    return result


def methods(sources):
    result = Counter()
    # The source's rustfmt convention puts method docs/signatures at four spaces
    # and its closing brace at four spaces; nested bodies are more indented.
    pattern = re.compile(r'(?:^    (?://[^\n]*|#\[[^\n]*\])\n)*^    fn \w+\b.*?^    \}\n', re.M | re.S)
    for path, text in sources.items():
        if path.startswith(FOLDERS) and not path.endswith('/mod.rs'):
            for match in pattern.finditer(text):
                result[(path.startswith(FOLDERS[1]), match[0])] += 1
    return result


base = sys.argv[1] if len(sys.argv) > 1 else BASE
head = sys.argv[2] if len(sys.argv) > 2 else None
changed = {p for p in git('diff', '--name-only', base, *([head] if head else [])).splitlines()
           if p.endswith('.rs')}
old = files(base)
new = files(head) if head else {p: (ROOT / p).read_text() for p in set(old) | changed if (ROOT / p).exists()}
# Untracked destination files matter before the regroup commit is staged, too.
if head is None:
    for folder in FOLDERS:
        for path in (ROOT / folder).glob('*.rs'):
            new[str(path.relative_to(ROOT))] = path.read_text()
for label, before, after in [('method bytes', methods(old), methods(new)), ('source lines', lines(old), lines(new))]:
    removed, added = before - after, after - before
    if removed or added:
        for sign, entries in [('-', removed), ('+', added)]:
            for value, count in list(entries.items())[:12]:
                print(f'{sign} {count} {value!r}')
        sys.exit(f'{label} differ: {sum(removed.values())} removed, {sum(added.values())} added')
    print(f'{label}: identical multiset ({sum(before.values())} entries)')
print('moved-lines check passed (declared scaffolding edits only)')
