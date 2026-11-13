"""Module boundary visibility and fixture wiring, shared by the replay script."""
from pathlib import Path
import re


def finish(api):
    for p in (api / 'parts').glob('*.rs'):
        s = p.read_text()
        if p.name != 'mod.rs' and 'pub mod tests {' in s:
            i = s.index('pub mod tests {')
            a, b = s[:i], s[i:]
            b = re.sub(r'^(    )(const|static|mod) ', r'\1pub(crate) \2 ', b, flags=re.M)
            from move import layout
            lines = b.splitlines(keepends=True)
            test = layout(b)[0]
            for item in test.get('children', []):
                start, end = item['start'] - 1, item['end']
                if re.search(r'^    (?:pub(?:\([^)]*\))? )?struct ', ''.join(lines[start:end]), re.M):
                    for n in range(start, end):
                        lines[n] = re.sub(r'^(        )(\w+:)', r'\1pub(crate) \2', lines[n])
            b = ''.join(lines)
            b = re.sub(r'(pub\(crate\) struct \w+\()([^\n]+)(\);)', lambda m: m[1] + ', '.join('pub(crate) '+x.strip() if not x.strip().startswith('pub') else x.strip() for x in m[2].split(',')) + m[3], b)
            if p.stem == 'blocks':
                b = b.replace('let installed = super::TEST_PARTS.get();', 'let installed_parts = super::TEST_PARTS.get();').replace('TEST_PARTS.set(installed)', 'TEST_PARTS.set(installed_parts)')
            if p.stem == 'shared':
                b = b.replace('use crate::*;', 'use crate::*;\n    use super::super::items::tests::Cell;')
            if p.stem in ('effects', 'game_rules', 'key_mappings', 'structures', 'tickets'):
                b = b.replace('    use crate::parts::tests::*;\n', '')
            if p.stem == 'tickets':
                b = b.replace('    use crate::*;\n', '')
            if p.stem == 'shared':
                for text in ('    #[cfg(test)]\n    use super::{Attached, KnownIds, resolve_parts};\n', '    #[cfg(test)]\n    use std::any::type_name;\n', '    #[cfg(test)]\n    use std::collections::{HashMap, HashSet};\n'):
                    b = b.replace(text, '')
            test_only_imports = {
                'block_entities': ('super::*', 'crate::parts::tests::*'),
                'blocks': ('crate::parts::tests::*',),
                'capabilities': ('super::*', 'crate::parts::tests::*'),
                'effects': ('super::*', 'crate::*'),
                'energy': ('super::*',),
                'fluids': ('super::*',),
                'game_rules': ('super::*', 'crate::*'),
                'key_mappings': ('super::*', 'crate::*'),
                'structures': ('super::*', 'crate::*'),
                'tickets': ('super::*',),
            }
            for name in test_only_imports.get(p.stem, ()):
                b = b.replace(f'    use {name};', f'    #[cfg(test)]\n    use {name};')
            if p.stem == 'key_mappings':
                b = b.replace('    #[cfg(test)]\n    use super::*;\n', '')
            s = a + b
        if p.name == 'energy.rs':
            s = s.replace('use super::*;\n', '#[cfg(test)]\nuse super::*;\n', 1)
        p.write_text(s)
    p = api / 'exports/part_table.rs'
    s = p.read_text().replace('part_table::', 'parts::')
    s = s.replace('#[cfg(any(test, feature = "testing"))]\npub(crate) use parts::install_parts;', '#[cfg(test)]\npub(crate) use parts::install_parts;')
    p.write_text(s)
