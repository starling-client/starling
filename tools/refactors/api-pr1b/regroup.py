#!/usr/bin/env python3
"""Regroup host declarations, game forwarders and callers using an areas table.

The table (old area, method, new area; tab separated) is not kept in the tree, so no
host-method PR ever edits a shared list: new host methods are written straight into
their area file under `host/`. A branch still on the old per-helper layout replays the
merged regroup with the table from the merge: `git show 5f67d7ad57:tools/refactors/api-pr1b/areas.tsv > /tmp/areas.tsv`,
then `regroup.py --table /tmp/areas.tsv`.

Run from any directory. Unknown methods fail before any write; pass
--assign method=area (repeatable) for a method the table does not list. The operation is idempotent, including on
branches which contain both curated files and newly added old-style area files.
Only trait scaffolding, imports, paths and double macros change; method text moves
unchanged. Rustfmt is run on the changed Rust files.
"""
import argparse
from collections import defaultdict
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/refactors/api-pr1'))
import ast

# Reuse the split's trait docs without executing add_method.py's CLI.
DESCRIPTION = ast.literal_eval(next(node.value for node in ast.parse(
    (ROOT / 'tools/refactors/api-pr1/add_method.py').read_text()).body
    if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'DESCRIPTION' for t in node.targets)))
from split_doubles import camel, split_items, depth_delta  # noqa: E402

API = ROOT / 'crates/horizon-mod-api/src/host'
GAME = ROOT / 'crates/horizon-client/src/game/host'
TIERS = ('Host', 'ServerHost', 'ClientHost', 'Backend', 'ClientBackend', 'BackendMut', 'ClientBackendMut')
MACROS = {'host': 'Host', 'server_host': 'ServerHost', 'client_host': 'ClientHost',
          'backend': 'Backend', 'backend_mut': 'BackendMut', 'client_backend_mut': 'ClientBackendMut'}
BLOCK = re.compile(r'^(?P<lead>(?:(?://[^\n]*|#\[[^\n]*\])\n)*)(?:pub trait (?P<trait>\w+)|impl (?P<impl>\w+) for Game<\'_>) \{\n(?P<body>.*?)^\}\n', re.M | re.S)
FN = re.compile(r'^\s*fn (\w+)', re.M)


def tier_of(stem, name):
    return next((tier for tier in TIERS if camel(stem) + tier == name), None)


def items(body):
    for chunk in split_items(body.splitlines(keepends=True), ''):
        text = ''.join(chunk)
        method = FN.search(text)
        if not method:
            if text.strip():
                raise ValueError(f'non-method area member: {text[:100]}')
            continue
        yield method[1], text.strip('\n') + '\n'


def read_table(path):
    table = {}
    for line in Path(path).read_text().splitlines():
        old, method, new = line.split('\t')
        key = (old, method)
        if key in table:
            raise ValueError(f'duplicate table row: {key}')
        table[key] = new
    return table


