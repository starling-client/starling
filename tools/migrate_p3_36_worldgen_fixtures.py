#!/usr/bin/env python3
"""Route P3-36 fixtures through scoped level/biome binding helpers."""
from pathlib import Path


def update(path: str, old: str, new: str, *, tests_only: bool = False) -> None:
    file = Path(path)
    text = file.read_text()
    start = text.index("#[cfg(test)]\nmod tests {") if tests_only else 0
    file.write_text(text[:start] + text[start:].replace(old, new))


update(
    "crates/sunrise-client/src/modgen.rs",
    "ModTerrain::for_stem(",
    "for_stem(",
    tests_only=True,
)
update(
    "crates/sunrise-world/src/worldgen/generator/mod_terrain.rs",
    "OverworldGenerator::for_mod(",
    "for_mod(",
    tests_only=True,
)
for id_arg in ('&format!("cap:l{n}")', '"gen:a"', '"state:a"', '"del:a"'):
    update(
        "crates/sunrise-client/src/game/world/runtime_levels_tests.rs",
        f"{id_arg}, flat())",
        f"{id_arg}, admitted_flat(1))",
    )
update(
    "crates/sunrise-client/src/game/world/runtime_levels_tests.rs",
    '.queue_open("cycle", "cycle:a", level)',
    '.queue_open("cycle", "cycle:a", admitted_flat(seed))',
)
