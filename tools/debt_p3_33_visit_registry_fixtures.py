#!/usr/bin/env python3
"""Migrate the P3-33 registry-list fixtures to scoped key visits (run once)."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/sunrise-client/src/game"


def main():
    updates = {}

    def replace(path, pattern, replacement, expected=1):
        path = GAME / path
        source = updates[path] if path in updates else path.read_text(encoding="utf-8")
        source, count = re.subn(pattern, replacement, source)
        if count != expected:
            raise SystemExit(f"{path.name}: expected {expected} matches, found {count}: {pattern}")
        updates[path] = source

    entries = "modding/native_registry_entries_tests.rs"
    replace(entries, r"use std::\{path::Path, sync::Arc\};", "use std::{ops::ControlFlow, path::Path, sync::Arc};")
    replace(entries, r"(?=// Feed the same server-built snapshot)", """fn registry_lists(host: &dyn mod_api::Host, registry: &str, expected: &str) -> bool {
    host.visit_registry_entries(registry, &mut |id| {
        if id == expected {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .unwrap_or_else(|error| panic!("{registry}: {error:?}"))
    .is_break()
}

""")
    replace(
        entries,
        r'host\(game\)\.registry_entries\(([^)]+)\),(\s+)None',
        r'host(game).visit_registry_entries(\1, &mut |_| panic!("unavailable registry visited")),\2Err(mod_api::RegistryReadError::UnknownRegistry)',
        6,
    )
    replace(
        entries,
        r'host\(game\)\.registry_entries\(("registry_entry_test:spell")\),\s+Some\(vec!\[id\(("[^"]+")\)\]\)',
        r'{\n                        let mut ids = Vec::new();\n                        assert_eq!(host(game).visit_registry_entries(\1, &mut |id| {\n                            ids.push(id.to_owned());\n                            ControlFlow::Continue(())\n                        }), Ok(ControlFlow::Continue(())));\n                        ids\n                    },\n                    [\2]',
        2,
    )
    replace(
        entries,
        r'host\(game\)\s*\.registry_entries\(([^)]+)\)\s*\.unwrap\(\)\s*\.contains\(&id\(([^)]+)\)\)',
        r'registry_lists(host(game), \1, \2)',
        5,
    )
    replace(
        entries,
        r'host\(game\)\s*\.registry_entries\("minecraft:damage_type"\)\s*\.unwrap\(\)\s*\.contains\(&fall.id\)',
        'registry_lists(host(game), "minecraft:damage_type", "minecraft:fall")',
    )
    replace(
        entries,
        r'            let entries = host\(game\)\.registry_entries\(registry\)\.unwrap_or_else\(\|\| \{\n.*\n            \}\);\n',
        '',
    )
    replace(entries, r'entries\.contains\(&id\(("[^"]+")\)\)', r'registry_lists(host(game), registry, \1)', 2)

    history = "modding/native_registry_history_tests.rs"
    replace(
        history,
        r'\b(host\.registry_entries|data\.registry_ids)\(("minecraft:worldgen/world_preset")\),(\s+)None',
        lambda m: f'{"host.visit_registry_entries" if m[1].startswith("host") else "data.visit_registry_ids"}'
        f'({m[2]}, &mut |_| panic!("unavailable registry visited")),{m[3]}'
        f'{"Err(mod_api::RegistryReadError::UnknownRegistry)" if m[1].startswith("host") else "None"}',
        2,
    )
    processor = "modding/native_registry_processor_tests.rs"
    replace(processor, r"use std::\{path::Path, sync::Arc\};", "use std::{ops::ControlFlow, path::Path, sync::Arc};")
    replace(processor, r'        let (accepted|rejected) = mod_api::Identifier::parse\(("[^"]+")\).unwrap\(\);\n', '', 2)
    replace(
        processor,
        r'            let entries = host\.registry_entries\(registry\)\.unwrap\(\);\n'
        r'            assert!\(entries\.contains\(&accepted\), "\{registry\}"\);\n'
        r'            assert!\(!entries\.contains\(&rejected\), "\{registry\}"\);',
        '''            let mut accepted = false;
            assert_eq!(host.visit_registry_entries(registry, &mut |id| {
                accepted |= id == "registry_processor_test:pillar";
                assert_ne!(id, "registry_processor_test:rejected", "{registry}");
                ControlFlow::Continue(())
            }), Ok(ControlFlow::Continue(())));
            assert!(accepted, "{registry}");''',
    )

    replace(
        "world/villager_professions/tests.rs",
        r'    assert!\(\s*host\.registry_entries\("minecraft:villager_profession"\)\s*\.unwrap\(\)'
        r'\s*\.contains\(&Identifier::parse\("test:engineer"\)\.unwrap\(\)\)\s*\);\n'
        r'    assert!\(\s*host\.registry_entries\("minecraft:villager_profession"\)\s*\.unwrap\(\)'
        r'\s*\.contains\(&Identifier::parse\("minecraft:farmer"\)\.unwrap\(\)\)\s*\);',
        '''    use std::ops::ControlFlow;
    let mut engineer_seen = false;
    let mut farmer_seen = false;
    assert_eq!(host.visit_registry_entries("minecraft:villager_profession", &mut |id| {
        engineer_seen |= id == "test:engineer";
        farmer_seen |= id == "minecraft:farmer";
        ControlFlow::Continue(())
    }), Ok(ControlFlow::Continue(())));
    assert!(engineer_seen && farmer_seen);''',
    )

    for path, source in updates.items():
        if re.search(r"\.registry_entries\(", source):
            raise SystemExit(f"{path.name}: missed a registry_entries call")
    for path, source in updates.items():
        path.write_text(source, encoding="utf-8")


if __name__ == "__main__":
    main()
