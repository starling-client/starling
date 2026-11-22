use super::*;
use serde_json::json;
use std::rc::Rc;
fn data(tables: Vec<(&str, Value)>) -> LootData {
    LootData {
        holder_sets: Default::default(),
        component_defaults: Arc::default(),
        copies_entity_nbt: false,
        types: types::Types::capture(),
        tables: Arc::new(tables.into_iter().map(|(k, v)| (k.into(), v)).collect()),
        global_modifiers: Arc::default(),
        modifiers: HashMap::new(),
        predicates: Arc::default(),
        items: HashMap::new(),
        enchantments: vec![],
        primary_answers: Default::default(),
        support_answers: Default::default(),
        tags: HashMap::new(),
        smelting: vec![],
    }
}
fn table(id: &str) -> Value {
    json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:loot_table_id","loot_table_id":id},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]})
}
/// Non-raw adapters preserve the raw draw stream and split only after the
/// outer table's function. A nested table must not complete the shared query.
#[test]
fn root_adapters_preserve_output_and_random_without_modifiers() {
    let nested = json!({"pools":[{"rolls":1,
        "condition":{"type":"neoforge:loot_table_id","loot_table_id":"neoforge:unknown_loot_table"},
        "entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"},
                   {"type":"minecraft:item","name":"minecraft:diamond"}]}],
        "modifier":{"type":"minecraft:set_count","count":200}});
    let root = json!({"pools":[{"rolls":1,"entries":[
        {"type":"minecraft:loot_table","value":"test:nested"}]}],
        "modifier":{"type":"minecraft:set_count","count":130}});
    let data = data(vec![("test:root", root.clone()), ("test:nested", nested)]);
    // One weighted choice in the child, no draws for either constant count.
    let mut expected_random = LegacyRandom::new(0);
    assert_eq!(expected_random.next_int_bound(2).unwrap(), 1);
    let expected_state = <(u64, Option<f64>)>::from(&expected_random);
    for adapter in 0..6 {
        let query = Rc::new(query::LootQuery::default());
        let mut random = LegacyRandom::new(0);
        let params = EntityLootParams::default();
        let drops = query::with_query(query.clone(), || match adapter {
            0 => data.roll("test:root", &mut random, 0.0, None, None),
            1 => data.roll_inline(&root, &mut random),
            2 => data.roll_entity("test:root", &mut random, &params),
            3 => data.roll_death_entity("test:root", &mut random, &params, &mut |_, _, _| {
                panic!("the fixture has no enchanted-level reads")
            }),
            4 => data
                .roll_container_with("test:root", &mut random, Default::default())
                .unwrap(),
            _ => data.roll_split("test:root", &mut random),
        });
        let observed: Vec<_> = drops.iter().map(|s| (s.item.as_str(), s.count)).collect();
        assert_eq!(
            observed,
            [
                ("minecraft:diamond", 64),
                ("minecraft:diamond", 64),
                ("minecraft:diamond", 2)
            ],
            "adapter {adapter}"
        );
        assert_eq!(
            <(u64, Option<f64>)>::from(&random),
            expected_state,
            "adapter {adapter}"
        );
        assert_eq!(
            query.id().as_deref(),
            Some(if adapter == 1 {
                "neoforge:unknown_loot_table"
            } else {
                "test:root"
            })
        );
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
            data.roll(
                "minecraft:blocks/short_grass",
                &mut LegacyRandom::new(7),
                0.0,
                None,
                None,
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
        data.roll(
            "minecraft:chests/simple_dungeon",
            &mut LegacyRandom::new(9),
            0.0,
            None,
            None,
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "minecraft:chests/simple_dungeon");
    let query = Rc::new(query::LootQuery::default());
    let drops = query::with_query(query.clone(), || {
        data.roll_inline(
            &table("neoforge:unknown_loot_table"),
            &mut LegacyRandom::new(9),
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
    let query = Rc::new(query::LootQuery::default());
    let inline = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":table("neoforge:unknown_loot_table")}]}]});
    let drops = query::with_query(query.clone(), || {
        data.roll_inline(&inline, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
}

#[test]
fn queried_context_and_ability_reach_native_adapter() {
    crate::native_load_hooks::testing::install();
    let stack = || {
        let mut s = LootStack::new("minecraft:stick");
        s.count = 3;
        s
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
            let drops = query::with_query(query, || {
                data.roll("test:a", &mut LegacyRandom::new(9), 0.0, None, None)
            });
            assert_eq!(
                crate::native_load_hooks::testing::take_ability(),
                tool.map(|item| (item.to_owned(), ability.to_owned()))
            );
            assert_eq!(
                drops.len(),
                if ability == "shears_dig" { expected } else { 0 }
            );
        }
    }
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
        data.roll("test:a", &mut LegacyRandom::new(9), 0.0, None, None)
    });
    assert_eq!(drops.len(), 1);
    let (id, stack) = crate::native_load_hooks::testing::take_context().expect("query callback");
    assert_eq!(id, "minecraft:blocks/short_grass");
    let stack = stack.expect("queried stack");
    assert_eq!((&*stack.item, stack.count), ("minecraft:stick", 3));
}

#[test]
#[ignore = "daily"]
fn queried_ability_tags_use_prepared_world_sets() {
    crate::native_load_hooks::testing::install();
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .or_else(|| std::env::var_os("SUNRISE_RESOURCES"))
            .expect("prepared resources"),
    );
    let defaults = crate::item_defaults::read(&root).unwrap();
    let table = json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:can_item_perform_ability","ability":"sword_sweep"},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let data = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
    let query = Rc::new(query::LootQuery::new(
        None,
        Some(LootStack::new("minecraft:diamond_sword")),
    ));
    let drops = query::with_query(query, || {
        data.roll_inline(&table, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
    assert!(data.contains(
        "item",
        "minecraft:douses_campfires",
        "minecraft:diamond_shovel"
    ));
    assert!(!data.contains("item", "minecraft:swords", "minecraft:diamond_axe"));
    let files = [(
        "tags/minecraft:item/swords".to_owned(),
        vec![serde_json::to_vec(&json!({"replace":true,"values":["minecraft:stick"]})).unwrap()],
    )];
    let reloaded = LootData::load(&root, &PackOverlay::new(&files), &defaults).unwrap();
    assert!(!reloaded.contains("item", "minecraft:swords", "minecraft:diamond_sword"));
    assert!(reloaded.contains("item", "minecraft:swords", "minecraft:stick"));
    assert!(data.contains("item", "minecraft:swords", "minecraft:diamond_sword"));
}

#[test]
fn modifiers_run_once_after_split_and_raw_subrolls_share_random_and_guards() {
    use global_modifiers::{
        DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile, ModifierRoll,
    };
    struct Observe;
    impl ModifierApply for Observe {
        fn apply(
            &self,
            roll: &mut dyn ModifierRoll,
            generated: &[LootStack],
        ) -> Result<Vec<LootStack>, String> {
            assert_eq!(
                generated
                    .iter()
                    .map(|stack| stack.count)
                    .collect::<Vec<_>>(),
                [64, 64, 2]
            );
            assert!(matches!(
                roll.facts().table_id(),
                "test:root" | "neoforge:unknown_loot_table"
            ));
            assert_eq!(roll.next_long(), LegacyRandom::new(31).next_long());
            let raw = roll.roll_raw("test:raw")?;
            assert_eq!(raw.len(), 1);
            assert_eq!(raw[0].count, 200); // raw never runs this pass or the splitter
            let mut empty = generated[0].clone();
            empty.count = -3;
            empty
                .components
                .push(("minecraft:custom_data".into(), Some(Tag::Int(42))));
            Ok(vec![raw[0].clone(), empty])
        }
    }
    struct Refuse;
    impl ModifierApply for Refuse {
        fn apply(
            &self,
            roll: &mut dyn ModifierRoll,
            _: &[LootStack],
        ) -> Result<Vec<LootStack>, String> {
            roll.next_long();
            roll.roll_raw("test:missing")
        }
    }
    struct Skipped;
    impl ModifierApply for Skipped {
        fn apply(
            &self,
            _: &mut dyn ModifierRoll,
            _: &[LootStack],
        ) -> Result<Vec<LootStack>, String> {
            panic!("a false base condition must skip application")
        }
    }
    let root = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}],
        "modifier":{"type":"minecraft:set_count","count":130}});
    let raw = json!({"pools":[
        {"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:raw"}]},
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot","modifier":{"type":"minecraft:set_count","count":200}}]}
    ]});
    let mut data = data(vec![("test:root", root.clone()), ("test:raw", raw)]);
    data.set_global_modifiers(Arc::new(GlobalLootModifiers::load(
        ["test:observe", "test:refuse", "test:skip"]
            .into_iter()
            .map(|id| ModifierFile {
                id: id.into(),
                pack: "test".into(),
                path: format!("{id}.json"),
                read: Box::new(|| Ok(br#"{"type":"test:observe"}"#.to_vec())),
            }),
        &crate::conditions::LoadConditions::vanilla(),
        |id, _, _| {
            let (priority, condition, apply): (_, _, Arc<dyn ModifierApply>) = match id {
                "test:observe" => (1001, None, Arc::new(Observe)),
                "test:refuse" => (1000, None, Arc::new(Refuse)),
                _ => (
                    999,
                    Some(json!({"type":"neoforge:loot_table_id","loot_table_id":"test:never"})),
                    Arc::new(Skipped),
                ),
            };
            Ok(DecodedModifier {
                priority,
                condition,
                apply,
            })
        },
        |error| panic!("{error}"),
    )));
    let mut expected_random = LegacyRandom::new(31);
    expected_random.next_long();
    expected_random.next_long();
    for adapter in 0..6 {
        let mut random = LegacyRandom::new(31);
        let params = EntityLootParams::default();
        let output = match adapter {
            0 => data.roll("test:root", &mut random, 0.0, None, None),
            1 => data.roll_inline(&root, &mut random),
            2 => data.roll_entity("test:root", &mut random, &params),
            3 => data.roll_death_entity("test:root", &mut random, &params, &mut |_, _, _| {
                panic!("no enchantments")
            }),
            4 => data
                .roll_container_with("test:root", &mut random, Default::default())
                .unwrap(),
            _ => data.roll_split("test:root", &mut random),
        };
        assert_eq!(
            output.iter().map(|stack| stack.count).collect::<Vec<_>>(),
            [200, -3],
            "adapter {adapter}"
        );
        assert_eq!(
            output[1].components,
            [("minecraft:custom_data".into(), Some(Tag::Int(42)))]
        );
        assert_eq!(
            <(u64, Option<f64>)>::from(&random),
            <(u64, Option<f64>)>::from(&expected_random)
        );
    }
}

/// What a root pass's modifier sees of a roll: THIS_ENTITY's type, the tool's
/// item, the origin and the luck.
type Seen = (Option<String>, Option<String>, Option<[f64; 3]>, f32);

/// A `LootData` with `table` as `test:caller` and a modifier that records the
/// facts of the one root pass.
fn watched(table: Value, raw: Option<Value>) -> (LootData, Arc<std::sync::Mutex<Option<Seen>>>) {
    use global_modifiers::{
        DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile, ModifierRoll,
    };
    struct Watch(Arc<std::sync::Mutex<Option<Seen>>>, bool);
    impl ModifierApply for Watch {
        fn apply(
            &self,
            roll: &mut dyn ModifierRoll,
            generated: &[LootStack],
        ) -> Result<Vec<LootStack>, String> {
            let facts = roll.facts();
            *self.0.lock().unwrap() = Some((
                facts.entity.map(|e| e.kind.clone()),
                facts.tool.as_ref().map(|t| t.item.clone()),
                facts.place.as_ref().and_then(|p| p.origin),
                facts.luck,
            ));
            let mut out = generated.to_vec();
            if self.1 {
                out.extend(roll.roll_raw("test:raw")?);
            }
            Ok(out)
        }
    }
    let seen = Arc::new(std::sync::Mutex::new(None));
    let has_raw = raw.is_some();
    let mut tables = vec![("test:caller", table)];
    tables.extend(raw.map(|raw| ("test:raw", raw)));
    let mut data = data(tables);
    let watch = seen.clone();
    data.set_global_modifiers(Arc::new(GlobalLootModifiers::load(
        [ModifierFile {
            id: "test:watch".into(),
            pack: "test".into(),
            path: "watch.json".into(),
            read: Box::new(|| Ok(br#"{"type":"test:watch"}"#.to_vec())),
        }],
        &crate::conditions::LoadConditions::vanilla(),
        move |_, _, _| {
            Ok(DecodedModifier {
                priority: 1,
                condition: None,
                apply: Arc::new(Watch(watch.clone(), has_raw)),
            })
        },
        |error| panic!("{error}"),
    )));
    (data, seen)
}

/// Runs `roll` with `tool` as the query's tool and `origin` as the place.
fn as_caller<T>(tool: Option<&str>, origin: [f64; 3], roll: impl FnOnce() -> T) -> T {
    let tool = tool.map(LootStack::new);
    let place = LootPlace {
        origin: Some(origin),
        ..Default::default()
    };
    query::with_query(Rc::new(query::LootQuery::new(None, tool)), || {
        with_native_loot_place(place, roll)
    })
}

/// `LootContextParamSets.FISHING`: the hook is THIS_ENTITY and its owner the
/// attacking entity, the rod the TOOL, the hook's position the ORIGIN, and the
/// luck what the hook and its owner add. A hook predicate keeps reading whether
/// the hook is in open water, and a raw sub-roll sees the same entities.
#[test]
fn a_fishing_roll_names_the_hook_its_owner_the_rod_and_the_position() {
    let pool = |target: &str, predicate: Value, item: &str| {
        json!({"rolls":1,
            "condition":{"type":"minecraft:entity_properties","entity":target,"predicate":predicate},
            "entries":[{"type":"minecraft:item","name":item}]})
    };
    let hook = |item| {
        pool(
            "this",
            json!({"minecraft:entity_type":"minecraft:fishing_bobber"}),
            item,
        )
    };
    let owner = |item| {
        pool(
            "attacker",
            json!({"minecraft:entity_type":"minecraft:player"}),
            item,
        )
    };
    let table = json!({"pools":[
        pool("this", json!({"minecraft:type_specific/fishing_hook":{"in_open_water":true}}), "minecraft:pufferfish"),
        hook("minecraft:cod"),
        owner("minecraft:name_tag"),
        // The hook is not the attacker, and the owner is not THIS_ENTITY.
        pool("attacker", json!({"minecraft:entity_type":"minecraft:fishing_bobber"}), "minecraft:bone"),
        pool("this", json!({"minecraft:entity_type":"minecraft:player"}), "minecraft:stick"),
    ]});
    let raw = json!({"pools":[hook("minecraft:lily_pad"), owner("minecraft:saddle")]});
    let (data, seen) = watched(table, Some(raw));
    let params = |owner: Option<LootEntity>| EntityLootParams {
        this: LootEntity {
            living: false,
            ..LootEntity::new("minecraft:fishing_bobber")
        },
        luck: 2.0,
        attacker: owner,
        ..Default::default()
    };
    for (open, owner, expected) in [
        (
            true,
            Some(LootEntity::new("minecraft:player")),
            vec![
                "minecraft:pufferfish",
                "minecraft:cod",
                "minecraft:name_tag",
                "minecraft:lily_pad",
                "minecraft:saddle",
            ],
        ),
        (
            false,
            Some(LootEntity::new("minecraft:player")),
            vec![
                "minecraft:cod",
                "minecraft:name_tag",
                "minecraft:lily_pad",
                "minecraft:saddle",
            ],
        ),
        (false, None, vec!["minecraft:cod", "minecraft:lily_pad"]),
    ] {
        let drops = as_caller(Some("minecraft:fishing_rod"), [4.5, 62.0, 9.5], || {
            data.roll_as(
                "test:caller",
                &mut LegacyRandom::new(3),
                None,
                Some(open),
                &params(owner),
            )
        });
        assert_eq!(
            drops.iter().map(|d| d.item.as_str()).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            seen.lock().unwrap().take(),
            Some((
                Some("minecraft:fishing_bobber".into()),
                Some("minecraft:fishing_rod".into()),
                Some([4.5, 62.0, 9.5]),
                2.0
            ))
        );
    }
}

/// `LootContextParamSets.ARCHAEOLOGY`: the brusher is THIS_ENTITY, the brush
/// the TOOL, the block's centre the ORIGIN, with the brusher's luck.
#[test]
fn a_brushing_roll_names_the_brusher_the_brush_and_the_block() {
    let table = json!({"pools":[{"rolls":1,"entries":[
        {"type":"minecraft:item","name":"minecraft:pottery_sherd_arms_up"}]}]});
    let (data, seen) = watched(table, None);
    let brusher = LootEntity::new("minecraft:player");
    let slots = as_caller(Some("minecraft:brush"), [8.5, 64.5, -7.5], || {
        data.fill_in(
            "test:caller",
            &mut LegacyRandom::new(3),
            1,
            fill::ContainerLootContext {
                luck: 1.5,
                entity: Some(&brusher),
                ..Default::default()
            },
        )
    });
    assert_eq!(
        slots[0].as_ref().map(|s| s.item.as_str()),
        Some("minecraft:pottery_sherd_arms_up")
    );
    assert_eq!(
        seen.lock().unwrap().take(),
        Some((
            Some("minecraft:player".into()),
            Some("minecraft:brush".into()),
            Some([8.5, 64.5, -7.5]),
            1.5
        ))
    );
}

/// `LootContextParamSets.VAULT`: the reward names the player, the inserted
/// key as the TOOL, the vault's centre as the ORIGIN and the player's luck;
/// the display roll supplies the origin alone.
#[test]
fn a_vault_reward_names_the_player_the_key_and_the_vault() {
    let table = json!({"pools":[{"rolls":1,"entries":[
        {"type":"minecraft:item","name":"minecraft:emerald"}]}]});
    let (data, seen) = watched(table, None);
    let player = LootEntity::new("minecraft:player");
    as_caller(Some("minecraft:trial_key"), [0.5, 70.5, 0.5], || {
        data.roll_container_with(
            "test:caller",
            &mut LegacyRandom::new(3),
            fill::ContainerLootContext {
                luck: 0.5,
                entity: Some(&player),
                ..Default::default()
            },
        )
    })
    .unwrap();
    assert_eq!(
        seen.lock().unwrap().take(),
        Some((
            Some("minecraft:player".into()),
            Some("minecraft:trial_key".into()),
            Some([0.5, 70.5, 0.5]),
            0.5
        ))
    );
    as_caller(None, [0.5, 70.5, 0.5], || {
        data.roll_split("test:caller", &mut LegacyRandom::new(3))
    });
    assert_eq!(
        seen.lock().unwrap().take(),
        Some((None, None, Some([0.5, 70.5, 0.5]), 0.0))
    );
}
