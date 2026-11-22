thread_local! {
    static OBSERVED_CONTEXT: std::cell::RefCell<Option<(String, Option<sunrise_world::loot::LootStack>)>> = const { std::cell::RefCell::new(None) };
    static OBSERVED_ABILITY: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}
use super::*;
use serde_json::json;
use std::rc::Rc;
use sunrise_core::random::LegacyRandom;
use sunrise_world::loot::query;
struct QueryCodec;
impl sunrise_world::loot::types::Registry for QueryCodec {
    fn contains(&self, kind: &str, function: bool) -> bool {
        !function && kind == "test:query"
    }
    fn validate(&self, _: &str, _: &Value, _: bool) -> std::result::Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    fn conditional(&self, _: &str) -> bool {
        true
    }
    fn apply(
        &self,
        _: &str,
        _: &Value,
        _: &sunrise_world::loot::LootContext<'_>,
        _: &mut dyn sunrise_world::loot::types::RollRandom,
        _: &dyn sunrise_world::loot::LootTags,
        stack: &sunrise_world::loot::LootStack,
    ) -> std::result::Result<sunrise_world::loot::LootStack, String> {
        Ok(stack.clone())
    }
    fn test(
        &self,
        _: &str,
        _: &Value,
        ctx: &sunrise_world::loot::LootContext<'_>,
        _: &mut dyn sunrise_world::loot::types::RollRandom,
        _: &dyn sunrise_world::loot::LootTags,
    ) -> std::result::Result<bool, String> {
        OBSERVED_CONTEXT.with(|value| {
            *value.borrow_mut() = Some((ctx.table_id().to_owned(), ctx.queried_stack.clone()))
        });
        Ok(true)
    }
}
fn data(tables: Vec<(&str, Value)>) -> Items {
    let mut items = Items::for_tests(HashMap::new());
    items.loot_types = sunrise_world::loot::types::Types::new(Arc::new(QueryCodec));
    items.loot = Arc::new(tables.into_iter().map(|(k, v)| (k.into(), v)).collect());
    items
}
fn table(id: &str) -> Value {
    json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:loot_table_id","loot_table_id":id},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]})
}
#[test]
fn every_root_splits_after_outer_functions_with_one_random_draw() {
    let nested = json!({"pools":[{"rolls":1,
        "condition":{"type":"neoforge:loot_table_id","loot_table_id":"neoforge:unknown_loot_table"},
        "entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"},
                   {"type":"minecraft:item","name":"minecraft:diamond"}]}],
        "modifier":{"type":"minecraft:set_count","count":200}});
    let root = json!({"pools":[{"rolls":1,"entries":[
        {"type":"minecraft:loot_table","value":"test:nested"}]}],
        "modifier":[{"type":"minecraft:set_count","count":130},
                    {"type":"minecraft:set_components","components":{"minecraft:max_stack_size":64}}]});
    let id = "minecraft:entities/zombie";
    let block_id = "minecraft:blocks/stone";
    let data = data(vec![
        (id, root.clone()),
        (block_id, root.clone()),
        ("test:nested", nested),
    ]);
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
    // Only the child's two-entry weighted choice advances this random.
    let mut expected_random = LegacyRandom::new(0);
    assert_eq!(expected_random.next_int_bound(2).unwrap(), 1);
    let expected_state = <(u64, Option<f64>)>::from(&expected_random);
    for adapter in 0..7 {
        let query = Rc::new(query::LootQuery::default());
        let mut random = LegacyRandom::new(0);
        let drops = query::with_query(query.clone(), || match adapter {
            0 => data.loot_table(id, false, None, &mut random),
            1 => data.roll_loot_table(id, None, None, &mut random).unwrap(),
            2 => data.inline_loot_table(&root, &mut random),
            3 => data.entity_drops("zombie", false, None, &mut random),
            4 => data.loot_table_from(id, false, None, &mut random),
            5 => data.block_drops(
                &registry,
                BlockStateId(0),
                Default::default(),
                &mut random,
                None,
            ),
            _ => data.interact_loot(
                id,
                &registry,
                BlockStateId(0),
                None,
                None,
                &mut random,
                &mut crate::random_sequences::RandomSequences::default(),
                0,
            ),
        });
        let observed: Vec<_> = drops.iter().map(|s| (&*s.item, s.count)).collect();
        let expected = vec![
            ("minecraft:diamond", 64),
            ("minecraft:diamond", 64),
            ("minecraft:diamond", 2),
        ];
        assert_eq!(observed, expected, "adapter {adapter}");
        assert_eq!(
            <(u64, Option<f64>)>::from(&random),
            expected_state,
            "adapter {adapter}"
        );
        assert_eq!(
            query.id().as_deref(),
            Some(if adapter == 2 {
                "neoforge:unknown_loot_table"
            } else if adapter == 5 {
                block_id
            } else {
                id
            })
        );
    }
}

