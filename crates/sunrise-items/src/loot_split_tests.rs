use super::*;
use serde_json::json;
use sunrise_core::{nbt::Tag, random::LegacyRandom};

#[test]
fn command_adapters_split_positive_stacks_and_keep_only_requested_empty_slots() {
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
    let root = json!({"pools":[
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}],
         "modifier":{"type":"minecraft:set_count","count":0}},
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}],
         "modifier":{"type":"minecraft:set_count","count":130}}],
        "modifier":{"type":"minecraft:set_components","components":{"minecraft:max_stack_size":64}}});
    let id = "minecraft:entities/zombie";
    let mut items = Items::for_tests(HashMap::new());
    items.loot = Arc::new(
        [
            (id.into(), root.clone()),
            ("minecraft:blocks/stone".into(), root.clone()),
        ]
        .into_iter()
        .collect(),
    );
    for adapter in 0..4 {
        for keep_empty in [false, true] {
            let mut random = LegacyRandom::new(7);
            let before = <(u64, Option<f64>)>::from(&random);
            let drops = match adapter {
                0 => items.loot_table_for_command(id, false, None, &mut random, keep_empty),
                1 => items.inline_loot_table_for_command(&root, None, &mut random, keep_empty),
                2 => items.entity_drops_for_command("zombie", false, None, &mut random, keep_empty),
                _ => items.block_drops(
                    &registry,
                    BlockStateId(0),
                    BlockLootRoll {
                        keep_empty,
                        ..Default::default()
                    },
                    &mut random,
                    None,
                ),
            };
            let mut expected = vec![
                ("minecraft:diamond", 64),
                ("minecraft:diamond", 64),
                ("minecraft:diamond", 2),
            ];
            if keep_empty {
                expected.insert(0, ("minecraft:gold_ingot", 0));
            }
            assert_eq!(
                drops
                    .iter()
                    .map(|s| (&*s.item, s.count))
                    .collect::<Vec<_>>(),
                expected,
                "adapter {adapter}, keep_empty {keep_empty}"
            );
            assert_eq!(<(u64, Option<f64>)>::from(&random), before);
        }
    }
}

#[test]
fn dynamic_block_output_splits_actual_items_not_internal_markers() {
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:decorated_pot":0},"states":[["decorated_pot",false,false,"","minecraft:decorated_pot"]]}"#[..]).unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.loot = Arc::new([("minecraft:blocks/decorated_pot".into(), json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:dynamic","name":"minecraft:sherds"}]}], "modifier":{"type":"minecraft:set_count","count":130}}))].into_iter().collect());
    let mut supplied = ItemStack::new("minecraft:diamond", 130);
    supplied
        .data_mut()
        .set_other("minecraft:max_stack_size", Some(Tag::Int(64)));
    let mut random = LegacyRandom::new(7);
    let before = <(u64, Option<f64>)>::from(&random);
    let drops = items.block_drops(
        &registry,
        BlockStateId(0),
        BlockLootRoll {
            dynamic: &[supplied],
            ..Default::default()
        },
        &mut random,
        None,
    );
    assert_eq!(
        drops
            .iter()
            .map(|s| (&*s.item, s.count))
            .collect::<Vec<_>>(),
        [
            ("minecraft:diamond", 64),
            ("minecraft:diamond", 64),
            ("minecraft:diamond", 2)
        ]
    );
    assert_eq!(<(u64, Option<f64>)>::from(&random), before);
}
