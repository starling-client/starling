#!/usr/bin/env python3
"""Turn joined Parts fragments into modules. Run at the repository root.

Declared edits: consolidate the two repeated trait envelopes in parts/mod.rs;
imports; sibling visibility; absolute API paths; distributed test fixture imports.
Bodies, trait members, and impl members are otherwise unchanged. The generator
join removal is deliberately separate. Requires the part-table-layout dev tool.
"""
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
API = ROOT / 'crates/horizon-mod-api/src'


def layout(source):
    return json.loads(subprocess.check_output(
        [str(ROOT / 'target/debug/part-table-layout')], input=source, text=True))


def visibility(source):
    # Only module-level declarations and members of module-level data/impls.
    lines = source.splitlines(keepends=True)
    for item in layout(source):
        start, end = item['start'] - 1, item['end']
        for i in range(start, end):
            if re.match(r'^(?:fn|struct|enum|type|static|mod) ', lines[i]):
                lines[i] = 'pub(crate) ' + lines[i]
        header = ''.join(lines[start:end])
        if re.search(r'^(?:pub(?:\([^)]*\))? )?struct ', header, re.M):
            for i in range(start, end):
                lines[i] = re.sub(r'^(    )(\w+:)', r'\1pub(crate) \2', lines[i])
        if 'members' in item and re.search(r'^impl(?:<[^\n]+>)? (?:PartRegistrar|Attached<)', header, re.M):
            for i in range(start, end):
                lines[i] = re.sub(r'^(    )(fn )', r'\1pub(crate) \2', lines[i])
    return ''.join(lines).replace('pub(super)', 'pub(crate)')


def main():
    old = API / 'part_table'
    new = API / 'parts'
    moved = old.exists()
    if moved:
        old_root = (API / 'part_table.rs').read_text()
        imports = old_root[old_root.index('use super::ticket_controller'):old_root.index('include!')]
        imports = imports.replace('super::', 'crate::')
        new.mkdir(exist_ok=True)
        traits = {}
        envelopes = {}
        kinds = sorted(p.stem for p in old.glob('*.rs'))
        for kind in kinds:
            path = old / f'{kind}.rs'
            source = path.read_text()
            lines = source.splitlines(keepends=True)
            for item in reversed(layout(source)):
                text = ''.join(lines[item['start'] - 1:item['end']])
                match = re.search(r'^(pub trait (?:Registrar|CapabilityProviders)|impl (?:Registrar for PartRegistrar|CapabilityProviders for InstalledProviders)) \{', text, re.M)
                if match:
                    name = match[1]
                    body = text[text.index('{') + 1:text.rindex('}')]
                    traits.setdefault(name, []).append(body)
                    envelopes.setdefault(name, text[:text.index('{')])
                    del lines[item['start'] - 1:item['end']]
            source = ''.join(lines)
            source = re.sub(r'^(    )static (?=(?:REGISTERING_KEYS|TEST_PARTS|TEST_PART_LOOKUPS|GAME_PARTS|GAME_THREAD|PRE_WORLD)\b)', r'\1pub(crate) static ', source, flags=re.M)
            # Paths in test modules were one level closer to the API root.
            source = source.replace('super::super::super::', 'crate::')
            source = source.replace('super::super::', 'crate::')
            # super:: in production referred to the API, not the table.
            split = source.find('pub mod tests {')
            if split < 0:
                source = re.sub(r'super::(?!\*)', 'crate::', source)
            else:
                source = re.sub(r'super::(?!\*)', 'crate::', source[:split]) + source[split:]
                source = source.replace('pub mod tests {', 'pub mod tests {\n    use super::*;\n    use crate::parts::tests::*;\n    use crate::*;')
                # Formerly shared test fixtures now cross real module boundaries.
                split = source.index('pub mod tests {')
                source = source[:split] + re.sub(r'^(    )(fn|struct|enum|trait) ', r'\1pub(crate) \2 ', source[split:], flags=re.M)
                source = source.replace('    use crate::*;\n    use crate::*;', '    use crate::*;')
            source = visibility(source)
            source = source.replace('pub use providers_of::', 'pub use super::capabilities::providers_of::')
            # Glob imports carry only explicitly exposed sibling state.
            first = source.find('\n') + 1 if source.startswith('//!') else 0
            source = source[:first] + 'use super::*;\n' + source[first:]
            subprocess.run(['git', 'mv', str(path), str(new / path.name)], check=True)
            (new / path.name).write_text(source)
        root = old_root[:old_root.index('use super::ticket_controller')] + imports
        root += '\ninclude!(concat!(env!("OUT_DIR"), "/parts_modules.rs"));\n'
        for name, bodies in traits.items():
            root += '\n' + envelopes[name].replace('super::', 'crate::') + '{' + ''.join(bodies).replace('super::', 'crate::') + '}\n'

        (new / 'mod.rs').write_text(root)
        subprocess.run(['git', 'rm', '-f', str(API / 'part_table.rs')], check=True)
        old.rmdir()
    # Replayable even after another structural PR updates a file.
    formatted = set(new.glob('*.rs'))
    formatted.add(API / 'registry_keys.rs')
    for folder in ('crates', 'tests', 'docs', '.github'):
        for path in (ROOT / folder).rglob('*'):
            if path.suffix not in ('.rs', '.py', '.md', '.yml') or not path.is_file():
                continue
            text = path.read_text()
            changed = text.replace('crate::part_table::', 'crate::parts::').replace('super::part_table::', 'super::parts::')
            if path == API / 'exports/part_table.rs':
                changed = changed.replace('mod part_table;', 'mod parts;').replace('use part_table::', 'use parts::')
            changed = changed.replace('crates/horizon-mod-api/src/part_table.rs', 'crates/horizon-mod-api/src/parts/mod.rs')
            if path.suffix == '.md':
                changed = changed.replace('`part_table.rs`', '`parts/`')
            if changed != text:
                path.write_text(changed)
                if path.suffix == '.rs':
                    formatted.add(path)
    from finish import finish
    if moved:
        finish(API)
    subprocess.run(['rustfmt', '--edition', '2024', '--config', 'skip_children=true', *map(str, sorted(formatted))], check=True)


if __name__ == '__main__':
    main()