#[test]
fn public_block_and_interaction_roots_complete_on_the_selected_random() {
    use crate::random_sequences::RandomSequences;
    use sunrise_core::nbt::Tag;

    // A wall block resolves the standing block's actual table, not its own name.
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:oak_wall_sign":0},"states":[["oak_wall_sign",false,false,"","minecraft:oak_wall_sign"]]}"#[..]).unwrap();
    for interaction in [false, true] {
        for named_sequence in [false, true] {
            let id = if interaction {
                "test:interact"
            } else {
                "minecraft:blocks/oak_sign"
            };
            let mut root = json!({"pools":[{"rolls":1,
                "condition":{"type":"neoforge:loot_table_id","loot_table_id":"neoforge:unknown_loot_table"},
                "entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"},
                           {"type":"minecraft:item","name":"minecraft:diamond"}]}]});
            if named_sequence {
                root["random_sequence"] = json!("test:selected");
            }
            let items = data(vec![(id, root)]);
            let mut sequences = RandomSequences::from_nbt(&Tag::compound([(
                "sequences",
                Tag::compound([
                    (
                        "test:selected",
                        Tag::compound([("source", Tag::LongArray(vec![17, 23]))]),
                    ),
                    (
                        "test:untouched",
                        Tag::compound([("source", Tag::LongArray(vec![31, 47]))]),
                    ),
                ]),
            )]));
            let mut expected_sequence = sequences.get("test:selected", 0).clone();
            let untouched = sequences.get("test:untouched", 0).state_words();
            let mut random = LegacyRandom::new(0);
            let mut expected_random = random.clone();
            let pick = if named_sequence {
                expected_sequence.next_int_bound(2).unwrap()
            } else {
                expected_random.next_int_bound(2).unwrap()
            };
            let expected_item = ["minecraft:gold_ingot", "minecraft:diamond"][pick as usize];
            let query = Rc::new(query::LootQuery::default());
            let drops = query::with_query(query.clone(), || {
                if interaction {
                    items.interact_loot(
                        id,
                        &registry,
                        BlockStateId(0),
                        None,
                        None,
                        &mut random,
                        &mut sequences,
                        0,
                    )
                } else {
                    items.block_drops(
                        &registry,
                        BlockStateId(0),
                        Default::default(),
                        &mut random,
                        Some((&mut sequences, 0)),
                    )
                }
            });
            assert_eq!(drops.len(), 1);
            assert_eq!((&*drops[0].item, drops[0].count), (expected_item, 1));
            assert_eq!(query.id().as_deref(), Some(id));
            assert_eq!(
                <(u64, Option<f64>)>::from(&random),
                <(u64, Option<f64>)>::from(&expected_random)
            );
            assert_eq!(
                sequences.get("test:selected", 0).state_words(),
                expected_sequence.state_words()
            );
            assert_eq!(sequences.get("test:untouched", 0).state_words(), untouched);
        }
    }
}

