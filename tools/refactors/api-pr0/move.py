#!/usr/bin/env python3
"""PR 0: move the plain vocabulary types into horizon-core and rewrite every path.

Run from the repository root. Step 1 moves whole modules (`git mv`) and cuts single
types out of their files verbatim; step 2 rewrites `use` trees and inline paths.
Every moved line is checked afterwards by `moved-lines-check.py`.
"""
import re, subprocess, sys, pathlib

ROOT = pathlib.Path('.')
W = ROOT / 'crates/horizon-world/src'
C = ROOT / 'crates/horizon-core/src'

# (module path inside horizon-world) -> horizon-core module path. Whole modules.
MODULES = {'nbt': 'nbt', 'identifier': 'identifier', 'java_hash_map': 'java_hash_map'}

# single types: (old file, type name, new core module, old path prefix, doc/attr header start marker)
TYPES = [
    ('consumable.rs', 'UseAnimation', 'use_animation', 'consumable'),
    ('player_inventory.rs', 'EquipmentSlot', 'equipment_slot', 'player_inventory'),
    ('level/mod.rs', 'Dimension', 'dimension', 'level'),
    ('level/mod.rs', 'ConnectionKind', 'connection_kind', 'level'),
    ('level/mod.rs', 'NeighbourUpdate', 'neighbour_update', 'level'),
    ('worldgen/structure.rs', 'BoundingBox', 'bounding_box', 'worldgen::structure'),
]


def run(*a):
    subprocess.run(a, check=True)


def cut_type(path, name):
    """Cut `name`'s definition (with its doc lines and attributes) and its inherent impl."""
    text = path.read_text()
    lines = text.split('\n')
    out_blocks = []

    def find(pattern):
        for i, l in enumerate(lines):
            if re.match(pattern, l):
                return i
        return None

    for pattern in (rf'pub (enum|struct) {name}\b', rf'impl {name} \{{'):
        i = find(pattern)
        if i is None:
            continue
        start = i
        while start > 0 and re.match(r'(///|#\[|//!)', lines[start - 1]):
            start -= 1
        end = i
        if lines[i].rstrip().endswith('}'):
            pass
        else:
            while lines[end] != '}':
                end += 1
        out_blocks.append('\n'.join(lines[start:end + 1]))
        del lines[start:end + 1]
    path.write_text('\n'.join(lines))
    return '\n'.join(out_blocks) + '\n'


def step1():
    for mod in MODULES:
        run('git', 'mv', str(W / f'{mod}.rs'), str(C / f'{mod}.rs'))
    for file, name, newmod, _ in TYPES:
        body = cut_type(W / file, name)
        header = ''
        if 'Direction' in body:
            header = 'use crate::geometry::Direction;\n\n'
        (C / f'{newmod}.rs').write_text(header + body)
    lib = (C / 'lib.rs').read_text()
    new = sorted(list(MODULES.values()) + [t[2] for t in TYPES])
    for m in new:
        lib = lib.replace('pub mod arm_pose;', f'pub mod arm_pose;\npub mod {m};', 1) if False else lib
    # insert in alphabetical position among the `pub mod` lines
    mods = re.findall(r'^pub mod (\w+);$', lib, re.M)
    allm = sorted(set(mods + new))
    first = lib.index('pub mod ')
    last = lib.rindex('pub mod ')
    last = lib.index('\n', last) + 1
    lib = lib[:first] + ''.join(f'pub mod {m};\n' for m in allm) + lib[last:]
    (C / 'lib.rs').write_text(lib)
    wl = (W / 'lib.rs').read_text()
    for m in MODULES:
        wl = re.sub(rf'^pub mod {m};\n', '', wl, flags=re.M)
    (W / 'lib.rs').write_text(wl)


# ---- step 2: rewriting -------------------------------------------------------------------

# old path (relative to crate root horizon_world) -> new path (relative to horizon_core)
MOVED = {}
for m in MODULES:
    MOVED[(m,)] = (m,)
for _, name, newmod, old in TYPES:
    MOVED[tuple(old.split('::')) + (name,)] = (newmod, name)


def parse_tree(s):
    """Parse the inside of a use item into leaves: list of (segments, alias-or-None)."""
    pos = 0

    def parse(prefix):
        nonlocal pos
        leaves = []
        while pos < len(s):
            while pos < len(s) and s[pos] in ' \n\t,':
                pos += 1
            if pos >= len(s) or s[pos] == '}':
                break
            if s[pos] == '{':
                pos += 1
                leaves += parse(prefix)
                pos += 1
                continue
            m = re.match(r'(\*|[A-Za-z_][A-Za-z0-9_]*)', s[pos:])
            seg = m.group(1)
            pos += len(seg)
            seg_path = prefix + [seg]
            rest = s[pos:]
            if rest.startswith('::'):
                pos += 2
                if s[pos] == '{':
                    pos += 1
                    leaves += parse(seg_path)
                    pos += 1
                else:
                    sub = parse_one(seg_path)
                    leaves += sub
            else:
                m2 = re.match(r'\s+as\s+(\w+)', rest)
                alias = None
                if m2:
                    alias = m2.group(1)
                    pos += len(m2.group(0))
                leaves.append((seg_path, alias))
        return leaves

    def parse_one(prefix):
        nonlocal pos
        out = []
        while True:
            m = re.match(r'(\*|[A-Za-z_][A-Za-z0-9_]*)', s[pos:])
            seg = m.group(1)
            pos += len(seg)
            prefix = prefix + [seg]
            if s[pos:].startswith('::'):
                pos += 2
                if s[pos] == '{':
                    pos += 1
                    out += parse(prefix)
                    pos += 1
                    return out
                continue
            m2 = re.match(r'\s+as\s+(\w+)', s[pos:])
            alias = None
            if m2:
                alias = m2.group(1)
                pos += len(m2.group(0))
            return [(prefix, alias)]

    return parse([])


