//! `Registries.PREDICATE` for block loot tables: a table's string condition
//! is a predicate id (`LootItemCondition.CODEC`, a `RegistryFileCodec`), so a
//! pack replaces `minecraft:tool/can_silk_touch` and adds its own ids.
use crate::items::{BlockLootRoll, ItemStack, Items};
use std::fs;
use sunrise_core::random::LegacyRandom;
use sunrise_world::loot::PackOverlay;
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// A block table whose first two branches name predicates by id: vanilla's
/// `minecraft:tool/can_silk_touch` (which the pack replaces) and a pack's own
/// `mypack:my_silk`.
const TABLE: &str = r#"{"type":"minecraft:block","pools":[{"rolls":1,"entries":[
    {"type":"minecraft:alternatives","children":[
        {"type":"minecraft:item","condition":"minecraft:tool/can_silk_touch","name":"minecraft:diamond"},
        {"type":"minecraft:item","condition":"mypack:my_silk","name":"minecraft:emerald"},
        {"type":"minecraft:item","condition":{"type":"minecraft:survives_explosion"},"name":"minecraft:cobblestone"}]}]}],
    "random_sequence":"minecraft:blocks/stone"}"#;

fn match_tool(items: &str) -> Vec<u8> {
    format!(r#"{{"type":"minecraft:match_tool","predicate":{{"items":"{items}"}}}}"#).into_bytes()
}

#[test]
fn block_tables_resolve_predicate_ids_from_the_packs() {
    let root = std::env::temp_dir().join(format!("sunrise-loot-predicates-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("data/minecraft/loot_table/blocks")).unwrap();
    fs::create_dir_all(root.join("data/minecraft/predicate/tool")).unwrap();
    fs::write(root.join("items.json"), "{}").unwrap();
    fs::write(
        root.join("data/minecraft/loot_table/blocks/stone.json"),
        TABLE,
    )
    .unwrap();
    // The root's own file for the built-in id: what the pack has to replace.
    fs::write(
        root.join("data/minecraft/predicate/tool/can_silk_touch.json"),
        match_tool("minecraft:diamond_pickaxe"),
    )
    .unwrap();
    // A referenced holder must also bind without packs (RegistryDataLoader).
    fs::create_dir_all(root.join("data/mypack/predicate")).unwrap();
    fs::write(
        root.join("data/mypack/predicate/my_silk.json"),
        match_tool("minecraft:golden_pickaxe"),
    )
    .unwrap();
    // The pack replaces both predicates, including the custom id with shears.
    let pack = [
        (
            "predicate/minecraft:tool/can_silk_touch".to_owned(),
            vec![match_tool("minecraft:stick")],
        ),
        (
            "predicate/mypack:my_silk".to_owned(),
            vec![match_tool("minecraft:shears")],
        ),
    ];
    let defaults = sunrise_world::item_defaults::shared(&root).unwrap();
    let items = Items::catalog(
        &root,
        &PackOverlay {
            files: &pack,
            ..PackOverlay::default()
        },
        &defaults,
    )
    .unwrap();
    let facts = br#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},
        "states":[["air",true,false,"","minecraft:air"],
                  ["stone",false,false,"","minecraft:stone"]]}"#;
    let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
    let drops = |items: &Items, held: Option<&ItemStack>| -> Vec<String> {
        items
            .block_drops(
                &registry,
                BlockStateId(1),
                BlockLootRoll {
                    held,
                    ..Default::default()
                },
                &mut LegacyRandom::new(7),
                None,
            )
            .into_iter()
            .map(|s| s.item.to_string())
            .collect()
    };
    let pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
    let stick = ItemStack::new("minecraft:stick", 1);
    let shears = ItemStack::new("minecraft:shears", 1);
    // The pack's file is the built-in id's content: the root's pickaxe
    // version no longer holds, the pack's stick one does.
    assert_eq!(drops(&items, Some(&pick)), ["minecraft:cobblestone"]);
    assert_eq!(drops(&items, Some(&stick)), ["minecraft:diamond"]);
    // The pack's own id resolves, and only for the tool it names.
    assert_eq!(drops(&items, Some(&shears)), ["minecraft:emerald"]);
    assert_eq!(drops(&items, None), ["minecraft:cobblestone"]);
    // Without the pack the root's own predicate file is the id's content.
    let base = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
    assert_eq!(drops(&base, Some(&pick)), ["minecraft:diamond"]);
    assert_eq!(drops(&base, Some(&stick)), ["minecraft:cobblestone"]);
    assert_eq!(drops(&base, Some(&shears)), ["minecraft:cobblestone"]);
    fs::remove_dir_all(root).unwrap();
}

/// The game's own `minecraft:tool/can_silk_touch` file, read through the
/// registry: stone drops itself only under Silk Touch (skips without the
/// prepared resources CI doesn't have).
#[test]
fn the_games_own_silk_touch_predicate_still_drops_stone() {
    let Some(items) = crate::items::prepared_items() else {
        return;
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/sunrise-client/26.3");
    let Ok((registry, _)) =
        BlockRegistry::from_facts_json_with_flags(fs::File::open(root.join("facts.json")).unwrap())
    else {
        return;
    };
    let Some(stone) = registry.default_state("minecraft:stone") else {
        return;
    };
    let drops = |held: &ItemStack| -> Vec<String> {
        items
            .block_drops(
                &registry,
                stone,
                BlockLootRoll {
                    held: Some(held),
                    ..Default::default()
                },
                &mut LegacyRandom::new(7),
                None,
            )
            .into_iter()
            .map(|s| s.item.to_string())
            .collect()
    };
    let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
    assert_eq!(drops(&pick), ["minecraft:cobblestone"]);
    pick.enchant("minecraft:silk_touch", 1);
    assert_eq!(drops(&pick), ["minecraft:stone"]);
}