#[test]
fn queried_id_is_written_after_root_and_first_wins() {
    for (guard, preset, count) in [
        ("minecraft:blocks/short_grass", None, 0),
        ("neoforge:unknown_loot_table", None, 1),
        (
            "minecraft:blocks/short_grass",
            Some("minecraft:blocks/short_grass"),
            1,
        ),
    ] {
        let data = data(vec![("minecraft:blocks/short_grass", table(guard))]);
        let query = Rc::new(query::LootQuery::default());
        if let Some(id) = preset {
            query.set_id(id);
        }
        let drops = query::with_query(query.clone(), || {
            data.loot_table(
                "minecraft:blocks/short_grass",
                false,
                None,
                &mut LegacyRandom::new(7),
            )
        });
        assert_eq!(drops.len(), count);
        if count == 1 {
            assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
        }
        assert_eq!(query.table_id(), "minecraft:blocks/short_grass");
    }
}
#[test]
fn queried_id_nested_and_unnamed_roots() {
    let data = data(vec![
        (
            "minecraft:chests/simple_dungeon",
            json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:b"}]}]}),
        ),
        ("test:b", table("neoforge:unknown_loot_table")),
    ]);
    let query = Rc::new(query::LootQuery::default());
    let drops = query::with_query(query.clone(), || {
        data.loot_table(
            "minecraft:chests/simple_dungeon",
            false,
            None,
            &mut LegacyRandom::new(9),
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "minecraft:chests/simple_dungeon");
    let query = Rc::new(query::LootQuery::default());
    let drops = query::with_query(query.clone(), || {
        data.inline_loot_table(
            &table("neoforge:unknown_loot_table"),
            &mut LegacyRandom::new(9),
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
    let query = Rc::new(query::LootQuery::default());
    let inline = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":table("neoforge:unknown_loot_table")}]}]});
    let drops = query::with_query(query.clone(), || {
        data.inline_loot_table(&inline, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
}

#[test]
fn queried_context_and_ability_reach_native_adapter() {
    use sunrise_world::native_load_hooks::{self, NativeLoadHooks};
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            item_ability_default: |item, ability, tags| match ability {
                "shears_dig" => item == "minecraft:shears",
                "sword_sweep" => tags.contains("item", "minecraft:swords", item),
                _ => false,
            },
            loot_table_load: |_, _, _| {},
            modify_default_components: |_, _, _| {},
            loot_ability: |tool, ability, holds| {
                OBSERVED_ABILITY
                    .with(|v| *v.borrow_mut() = Some((tool.item.clone(), ability.to_owned())));
                if tool.item == "test:grant" && ability == "shears_dig" {
                    *holds = true;
                }
                if tool.item == "test:deny" {
                    *holds = false;
                }
            },
        })
        .is_ok()
    );
    let stack = || {
        super::loot_of(&ItemStack {
            item: Arc::from("minecraft:stick"),
            count: 3,
            damage: 0,
            extra: None,
        })
    };
    for (tool, expected) in [
        (None, 0),
        (Some("minecraft:stick"), 0),
        (Some("minecraft:shears"), 1),
        (Some("minecraft:stone_pickaxe"), 0),
        (Some("test:grant"), 1),
        (Some("test:deny"), 0),
    ] {
        for ability in ["shears_dig", "minecraft:shears_dig", "unknown_ability"] {
            let table = json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:can_item_perform_ability","ability":ability},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
            let data = data(vec![
                (
                    "test:a",
                    json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:b"}]}]}),
                ),
                ("test:b", table),
            ]);
            let query = query::LootQuery::new(
                None,
                tool.map(|item| {
                    let mut s = stack();
                    s.item = item.into();
                    s
                }),
            );
            let query = Rc::new(query);
            let held = tool.map(|item| ItemStack {
                item: Arc::from(item),
                count: 1,
                damage: 0,
                extra: None,
            });
            let drops = query::with_query(query, || {
                data.loot_table("test:a", false, held.as_ref(), &mut LegacyRandom::new(9))
            });
            assert_eq!(
                OBSERVED_ABILITY.with(|v| v.take()),
                tool.map(|item| (item.to_owned(), ability.to_owned()))
            );
            assert_eq!(
                drops.len(),
                if ability == "shears_dig" { expected } else { 0 }
            );
        }
    }
    let empty_shears = ItemStack {
        item: Arc::from("minecraft:shears"),
        count: 0,
        damage: 0,
        extra: None,
    };
    let guard = json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:can_item_perform_ability","ability":"shears_dig"},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let empty = data(vec![("test:a", guard)]).loot_table(
        "test:a",
        false,
        Some(&empty_shears),
        &mut LegacyRandom::new(9),
    );
    assert_eq!(empty.len(), 0);
    assert_eq!(OBSERVED_ABILITY.with(|v| v.take()), None);
    let table = json!({"pools":[{"rolls":1,"condition":{"type":"test:query"},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let data = data(vec![
        (
            "test:a",
            json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:b"}]}]}),
        ),
        ("test:b", table),
    ]);
    let query = query::LootQuery::new(Some(stack()), None);
    let query = Rc::new(query);
    query.set_id("minecraft:blocks/short_grass");
    let drops = query::with_query(query, || {
        data.loot_table("test:a", false, None, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    let (id, stack) = OBSERVED_CONTEXT.with(|v| v.take()).expect("query callback");
    assert_eq!(id, "minecraft:blocks/short_grass");
    let stack = stack.expect("queried stack");
    assert_eq!((&*stack.item, stack.count), ("minecraft:stick", 3));
}

#[test]
fn queried_ability_default_sets_have_literal_outcomes() {
    let items = Items::for_tests(HashMap::new());
    for (item, abilities) in [
        (
            "minecraft:shears",
            &[
                "shears_dig",
                "shears_harvest",
                "shears_remove_armor",
                "shears_carve",
                "shears_disarm",
                "shears_trim",
            ][..],
        ),
        ("minecraft:fishing_rod", &["fishing_rod_cast"][..]),
        ("minecraft:trident", &["trident_throw"][..]),
        ("minecraft:brush", &["brush_brush"][..]),
        ("minecraft:flint_and_steel", &["firestarter_light"][..]),
        ("minecraft:fire_charge", &["firestarter_light"][..]),
        ("minecraft:spyglass", &["spyglass_scope"][..]),
    ] {
        for ability in abilities {
            assert!(
                items.vanilla_can_perform(item, ability),
                "{item}: {ability}"
            );
        }
    }
    for item in [
        "minecraft:shears",
        "minecraft:stone_pickaxe",
        "minecraft:shield",
        "minecraft:bow",
        "minecraft:crossbow",
        "minecraft:mace",
        "minecraft:diamond_axe",
        "minecraft:diamond_hoe",
        "minecraft:diamond_shovel",
    ] {
        for ability in [
            "unknown_ability",
            "minecraft:shears_dig",
            "SHEARS_DIG",
            "shield_block",
            "axe_strip",
            "pickaxe_dig",
        ] {
            assert!(!items.vanilla_can_perform(item, ability));
        }
    }
    assert!(!items.vanilla_can_perform("minecraft:stone_pickaxe", "shears_dig"));
    assert!(!items.vanilla_can_perform("minecraft:diamond_sword", "sword_sweep"));
}
#[test]
#[ignore = "daily"]
fn queried_ability_tags_use_prepared_item_sets() {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .or_else(|| std::env::var_os("SUNRISE_RESOURCES"))
            .expect("prepared resources"),
    );
    let (registry, _) = BlockRegistry::from_facts_json_with_flags(
        std::fs::File::open(root.join("facts.json")).unwrap(),
    )
    .unwrap();
    let facts = crate::blocks::BlockFacts::load(&root, &registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = Items::load(&root, &registry, &facts, &PackOverlay::NONE, &defaults).unwrap();
    assert!(items.vanilla_can_perform("minecraft:diamond_sword", "sword_sweep"));
    assert!(items.vanilla_can_perform("minecraft:diamond_shovel", "shovel_douse"));
    assert!(!items.vanilla_can_perform("minecraft:diamond_axe", "sword_sweep"));
    assert!(!items.vanilla_can_perform("minecraft:diamond_sword", "minecraft:sword_sweep"));
    let guard = json!({"type":"neoforge:can_item_perform_ability","ability":"sword_sweep"});
    let props = sunrise_world::chunk::BlockProperties::new();
    let tool = ItemStack {
        item: Arc::from("minecraft:diamond_sword"),
        count: 1,
        damage: 0,
        extra: None,
    };
    let context = LootContext {
        query: query::current_query(),
        items: &items,
        block: "",
        properties: &props,
        held: Some(&tool),
        killed_by_player: false,
        explosion_radius: None,
        this_entity: false,
        entity_levels: None,
        block_levels: None,
        target: None,
        target_entity: None,
        interacting_entity: None,
        location: None,
        keep_empty: false,
    };
    assert!(context.condition(&guard, &mut LegacyRandom::new(9)));
    for invalid in [
        json!({"type":"neoforge:can_item_perform_ability"}),
        json!({"type":"neoforge:can_item_perform_ability","ability":7}),
        json!({"type":"neoforge:can_item_perform_ability","ability":"unknown_ability"}),
    ] {
        assert!(!context.condition(&invalid, &mut LegacyRandom::new(9)));
    }
}

#[test]
fn root_modifiers_chain_before_unsigned_caller_consumption() {
    use sunrise_world::loot::{
        LootStack,
        global_modifiers::{
            AddTableModifier, DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile,
            ModifierRoll,
        },
    };
    struct Stage(bool);
    impl ModifierApply for Stage {
        fn apply(
            &self,
            roll: &mut dyn ModifierRoll,
            generated: &[LootStack],
        ) -> std::result::Result<Vec<LootStack>, String> {
            if self.0 {
                assert_eq!(
                    generated
                        .iter()
                        .map(|stack| stack.count)
                        .collect::<Vec<_>>(),
                    [-3, 200, 200]
                );
                return Ok(generated.to_vec());
            }
            assert_eq!(
                generated
                    .iter()
                    .map(|stack| stack.count)
                    .collect::<Vec<_>>(),
                [64, 64, 2]
            );
            assert!(matches!(
                roll.facts().table_id(),
                "minecraft:entities/zombie"
                    | "minecraft:blocks/stone"
                    | "neoforge:unknown_loot_table"
            ));
            assert_eq!(roll.next_long(), LegacyRandom::new(31).next_long());
            let raw = roll.roll_raw("test:raw")?;
            assert_eq!(
                raw.iter().map(|stack| stack.count).collect::<Vec<_>>(),
                [200]
            );
            let mut negative = generated[0].clone();
            negative.count = -3;
            Ok(vec![negative, raw[0].clone()])
        }
    }
    let root = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}],
        "modifier":[{"type":"minecraft:set_count","count":130},{"type":"minecraft:set_components","components":{"minecraft:max_stack_size":64}}]});
    let raw = json!({"pools":[
        {"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:raw"}]},
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot","modifier":{"type":"minecraft:set_count","count":200}}]}
    ]});
    let mut items = data(vec![
        ("minecraft:entities/zombie", root.clone()),
        ("minecraft:blocks/stone", root.clone()),
        ("test:raw", raw),
    ]);
    let files = ["test:first", "test:add", "test:last"]
        .into_iter()
        .map(|id| ModifierFile {
            id: id.into(),
            pack: "test".into(),
            path: id.into(),
            read: Box::new(|| Ok(br#"{"type":"test:stage"}"#.to_vec())),
        });
    items.global_modifiers = Arc::new(GlobalLootModifiers::load(
        files,
        &sunrise_world::conditions::LoadConditions::vanilla(),
        |id, _, _| {
            let (priority, apply): (_, Arc<dyn ModifierApply>) = match id {
                "test:first" => (1001, Arc::new(Stage(false))),
                "test:add" => (
                    1000,
                    Arc::new(AddTableModifier {
                        table: "test:raw".into(),
                    }),
                ),
                _ => (999, Arc::new(Stage(true))),
            };
            Ok(DecodedModifier {
                priority,
                condition: None,
                apply,
            })
        },
        |error| panic!("{error}"),
    ));
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
    let mut expected = LegacyRandom::new(31);
    expected.next_long();
    for adapter in 0..7 {
        let mut random = LegacyRandom::new(31);
        let drops = match adapter {
            0 => items.loot_table("minecraft:entities/zombie", false, None, &mut random),
            1 => items
                .roll_loot_table("minecraft:entities/zombie", None, None, &mut random)
                .unwrap(),
            2 => items.inline_loot_table(&root, &mut random),
            3 => items.entity_drops("zombie", false, None, &mut random),
            4 => items.loot_table_from("minecraft:entities/zombie", false, None, &mut random),
            5 => items.block_drops(
                &registry,
                BlockStateId(0),
                Default::default(),
                &mut random,
                None,
            ),
            _ => items.interact_loot(
                "minecraft:entities/zombie",
                &registry,
                BlockStateId(0),
                None,
                None,
                &mut random,
                &mut crate::random_sequences::RandomSequences::default(),
                0,
            ),
        };
        assert_eq!(
            drops.iter().map(|stack| stack.count).collect::<Vec<_>>(),
            [0, 200, 200],
            "adapter {adapter}"
        );
        assert_eq!(
            <(u64, Option<f64>)>::from(&random),
            <(u64, Option<f64>)>::from(&expected)
        );
    }
}

#[test]
fn block_dynamic_stacks_are_materialized_before_the_pass_and_raw_output_is_unsplit() {
    use sunrise_world::loot::{
        LootStack,
        global_modifiers::{
            DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile, ModifierRoll,
        },
    };
    struct Dynamic;
    impl ModifierApply for Dynamic {
        fn apply(
            &self,
            roll: &mut dyn ModifierRoll,
            generated: &[LootStack],
        ) -> std::result::Result<Vec<LootStack>, String> {
            assert_eq!(
                generated
                    .iter()
                    .map(|stack| (stack.item.as_str(), stack.count))
                    .collect::<Vec<_>>(),
                [
                    ("minecraft:diamond", 64),
                    ("minecraft:diamond", 64),
                    ("minecraft:diamond", 2),
                ]
            );
            let raw = roll.roll_raw("test:dynamic")?;
            assert_eq!(
                raw.iter()
                    .map(|stack| (stack.item.as_str(), stack.count))
                    .collect::<Vec<_>>(),
                [("minecraft:diamond", 130)]
            );
            let mut output = generated.to_vec();
            output.extend(raw);
            Ok(output)
        }
    }
    let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:dynamic","name":"minecraft:sherds"}]}]});
    let mut items = data(vec![
        ("minecraft:blocks/decorated_pot", table.clone()),
        ("test:dynamic", table),
    ]);
    items.global_modifiers = Arc::new(GlobalLootModifiers::load(
        [ModifierFile {
            id: "test:dynamic".into(),
            pack: "test".into(),
            path: "dynamic.json".into(),
            read: Box::new(|| Ok(br#"{"type":"test:dynamic"}"#.to_vec())),
        }],
        &sunrise_world::conditions::LoadConditions::vanilla(),
        |_, _, _| {
            Ok(DecodedModifier {
                priority: 1000,
                condition: None,
                apply: Arc::new(Dynamic),
            })
        },
        |error| panic!("{error}"),
    ));
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:decorated_pot":0},"states":[["decorated_pot",false,false,"","minecraft:decorated_pot"]]}"#[..]).unwrap();
    let mut supplied = ItemStack::new("minecraft:diamond", 130);
    supplied.data_mut().set_other(
        "minecraft:max_stack_size",
        Some(sunrise_core::nbt::Tag::Int(64)),
    );
    let mut random = LegacyRandom::new(31);
    let before = <(u64, Option<f64>)>::from(&random);
    let output = items.block_drops(
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
        output.iter().map(|stack| stack.count).collect::<Vec<_>>(),
        [64, 64, 2, 130]
    );
    assert_eq!(<(u64, Option<f64>)>::from(&random), before);
}

/// What a root pass's modifier sees of a roll: THIS_ENTITY's type, the tool's
/// item and the origin.
type Seen = (Option<String>, Option<String>, Option<[f64; 3]>);

/// Rolls `table` once through `roll_loot_table` with `this` (a type id) as
/// THIS_ENTITY, `tool` and `origin` as a caller supplies them. Returns the
/// number of stacks, and what a global modifier saw of the context.
fn roll_as_caller(
    table: Value,
    this: Option<&str>,
    tool: Option<&str>,
    origin: Option<[f64; 3]>,
) -> (usize, Seen) {
    use std::sync::Mutex;
    use sunrise_world::loot::{
        LootPlace, LootStack,
        global_modifiers::{
            DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile, ModifierRoll,
        },
        with_native_loot_place,
    };
    struct Watch(Arc<Mutex<Option<Seen>>>);
    impl ModifierApply for Watch {
        fn apply(
            &self,
            roll: &mut dyn ModifierRoll,
            generated: &[LootStack],
        ) -> std::result::Result<Vec<LootStack>, String> {
            let facts = roll.facts();
            *self.0.lock().unwrap() = Some((
                facts.entity.map(|e| e.kind.clone()),
                facts.tool.as_ref().map(|t| t.item.clone()),
                facts.place.as_ref().and_then(|p| p.origin),
            ));
            Ok(generated.to_vec())
        }
    }
    let seen = Arc::new(Mutex::new(None));
    let mut items = data(vec![("test:caller", table)]);
    let watch = seen.clone();
    items.global_modifiers = Arc::new(GlobalLootModifiers::load(
        [ModifierFile {
            id: "test:watch".into(),
            pack: "test".into(),
            path: "watch".into(),
            read: Box::new(|| Ok(br#"{"type":"test:watch"}"#.to_vec())),
        }],
        &sunrise_world::conditions::LoadConditions::vanilla(),
        move |_, _, _| {
            Ok(DecodedModifier {
                priority: 1,
                condition: None,
                apply: Arc::new(Watch(watch.clone())),
            })
        },
        |error| panic!("{error}"),
    ));
    let entity = this.map(sunrise_world::loot::LootEntity::new);
    let matcher = |predicate: &Value, entity: &sunrise_world::loot::LootEntity| {
        predicate["type"].as_str() == Some(&entity.kind)
    };
    let held = tool.map(|item| ItemStack::new(item, 1));
    let place = LootPlace {
        origin,
        ..Default::default()
    };
    let drops = with_native_loot_place(place, || {
        items
            .roll_loot_table(
                "test:caller",
                held.as_ref(),
                entity.as_ref().map(|e| (e, &matcher as &EntityMatcher)),
                &mut LegacyRandom::new(5),
            )
            .unwrap()
    });
    let seen = seen.lock().unwrap().take().expect("the modifier pass ran");
    (drops.len(), seen)
}
type EntityMatcher = dyn Fn(&Value, &sunrise_world::loot::LootEntity) -> bool;

fn table_for_this(kind: &str) -> Value {
    json!({"pools":[{"rolls":1,
        "conditions":[{"type":"minecraft:entity_properties","entity":"this","predicate":{"type":kind}}],
        "entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}]}]})
}

/// A mod's roll call names a real entity: its type reaches the conditions
/// (through the matcher) and the modifiers, with the tool and origin it gave.
#[test]
fn a_mod_roll_names_the_entity_it_was_given() {
    let (stacks, seen) = roll_as_caller(
        table_for_this("minecraft:cow"),
        Some("minecraft:cow"),
        Some("minecraft:shears"),
        Some([1.5, 64.0, -2.5]),
    );
    assert_eq!(stacks, 1);
    assert_eq!(
        seen,
        (
            Some("minecraft:cow".into()),
            Some("minecraft:shears".into()),
            Some([1.5, 64.0, -2.5])
        )
    );
    let (stacks, seen) = roll_as_caller(table_for_this("minecraft:cow"), None, None, None);
    assert_eq!(stacks, 0);
    assert_eq!(seen, (None, None, None));
}

/// `LootContextParamSets.PIGLIN_BARTER`: THIS_ENTITY is the piglin and nothing
/// else is supplied (no tool, no origin).
#[test]
fn a_barter_roll_names_the_piglin_and_nothing_else() {
    let (stacks, seen) = roll_as_caller(
        table_for_this("minecraft:piglin"),
        Some("minecraft:piglin"),
        None,
        None,
    );
    assert_eq!(stacks, 1);
    assert_eq!(seen, (Some("minecraft:piglin".into()), None, None));
}

/// `LootContextParamSets.ADVANCEMENT_REWARD`: THIS_ENTITY is the rewarded player
/// and the ORIGIN their position; no tool.
#[test]
fn an_advancement_reward_roll_names_the_player_and_their_position() {
    let (stacks, seen) = roll_as_caller(
        table_for_this("minecraft:player"),
        Some("minecraft:player"),
        None,
        Some([10.5, 70.0, -3.25]),
    );
    assert_eq!(stacks, 1);
    assert_eq!(
        seen,
        (
            Some("minecraft:player".into()),
            None,
            Some([10.5, 70.0, -3.25])
        )
    );
}

/// `Block.dropFromBlockInteractLootTable`: `entity_properties` on
/// `interacting_entity` reads the clicking player, and `this` (not a
/// `block_interact` parameter) stays unselected.
#[test]
fn an_interaction_drop_selects_the_interacting_entity() {
    let named = |who: &str| {
        json!({"pools":[{"rolls":1,
            "conditions":[{"type":"minecraft:entity_properties","entity":who,"predicate":{"type":"minecraft:player"}}],
            "entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}]}]})
    };
    let items = data(vec![
        ("test:interacting", named("interacting_entity")),
        ("test:this", named("this")),
    ]);
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
    let player = sunrise_world::loot::LootEntity::new("minecraft:player");
    let zombie = sunrise_world::loot::LootEntity::new("minecraft:zombie");
    let matcher: &EntityMatcher = &|predicate, entity| predicate["type"] == entity.kind;
    let roll = |id: &str, actor: Option<&sunrise_world::loot::LootEntity>| {
        items
            .interact_loot(
                id,
                &registry,
                BlockStateId(0),
                actor.map(|a| (a, matcher)),
                None,
                &mut LegacyRandom::new(0),
                &mut crate::random_sequences::RandomSequences::default(),
                0,
            )
            .len()
    };
    assert_eq!(roll("test:interacting", Some(&player)), 1);
    assert_eq!(roll("test:interacting", Some(&zombie)), 0);
    assert_eq!(roll("test:interacting", None), 0);
    assert_eq!(roll("test:this", Some(&player)), 0);
}

/// `copy_components` names its source as a `LootContext.EntityTarget`: the
/// clicking player's custom data reaches the drop, and an interaction without
/// that entity copies nothing.
#[test]
fn an_interaction_drop_copies_components_from_the_interacting_entity() {
    let items = data(vec![(
        "test:copy",
        json!({"pools":[{"rolls":1,"entries":[{
            "type":"minecraft:item","name":"minecraft:gold_ingot",
            "modifier":{"type":"minecraft:copy_components","source":"interacting_entity",
                "include":["minecraft:custom_data"]}}]}]}),
    )]);
    let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
    let mut player = sunrise_world::loot::LootEntity::new("minecraft:player");
    player.components.insert(
        "minecraft:custom_data".into(),
        sunrise_core::nbt::Tag::compound([("flag", sunrise_core::nbt::Tag::Long(7))]),
    );
    let matcher: &EntityMatcher = &|_, _| true;
    let roll = |actor: Option<&sunrise_world::loot::LootEntity>| {
        let mut stacks = items.interact_loot(
            "test:copy",
            &registry,
            BlockStateId(0),
            actor.map(|a| (a, matcher)),
            None,
            &mut LegacyRandom::new(0),
            &mut crate::random_sequences::RandomSequences::default(),
            0,
        );
        assert_eq!(stacks.len(), 1);
        crate::item_data::component_snapshot(&stacks.remove(0))
    };
    let copied = roll(Some(&player)).expect("the player's custom data was copied");
    assert_eq!(
        copied.get("minecraft:custom_data"),
        Some(&sunrise_core::nbt::Tag::compound([(
            "flag",
            sunrise_core::nbt::Tag::Long(7)
        )]))
    );
    assert_eq!(roll(None), None, "no interacting entity, nothing to copy");
}