def area_sources(directory, table, assignments):
    """Return output text and (old trait, method) -> new trait, without writing."""
    grouped, imports, tails, old_names, mapping = defaultdict(dict), defaultdict(set), {}, {}, {}
    missing = []
    for path in sorted(directory.glob('*.rs')):
        if path.stem == 'mod':
            continue
        text = path.read_text()
        blocks = list(BLOCK.finditer(text))
        rest, pos = [], 0
        for block in blocks:
            old_name = block['trait'] or block['impl']
            tier = tier_of(path.stem, old_name)
            if tier is None:
                if path.stem not in ('draw', 'services'):
                    raise ValueError(f'unrecognised area trait: {path.stem}.{old_name}')
                continue  # draw contexts and service traits are left intact
            rest.append(text[pos:block.start()])
            pos = block.end()
            old_names[old_name] = (path.stem, tier)
            for method, body in items(block['body']):
                dest = table.get((path.stem, method), assignments.get(method))
                if dest is None and path.stem in ('draw', 'services'):
                    dest = path.stem
                if dest is None:
                    # Curated files are also valid input on a replay.
                    destinations = {d for (a, m), d in table.items() if m == method}
                    if len(destinations) == 1:
                        dest = next(iter(destinations))
                    elif path.stem in destinations:
                        dest = path.stem
                    else:
                        missing.append(f'{path.stem}.{method}')
                        continue
                key = (dest, tier)
                if method in grouped[key] and grouped[key][method] != body:
                    raise ValueError(f'conflicting method copies: {dest}.{tier}.{method}')
                grouped[key][method] = body
                mapping[(old_name, method)] = camel(dest) + tier
                imports[dest].update(re.findall(r'^use [^\n]+;\n', text, re.M))
        rest.append(text[pos:])
        leftover = ''.join(rest)
        # These files contain independent non-area traits; retain them verbatim.
        if path.stem in ('draw', 'services'):
            tails[path.stem] = re.sub(r'\A(?://![^\n]*\n|#![^\n]*\n|use [^\n]*\n)*', '', leftover)
    if missing:
        raise ValueError('unassigned methods (--assign name=area):\n  ' + '\n  '.join(missing))
    output = {}
    for dest in sorted({a for a, t in grouped} | tails.keys()):
        sections = []
        for tier in TIERS:
            methods = grouped.get((dest, tier))
            if not methods:
                continue
            name = camel(dest) + tier
            body = ''.join(methods.values())
            if directory == API:
                doc = DESCRIPTION[tier].format(a=dest).replace('[`ClientHost`]', '[`crate::ClientHost`]')
                lead = f'/// {doc}\n'
                if 'Backend' in tier:
                    lead += "// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait\n#[doc(hidden)]\n"
                sections.append(f'{lead}pub trait {name} {{\n{body}}}\n')
            else:
                sections.append(f"impl {name} for Game<'_> {{\n{body}}}\n")
        body = ''.join(sections) + tails.get(dest, '')
        uses = imports[dest]
        if directory == API:
            head = f'//! The `{dest}` area: host methods the game implements (`game/host/{dest}.rs`).\n#![allow(unused_variables)]\n'
            uses.add('use crate::*;\n')
        else:
            head = f"//! The game's side of the `{dest}` area (`horizon-mod-api/src/host/{dest}.rs`).\n"
            uses.add('use crate::game::Game;\n')
            if dest in tails:
                uses.add('use mod_api::{prelude::*, *};\n')
            if 'use mod_api::{prelude::*, *};\n' in uses:
                uses.discard('use mod_api::prelude::*;\n')
        for imported in ('PathBuf', 'Arc'):
            if not re.search(rf'\b{imported}\b', body):
                uses = {line for line in uses if not re.search(rf'\b{imported}\b', line)}
        output[directory / f'{dest}.rs'] = head + ''.join(sorted(uses)) + body
    return output, old_names, mapping


def scope_keys(text):
    """Stable lexical scope IDs, so identically named local doubles never merge."""
    stack, scopes = [], {}
    for number, line in enumerate(text.splitlines(keepends=True)):
        scopes[number] = tuple(stack)
        delta = depth_delta(line)
        if delta > 0:
            stack.extend([number] * delta)
        elif delta < 0:
            del stack[max(0, len(stack) + delta):]
    return scopes