def build_tree(leaves):
    root = {}
    for segs, alias in leaves:
        node = root
        for seg in segs[:-1]:
            node = node.setdefault(seg, {})
        key = segs[-1] + (f' as {alias}' if alias else '')
        node.setdefault('\0' + key, None)
    return root


def render(node):
    parts = []
    for k in sorted(node, key=lambda k: (k.lstrip('\0') != 'self', k.lstrip('\0'))):
        v = node[k]
        if k.startswith('\0'):
            parts.append(k[1:])
        else:
            inner = render(v)
            parts.append(f'{k}::{inner}')
    if len(parts) == 1:
        return parts[0]
    return '{' + ', '.join(parts) + '}'


def map_leaf(segs, root_names, base=None):
    """If `segs` (starting with a crate-root alias in root_names) names a moved item, return new segs."""
    if base is not None and segs[0] in ('super', 'self'):
        for cand in absolute_candidates(segs, base):
            for n in range(len(cand), 0, -1):
                if tuple(cand[:n]) in MOVED:
                    return ['horizon_core'] + list(MOVED[tuple(cand[:n])]) + cand[n:]
        return None
    if segs[0] not in root_names:
        return None
    rest = segs[1:]
    for n in range(len(rest), 0, -1):
        if tuple(rest[:n]) in MOVED:
            new = MOVED[tuple(rest[:n])]
            return ['horizon_core'] + list(new) + rest[n:]
    return None


USE_RE = re.compile(r'(?P<head>^[ \t]*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([a-z]+\))? )?use )(?P<body>[^;]*?);', re.M | re.S)


def relative_prefixes(f):
    """The absolute module path (inside horizon-world) of file f, and its inline-module variant."""
    rel = f[len('crates/horizon-world/src/'):-3].split('/')
    if rel[-1] == 'mod':
        rel = rel[:-1]
    if rel == ['lib']:
        rel = []
    return rel


def absolute_candidates(segs, base):
    """For a path starting with `super`/`self`, the absolute module paths it can mean: from the
    file's own module, or from an inline module (`mod tests {`) one level deeper."""
    k = 0
    while k < len(segs) and segs[k] == 'super':
        k += 1
    if k == 0 and segs[0] != 'self':
        return []
    rest = segs[k:] if k else segs[1:]
    out = []
    for deeper in (0, 1):
        b = base + (['_inline'] * deeper)
        if k > len(b):
            continue
        out.append(b[:len(b) - k] + rest if k else b + rest)
    return out


def rewrite_text(text, root_names, base=None):
    def repl(m):
        body = m.group('body')
        stripped = body.strip()
        # whole-tree: first segment(s) decide; parse and regroup moved leaves
        try:
            leaves = parse_tree(stripped)
        except Exception:
            return m.group(0)
        changed = False
        out = []
        for segs, alias in leaves:
            new = map_leaf(segs, root_names, base)
            if new is not None:
                changed = True
                out.append((new, alias))
            else:
                out.append((segs, alias))
        if not changed:
            return m.group(0)
        # `use horizon_world::nbt::{self}` style leaves: 'self' leaf -> drop self
        fixed = []
        for segs, alias in out:
            if segs[-1] == 'self' and len(segs) > 1:
                fixed.append((segs[:-1], alias))
            else:
                fixed.append((segs, alias))
        tree = build_tree(fixed)
        return m.group('head') + render(tree) + ';'

    text = USE_RE.sub(repl, text)
    # inline paths
    for root in root_names:
        for old, new in sorted(MOVED.items(), key=lambda kv: -len(kv[0])):
            pat = r'\b' + re.escape(root) + '::' + '::'.join(old) + r'\b'
            text = re.sub(pat, 'horizon_core::' + '::'.join(new), text)
    if base is not None:
        def inline(m):
            segs = m.group(0).split('::')
            new = map_leaf(segs, root_names, base)
            return '::'.join(new) if new else m.group(0)
        text = re.sub(r'\b(?:(?:super|self)::)+(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*', inline, text)
    return text


def step2():
    files = subprocess.run(['git', 'ls-files', '*.rs'], capture_output=True, text=True, check=True).stdout.split()
    changed = []
    for f in files:
        p = ROOT / f
        text = p.read_text()
        if f.startswith('crates/horizon-world/src/') and not f.startswith('crates/horizon-world/src/lib.rs'):
            roots = {'crate', 'horizon_world'}
        else:
            roots = {'horizon_world'}
        base = relative_prefixes(f) if f.startswith('crates/horizon-world/src/') else None
        new = rewrite_text(text, roots, base)
        if new != text:
            p.write_text(new)
            changed.append(f)
    pathlib.Path('/box/shared/api-lead/pr0/changed-files.txt').write_text('\n'.join(changed))
    print(len(changed), 'files rewritten')


if __name__ == '__main__':
    {'1': step1, '2': step2}[sys.argv[1]]()
