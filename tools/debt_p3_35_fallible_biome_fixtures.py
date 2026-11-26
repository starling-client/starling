#!/usr/bin/env python3
"""Update biome callers' fixtures to unwrap registry loading before checking results."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def replace(path: str, substitutions: dict[str, str]) -> None:
    file = ROOT / path
    text = file.read_text()
    for old, new in substitutions.items():
        if new in text and old not in text:
            continue
        if text.count(old) != 1:
            raise SystemExit(f"{file}: expected exactly one {old!r}")
        text = text.replace(old, new)
    file.write_text(text)


replace(
    "crates/sunrise-client-render/src/biome.rs",
    {
        "base.with_packs(files);": "base.with_packs(files).unwrap();",
        "world.with_packs([]);": "world.with_packs([]).unwrap();",
        'next.with_packs([("limbo:void", &json!({}))]);':
        'next.with_packs([("limbo:void", &json!({}))]).unwrap();',
        "biomes.with_packs([]);": "biomes.with_packs([]).unwrap();",
    },
)
replace(
    "crates/sunrise-client/src/game/world/world_loading.rs",
    {"game.res.biomes().with_packs([]);": "game.res.biomes().with_packs([]).unwrap();"},
)
replace(
    "crates/sunrise-client/tests/it/datapack_biome_sync.rs",
    {
        'base.with_packs([("minecraft:tundra_x", &pack)]);':
        'base.with_packs([("minecraft:tundra_x", &pack)]).unwrap();',
        'client_biomes(&base, &received).expect("the pack\'s biome is new to the client");':
        'client_biomes(&base, &received).unwrap().expect("the pack\'s biome is new to the client");',
        "client_biomes(&base, &same).is_none()": "client_biomes(&base, &same).unwrap().is_none()",
        "client_biomes(&base, &known).unwrap();": "client_biomes(&base, &known).unwrap().unwrap();",
        "client_biomes(&base, &mixed).unwrap();": "client_biomes(&base, &mixed).unwrap().unwrap();",
        'client_biomes(&base, &received).expect("plains differs from the client\'s file");':
        'client_biomes(&base, &received).unwrap().expect("plains differs from the client\'s file");',
        'client_biomes(&base, &received).expect("plains differs");':
        'client_biomes(&base, &received).unwrap().expect("plains differs");',
    },
)
replace(
    "crates/sunrise-client/tests/it/mod_biome_sync.rs",
    {
        'with.with_packs([("pack:late", &pack), ("zinc:foo", &pack)]);':
        'with.with_packs([("pack:late", &pack), ("zinc:foo", &pack)]).unwrap();',
        'client_biomes(&load(&client), &received).expect("the mods\' biomes are new");':
        'client_biomes(&load(&client), &received).unwrap().expect("the mods\' biomes are new");',
        "client_biomes(&load(&modded), &received).is_none()":
        "client_biomes(&load(&modded), &received).unwrap().is_none()",
    },
)