def rewrite_references(text, old_names, mapping):
    """Split/merge explicit area impls, then rebuild each double's skip list."""
    scopes = scope_keys(text)
    pattern = re.compile(r'^(?P<attrs>(?:[ \t]*#\[[^\n]*\]\n)*)(?P<indent>[ \t]*)impl (?P<prefix>(?:[\w]+::)*)(?P<name>\w+) for (?P<ty>[^\n{]+) \{(?P<empty>\})?\n', re.M)
    replacements, groups = [], {}
    for match in pattern.finditer(text):
        if match['name'] not in old_names:
            continue
        indent = match['indent']
        close = None if match['empty'] else re.compile(rf'^{indent}\}}\n', re.M).search(text, match.end())
        if not match['empty'] and not close:
            raise ValueError(f'unclosed impl {match[0]}')
        end = match.end() if match['empty'] else close.end()
        body = '' if match['empty'] else text[match.end():close.start()]
        scope = scopes[text.count('\n', 0, match.start())]
        for method, chunk in items(body):
            new = mapping[(match['name'], method)]
            key = (scope, match['attrs'], indent, match['ty'], new)
            if key not in groups:
                groups[key] = [match.start(), match['prefix'], []]
            groups[key][2].append(chunk)
        replacements.append((match.start(), end, ''))
    insertions = defaultdict(str)
    overrides = defaultdict(set)
    for (scope, attrs, indent, ty, name), (start, prefix, chunks) in groups.items():
        insertions[start] += f'{attrs}{indent}impl {prefix}{name} for {ty} {{\n' + ''.join(chunks) + f'{indent}}}\n'
        # Read the destination stem from the mapping; no CamelCase reverse guess.
        old_method = next(k for k, value in mapping.items() if value == name)
        old_tier = old_names[old_method[0]][1]
        stem = name[:-len(old_tier)]
        tier = 'Backend' if old_tier == 'ClientBackend' else old_tier
        overrides[(scope, ty, tier)].add(stem)
    # The Backend macro skips an entire area, including both Backend and
    # ClientBackend. Supply a sibling default when only one tier is overridden.
    for (scope, attrs, indent, ty, name), (start, prefix, _) in groups.items():
        if name.endswith('ClientBackend'):
            sibling = name[:-len('ClientBackend')] + 'Backend'
        elif name.endswith('Backend'):
            sibling = name[:-len('Backend')] + 'ClientBackend'
        else:
            continue
        if sibling in mapping.values() and (scope, attrs, indent, ty, sibling) not in groups:
            insertions[start] += f'{attrs}{indent}impl {prefix}{sibling} for {ty} {{}}\n'
    replacements = [(a, b, insertions[a]) for a, b, _ in replacements]
    macro = re.compile(r'(?P<head>(?:mod_api::|crate::)?impl_(?P<kind>host|server_host|client_host|backend|backend_mut|client_backend_mut)_areas!\(\s*)(?P<ty>[^,;)]+)(?:,\s*except\([^)]*\))?\s*\)')
    for match in macro.finditer(text):
        scope = scopes[text.count('\n', 0, match.start())]
        names = overrides.get((scope, match['ty'].strip(), MACROS[match['kind']]), set())
        stems = sorted(re.sub(r'(?<!^)(?=[A-Z])', '_', name).lower() for name in names)
        skip = f", except({', '.join(stems)})" if stems else ''
        replacements.append((match.start(), match.end(), match['head'] + match['ty'].strip() + skip + ')'))
    for start, end, replacement in sorted(replacements, reverse=True):
        text = text[:start] + replacement + text[end:]
    # Qualified calls select by method, since one old trait can split across areas.
    names = '|'.join(sorted(old_names, key=len, reverse=True))
    text = re.sub(rf'\b({names})::(\w+)', lambda m: mapping.get((m[1], m[2]), m[1]) + '::' + m[2], text)
    # A use of an old trait may now name several destination traits.
    targets = defaultdict(set)
    for (old, method), new in mapping.items():
        targets[old].add(new)
    def imports(match):
        line = match[0]
        for old, news in targets.items():
            line = re.sub(rf'\b{old}\b', '{' + ', '.join(sorted(news)) + '}' if len(news) > 1 else next(iter(news)), line)
        return line
    text = re.sub(r'^(?:[ \t]*|/// )use [^;]+;', imports, text, flags=re.M)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--table', required=True, metavar='AREAS.TSV')
    parser.add_argument('--assign', action='append', default=[], metavar='NAME=AREA')
    args = parser.parse_args()
    assignments = dict(value.split('=', 1) for value in args.assign)
    table = read_table(args.table)
    output, old_names, mapping = area_sources(API, table, assignments)
    forwards, _, _ = area_sources(GAME, table, assignments)
    output.update(forwards)
    for name in subprocess.check_output(['git', 'ls-files', '*.rs', 'docs/modding/*.md'], cwd=ROOT, text=True).splitlines():
        path = ROOT / name
        if path.parent in (API, GAME) and path.stem != 'mod':
            continue
        text = path.read_text()
        new = rewrite_references(text, old_names, mapping) if path.suffix == '.rs' else text
        # File-path references use all destinations, not an arbitrary sibling.
        for old in {stem for stem, _ in old_names.values()}:
            dests = sorted({d for (a, m), d in table.items() if a == old})
            if dests and dests != [old]:
                new = new.replace(f'host/{old}.rs', ', '.join(f'host/{d}.rs' for d in dests))
        # The curated table moves every base method; no empty base module remains.
        if name == 'crates/horizon-mod-api-gen/src/host.rs':
            new = new.replace('area.stem == "base"', 'area.stem == "config"')
        if new != text:
            output[path] = new
    changed = []
    for path, text in output.items():
        if not path.exists() or path.read_text() != text:
            path.write_text(text)
            changed.append(path)
    for directory in (API, GAME):
        for path in directory.glob('*.rs'):
            if path.stem != 'mod' and path not in output:
                path.unlink()
    if changed:
        subprocess.run(['rustfmt', '--edition', '2024', '--config', 'skip_children=true', *map(str, (p for p in changed if p.suffix == '.rs'))], check=True)
    print(f'regrouped {len(old_names)} traits into {len(forwards)} game areas; changed {len(changed)} files')


if __name__ == '__main__':
    main()
