use super::*;
use serde_json::json;
use sunrise_core::{
    nbt::{Compound, Tag},
    random::LegacyRandom,
};
use sunrise_world::loot::{self, copy::BlockSource};

fn programmatic_tables(
    data: &loot::LootData,
    extra: &HashMap<String, Value>,
) -> Arc<loot::LootTables> {
    Arc::new(
        data.tables()
            .ids()
            .map(|id| (id.to_owned(), data.tables()[id].clone()))
            .chain(extra.iter().map(|(id, table)| (id.clone(), table.clone())))
            .collect(),
    )
}

#[test]
fn copy_components_uses_available_sources_and_table_filters_in_both_evaluators() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-copy-components-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let folder = root.join("data/test/loot_table");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        root.join("items.json"),
        json!({
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "test:tool":{"minecraft:max_stack_size":1,"minecraft:damage":0,
                "minecraft:unbreakable":{},"minecraft:enchantment_glint_override":true}
        })
        .to_string(),
    )
    .unwrap();
    let base = Tag::compound([("minecraft:oak_log", Tag::String("axis".into()))]);
    let block = BlockSource {
        nbt: Tag::compound([("id", Tag::String("test:source".into()))]),
        components: [
            ("test:keep".into(), Tag::ByteArray(vec![7, 252])),
            ("test:exclude".into(), Tag::Long(99)),
            ("test:ignored".into(), Tag::Int(8)),
        ]
        .into_iter()
        .collect(),
    };
    let potion = Tag::compound([("potion", Tag::String("minecraft:healing".into()))]);
    let tool_patch = Tag::compound([
        ("minecraft:damage", Tag::Int(8)),
        ("minecraft:potion_contents", potion.clone()),
        ("!minecraft:unbreakable", Tag::Compound(Compound::new())),
        ("test:keep", Tag::Long(17)),
    ]);
    let (damage, extra) = crate::item_data::decode_components(Some(&tool_patch));
    let held = ItemStack {
        item: Arc::from("test:tool"),
        count: 1,
        damage,
        extra,
    };
    let empty = loot::LootStack {
        item: String::new(),
        count: 1,
        damage: 0,
        enchantments: Vec::new(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components: Vec::new(),
    };
    let mut tool = loot::LootStack {
        item: "test:tool".into(),
        count: 1,
        damage: 8,
        ..empty.clone()
    };
    let Tag::Compound(patch) = &tool_patch else {
        unreachable!()
    };
    for (id, value) in patch {
        let id = id.to_string_lossy();
        tool.components
            .push(if let Some(base) = id.strip_prefix('!') {
                (base.to_owned(), None)
            } else {
                (id, Some(value.clone()))
            });
    }
    let mut entity = loot::LootEntity::new("test:entity");
    entity
        .components
        .insert("test:entity".into(), Tag::LongArray(vec![3, 4]));
    let cases = [
        (
            "intersection",
            json!({"source":"block_entity","include":["test:keep","test:exclude"],"exclude":["test:exclude"]}),
            true,
            false,
            false,
            vec![("test:keep", Tag::ByteArray(vec![7, 252]))],
        ),
        (
            "invalid_include_is_atomic",
            json!({"source":"block_entity","include":[7]}),
            true,
            false,
            false,
            vec![],
        ),
        (
            "empty_include",
            json!({"source":"block_entity","include":[]}),
            true,
            false,
            false,
            vec![],
        ),
        (
            "without_include",
            json!({"source":"block_entity","exclude":["test:exclude","test:ignored"]}),
            true,
            false,
            false,
            vec![("test:keep", Tag::ByteArray(vec![7, 252]))],
        ),
        (
            "missing_block",
            json!({"source":"block_entity"}),
            false,
            false,
            false,
            vec![],
        ),
        (
            "tool",
            json!({"source":"tool","include":["minecraft:damage","minecraft:potion_contents","minecraft:enchantment_glint_override","minecraft:unbreakable","test:keep"],"exclude":["test:keep"]}),
            false,
            true,
            false,
            vec![
                ("minecraft:damage", Tag::Int(8)),
                ("minecraft:potion_contents", potion),
                ("minecraft:enchantment_glint_override", Tag::Byte(1)),
            ],
        ),
        (
            "missing_tool",
            json!({"source":"tool"}),
            false,
            false,
            false,
            vec![],
        ),
        (
            "entity",
            json!({"source":"this","include":["test:entity"]}),
            false,
            false,
            true,
            vec![("test:entity", Tag::LongArray(vec![3, 4]))],
        ),
        (
            "entity_empty_custom_data",
            json!({"source":"this","include":["minecraft:custom_data"]}),
            false,
            false,
            true,
            vec![("minecraft:custom_data", Tag::Compound(Compound::new()))],
        ),
        (
            "missing_entity",
            json!({"source":"this"}),
            false,
            false,
            false,
            vec![],
        ),
        (
            "missing_attacker",
            json!({"source":"attacker"}),
            false,
            false,
            true,
            vec![],
        ),
    ];
    let mut programmatic = HashMap::new();
    for (name, function, _, _, _, _) in &cases {
        let mut function = function.clone();
        function["type"] = json!("minecraft:copy_components");
        let table = json!({"type":"minecraft:empty","pools":[{"rolls":1,"entries":[{
            "type":"minecraft:item","name":"minecraft:stick",
            "modifier":{"type":"minecraft:sequence","functions":[
                {"type":"minecraft:set_components","components":{"minecraft:debug_stick_state":{"minecraft:oak_log":"axis"}}}, function
            ]}
        }]}]});
        if *name == "invalid_include_is_atomic" {
            // Malformed arguments are an evaluator fallback case, not pack data.
            programmatic.insert(format!("test:{name}"), table);
        } else {
            std::fs::write(folder.join(format!("{name}.json")), table.to_string()).unwrap();
        }
    }
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let data = loot::LootData::load_with_types(
        &root,
        &loot::PackOverlay::NONE,
        &defaults,
        loot::types::Types::default(),
    )
    .unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.set_loot_data(&data);
    items.component_defaults = data.component_defaults().clone();
    items.loot = programmatic_tables(&data, &programmatic);
    for (name, _, has_block, has_tool, has_entity, copied) in cases {
        let mut expected: Compound = [("minecraft:debug_stick_state".into(), base.clone())]
            .into_iter()
            .collect();
        expected.extend(copied.into_iter().map(|(id, tag)| (id.into(), tag)));
        let id = format!("test:{name}");
        for item_engine in [false, true] {
            // A queried stack is deliberately not the TOOL.
            let queried = loot::LootStack {
                item: "test:not_the_tool".into(),
                count: 1,
                components: vec![("test:wrong".into(), Some(Tag::Int(42)))],
                ..empty.clone()
            };
            let query = std::rc::Rc::new(loot::query::LootQuery::new(
                Some(queried),
                has_tool.then(|| tool.clone()),
            ));
            let actual = loot::query::with_query(query, || {
                loot::copy::with_block_source(has_block.then_some(&block), || {
                    let mut random = LegacyRandom::new(1234);
                    if item_engine {
                        let predicate = |_: &Value, _: &loot::LootEntity| true;
                        let mut output = items
                            .roll_loot_table(
                                &id,
                                has_tool.then_some(&held),
                                has_entity.then_some((&entity, &predicate)),
                                &mut random,
                            )
                            .unwrap();
                        assert_eq!(output.len(), 1);
                        crate::item_data::component_snapshot(&output.remove(0)).unwrap()
                    } else {
                        let mut output = if has_entity {
                            data.roll_entity(
                                &id,
                                &mut random,
                                &loot::EntityLootParams {
                                    this: entity.clone(),
                                    ..Default::default()
                                },
                            )
                        } else if let Some(table) = programmatic.get(&id) {
                            data.roll_inline(table, &mut random)
                        } else {
                            data.roll(&id, &mut random, 0.0, None, None)
                        };
                        assert_eq!(output.len(), 1);
                        let mut patch = Compound::new();
                        for (id, value) in output.remove(0).components {
                            if let Some(value) = value {
                                patch.insert(id.as_str().into(), value);
                            } else {
                                patch.remove(&id.as_str().into());
                            }
                        }
                        Tag::Compound(patch)
                    }
                })
            });
            assert_eq!(
                actual,
                Tag::Compound(expected.clone()),
                "{name}, items = {item_engine}"
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_custom_data_runs_the_same_ordered_nbt_operations_in_both_evaluators() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-copy-custom-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let folder = root.join("data/test/loot_table");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        root.join("items.json"),
        r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
    )
    .unwrap();
    let base_text = "{keep:13L,list:[\"old\"],array:[I;5,6],dst:{nested:{old:1b}},same:3b,rows:[{id:\"a\"},{id:\"b\"}]}";
    let base = sunrise_core::nbt::parse_snbt(base_text).unwrap().0;
    let opaque = Tag::Compound(
        [
            (
                sunrise_core::nbt::JavaString(vec![0xd800]),
                Tag::LongArray(vec![i64::MIN, i64::MAX]),
            ),
            ("bytes".into(), Tag::ByteArray(vec![0, 255])),
        ]
        .into_iter()
        .collect(),
    );
    let source_tag = Tag::compound([
        (
            "picks",
            Tag::List(vec![
                Tag::compound([("id", Tag::String("a".into())), ("n", Tag::Byte(3))]),
                Tag::compound([("id", Tag::String("b".into())), ("n", Tag::Long(9))]),
            ]),
        ),
        ("values", Tag::List(vec![Tag::Int(3), Tag::Long(4)])),
        (
            "merge",
            Tag::List(vec![
                Tag::compound([("nested", Tag::compound([("new", Tag::Long(2))]))]),
                Tag::Int(7),
                Tag::compound([("last", Tag::ByteArray(vec![1, 255]))]),
            ]),
        ),
        ("ints", Tag::IntArray(vec![6, 7])),
        ("opaque", opaque.clone()),
        ("data", Tag::compound([("entity_flag", Tag::Long(7))])),
    ]);
    let source = BlockSource {
        nbt: source_tag.clone(),
        components: Default::default(),
    };
    let entity = loot::LootEntity {
        nbt: Some(Arc::new(source_tag.clone())),
        ..loot::LootEntity::new("test:entity")
    };
    let storage: Arc<loot::copy::StorageData> = Arc::new(
        [
            (
                "test".to_owned(),
                [("cache".to_owned(), source_tag)].into_iter().collect(),
            ),
            (
                "minecraft".to_owned(),
                [("cache".to_owned(), source.nbt.clone())]
                    .into_iter()
                    .collect(),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let op = |source: &str, target: &str, strategy: &str| json!({"source":source,"target":target,"op":strategy});
    // Expected SNBT is independent of the path writer and merge implementation.
    let deep_target = vec!["new"; 512].join(".");
    let cases = [
        (
            "depth_refusal_is_atomic",
            json!("block_entity"),
            vec![
                op("values[]", &deep_target, "append"),
                op("merge[]", &deep_target, "merge"),
            ],
            None,
        ),
        (
            "replace_last",
            json!("block_entity"),
            vec![op("picks[].n", "result", "replace")],
            Some("{result:9L}"),
        ),
        (
            "append_all",
            json!("block_entity"),
            vec![op("values[]", "list", "append")],
            Some("{list:[\"old\",3,4L]}"),
        ),
        (
            "append_array_refused",
            json!("block_entity"),
            vec![op("values[]", "array", "append")],
            None,
        ),
        (
            "append_created",
            json!("block_entity"),
            vec![op("values[]", "new.child", "append")],
            Some("{new:{child:[3,4L]}}"),
        ),
        (
            "merge_compounds",
            json!("block_entity"),
            vec![op("merge[]", "dst", "merge")],
            Some("{dst:{nested:{old:1b,new:2L},last:[B;1b,-1b]}}"),
        ),
        (
            "merge_created_without_compound",
            json!("block_entity"),
            vec![op("picks[].n", "made", "merge")],
            Some("{made:{}}"),
        ),
        (
            "merge_array_refused",
            json!("block_entity"),
            vec![op("merge[]", "array", "merge")],
            None,
        ),
        (
            "ordered",
            json!("block_entity"),
            vec![
                op("picks[].n", "dst", "replace"),
                op("merge[]", "dst", "merge"),
                op("picks[0].n", "list", "append"),
            ],
            Some("{dst:9L,list:[\"old\",3b]}"),
        ),
        (
            "selectors",
            json!("block_entity"),
            vec![
                op("picks[{id:b}].n", "rows[{id:b}].added", "replace"),
                op("ints[-1]", "tail", "replace"),
            ],
            Some("{rows:[{id:a},{id:b,added:9L}],tail:7}"),
        ),
        (
            "opaque",
            json!("block_entity"),
            vec![op("opaque", "opaque", "replace")],
            None,
        ),
        (
            "no_match",
            json!("block_entity"),
            vec![op("absent.field", "never.created", "replace")],
            None,
        ),
        (
            "same_value",
            json!("block_entity"),
            vec![op("picks[0].n", "same", "replace")],
            None,
        ),
        (
            "missing_block",
            json!("block_entity"),
            vec![op("picks[].n", "result", "replace")],
            None,
        ),
        (
            "entity",
            json!("this"),
            vec![op("picks[].n", "result", "replace")],
            Some("{result:9L}"),
        ),
        (
            "entity_object",
            json!({"type":"context","target":"this"}),
            vec![op("picks[].n", "result", "replace")],
            Some("{result:9L}"),
        ),
        (
            "missing_entity",
            json!("this"),
            vec![op("picks[].n", "result", "replace")],
            None,
        ),
        (
            "missing_attacker",
            json!("attacker"),
            vec![op("picks[].n", "result", "replace")],
            None,
        ),
        (
            "storage",
            json!({"type":"minecraft:storage","source":"test:cache"}),
            vec![op("picks[].n", "result", "replace")],
            Some("{result:9L}"),
        ),
        (
            "storage_default_namespace",
            json!({"type":"storage","source":":cache"}),
            vec![op("picks[].n", "result", "replace")],
            Some("{result:9L}"),
        ),
        (
            "unset_storage_is_empty",
            json!({"type":"storage","source":"test:unset"}),
            vec![op("{}", "empty", "replace")],
            Some("{empty:{}}"),
        ),
        (
            "component_then_custom",
            json!("this"),
            vec![op("picks[].n", "added", "replace")],
            Some("{entity_flag:7L,added:9L}"),
        ),
    ];
    for (name, provider, operations, _) in &cases {
        let mut functions = vec![
            json!({"type":"minecraft:set_components","components":{"minecraft:debug_stick_state":{"minecraft:oak_log":"axis"}}}),
            json!({"type":"minecraft:set_components","components":{"minecraft:custom_data":base_text}}),
        ];
        if *name == "component_then_custom" {
            functions.push(json!({"type":"minecraft:copy_components","source":"this","include":["minecraft:custom_data"]}));
        }
        functions
            .push(json!({"type":"minecraft:copy_custom_data","source":provider,"ops":operations}));
        let table = json!({"type":"minecraft:empty","pools":[{"rolls":1,"entries":[{
            "type":"minecraft:item","name":"minecraft:stick","modifier":{"type":"minecraft:sequence","functions":functions}
        }]}]});
        std::fs::write(folder.join(format!("{name}.json")), table.to_string()).unwrap();
    }
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let data = loot::LootData::load_with_types(
        &root,
        &loot::PackOverlay::NONE,
        &defaults,
        loot::types::Types::default(),
    )
    .unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.set_loot_data(&data);
    for (name, _, _, patch) in cases {
        let mut expected = if name == "component_then_custom" {
            Compound::new()
        } else {
            let Tag::Compound(base) = &base else {
                unreachable!()
            };
            base.clone()
        };
        if let Some(patch) = patch {
            let Tag::Compound(patch) = sunrise_core::nbt::parse_snbt(patch).unwrap().0 else {
                unreachable!()
            };
            expected.extend(patch);
        }
        if name == "opaque" {
            expected.insert("opaque".into(), opaque.clone());
        }
        let expected = Tag::compound([
            (
                "minecraft:debug_stick_state",
                Tag::compound([("minecraft:oak_log", Tag::String("axis".into()))]),
            ),
            ("minecraft:custom_data", Tag::Compound(expected)),
        ]);
        for item_engine in [false, true] {
            let output = loot::copy::with_storage(storage.clone(), || {
                loot::copy::with_block_source((name != "missing_block").then_some(&source), || {
                    let target = (name != "missing_entity").then_some(&entity);
                    let mut random = LegacyRandom::new(45);
                    if item_engine {
                        let predicate = |_: &Value, _: &loot::LootEntity| true;
                        let mut stacks = items
                            .roll_loot_table(
                                &format!("test:{name}"),
                                None,
                                target.map(|entity| {
                                    (
                                        entity,
                                        &predicate as &dyn Fn(&Value, &loot::LootEntity) -> bool,
                                    )
                                }),
                                &mut random,
                            )
                            .unwrap();
                        assert_eq!(stacks.len(), 1);
                        crate::item_data::component_snapshot(&stacks.remove(0)).unwrap()
                    } else {
                        let mut stacks = match target {
                            Some(entity) => data.roll_entity(
                                &format!("test:{name}"),
                                &mut random,
                                &loot::EntityLootParams {
                                    this: entity.clone(),
                                    ..Default::default()
                                },
                            ),
                            None => {
                                data.roll(&format!("test:{name}"), &mut random, 0.0, None, None)
                            }
                        };
                        assert_eq!(stacks.len(), 1);
                        let stack = stacks.remove(0);
                        assert_eq!(
                            stack
                                .components
                                .iter()
                                .filter(|(id, _)| id == "minecraft:custom_data")
                                .count(),
                            1
                        );
                        Tag::Compound(
                            stack
                                .components
                                .into_iter()
                                .filter_map(|(id, value)| {
                                    value.map(|value| (id.as_str().into(), value))
                                })
                                .collect(),
                        )
                    }
                })
            });
            assert_eq!(output, expected, "{name}, items = {item_engine}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_custom_data_respects_removed_prototype_data_in_both_evaluators() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-copy-removed-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let folder = root.join("data/test/loot_table");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        root.join("items.json"),
        r#"{"minecraft:stick":{"minecraft:max_stack_size":64,"minecraft:custom_data":{"old":1}}}"#,
    )
    .unwrap();
    let mut programmatic = HashMap::new();
    for (name, removal) in [
        ("null", json!({"minecraft:custom_data":null})),
        ("bang", json!({"!minecraft:custom_data":{}})),
        ("bang_null", json!({"!minecraft:custom_data":null})),
    ] {
        let table = json!({"type":"minecraft:empty","pools":[{"rolls":1,"entries":[{
            "type":"minecraft:item","name":"minecraft:stick",
            "modifier":{"type":"minecraft:sequence","functions":[
                {"type":"minecraft:set_components","components":removal},
                {"type":"minecraft:copy_custom_data","source":"block_entity","ops":[
                    {"source":"fresh","target":"new","op":"replace"}
                ]}
            ]}
        }]}]});
        if name == "bang" {
            std::fs::write(folder.join(format!("{name}.json")), table.to_string()).unwrap();
        }
        // Null forms exercise the permissive evaluator, never the pack codec.
        programmatic.insert(format!("test:{name}"), table);
    }
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let data = loot::LootData::load_with_types(
        &root,
        &loot::PackOverlay::NONE,
        &defaults,
        loot::types::Types::default(),
    )
    .unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.set_loot_data(&data);
    items.component_defaults = data.component_defaults().clone();
    items.loot = programmatic_tables(&data, &programmatic);
    let source = BlockSource {
        nbt: Tag::compound([("fresh", Tag::Long(7))]),
        components: Default::default(),
    };
    let expected = Tag::compound([("new", Tag::Long(7))]);
    for name in ["null", "bang", "bang_null"] {
        for item_engine in [false, true] {
            let actual = loot::copy::with_block_source(Some(&source), || {
                let mut random = LegacyRandom::new(45);
                let id = format!("test:{name}");
                if item_engine {
                    let mut output = items.roll_loot_table(&id, None, None, &mut random).unwrap();
                    assert_eq!(output.len(), 1);
                    Tag::Compound(output.remove(0).custom_data().unwrap().clone())
                } else {
                    let mut output = data.roll_inline(&programmatic[&id], &mut random);
                    assert_eq!(output.len(), 1);
                    output
                        .remove(0)
                        .components
                        .into_iter()
                        .find(|(id, _)| id == "minecraft:custom_data")
                        .unwrap()
                        .1
                        .unwrap()
                }
            });
            assert_eq!(actual, expected, "{name}, items = {item_engine}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn set_custom_data_preserves_nbt_types_and_nested_merges_in_both_evaluators() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-set-custom-data-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let folder = root.join("data/test/loot_table");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(root.join("items.json"), r#"{"minecraft:stick":{}}"#).unwrap();
    let table = json!({"type":"minecraft:empty","pools":[{"rolls":1,"entries":[{
        "type":"minecraft:item","name":"minecraft:stick",
        "modifier":{"type":"minecraft:sequence","functions":[
            {"type":"minecraft:set_components","components":{
                "minecraft:custom_data":"{nested:{left:3L}}"
            }},
            {"type":"minecraft:set_custom_data","tag":"{keep:13L,array:[I;5,6]}"},
            {"type":"minecraft:set_custom_data","tag":{"nested":{"right":4}}}
        ]}
    }]}]});
    std::fs::write(folder.join("typed.json"), table.to_string()).unwrap();
    let data = loot::LootData::load(
        &root,
        &loot::PackOverlay::NONE,
        &sunrise_world::item_defaults::read(&root).unwrap(),
    )
    .unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.set_loot_data(&data);
    let expected = Tag::compound([
        ("keep", Tag::Long(13)),
        ("array", Tag::IntArray(vec![5, 6])),
        (
            "nested",
            Tag::compound([("left", Tag::Long(3)), ("right", Tag::Byte(4))]),
        ),
    ]);
    for item_engine in [true, false] {
        let mut random = LegacyRandom::new(45);
        let actual = if item_engine {
            let mut output = items
                .roll_loot_table("test:typed", None, None, &mut random)
                .unwrap();
            assert_eq!(output.len(), 1);
            Tag::Compound(output.remove(0).custom_data().unwrap().clone())
        } else {
            let mut output = data.roll("test:typed", &mut random, 0.0, None, None);
            assert_eq!(output.len(), 1);
            output
                .remove(0)
                .components
                .into_iter()
                .find(|(id, _)| id == "minecraft:custom_data")
                .unwrap()
                .1
                .unwrap()
        };
        assert_eq!(actual, expected, "items = {item_engine}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn set_custom_data_removes_empty_effective_results_in_both_evaluators() {
    let root = std::env::temp_dir().join(format!("sunrise-set-empty-{}", std::process::id()));
    let folder = root.join("data/test/loot_table");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        root.join("items.json"),
        r#"{
        "minecraft:stick":{},
        "minecraft:paper":{"minecraft:custom_data":{}},
        "minecraft:book":{"minecraft:custom_data":{"old":1}}
    }"#,
    )
    .unwrap();
    for (name, item, remove) in [
        ("absent", "minecraft:stick", false),
        ("empty", "minecraft:paper", false),
        ("removed", "minecraft:book", true),
        ("retained", "minecraft:book", false),
    ] {
        let mut functions = Vec::new();
        if remove {
            functions.push(json!({"type":"minecraft:set_components",
                "components":{"!minecraft:custom_data":{}}}));
        }
        functions.push(json!({"type":"minecraft:set_custom_data","tag":{}}));
        let table = json!({"pools":[{"rolls":1,"entries":[{
            "type":"minecraft:item","name":item,
            "modifier":{"type":"minecraft:sequence","functions":functions}
        }]}]});
        std::fs::write(folder.join(format!("{name}.json")), table.to_string()).unwrap();
    }
    let data = loot::LootData::load(
        &root,
        &loot::PackOverlay::NONE,
        &sunrise_world::item_defaults::read(&root).unwrap(),
    )
    .unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.set_loot_data(&data);
    items.component_defaults = data.component_defaults().clone();
    for name in ["absent", "empty", "removed", "retained"] {
        let expected = (name == "retained").then(|| Tag::compound([("old", Tag::Byte(1))]));
        for item_engine in [false, true] {
            let mut random = LegacyRandom::new(45);
            let id = format!("test:{name}");
            let mut components = if item_engine {
                let output = items.roll_loot_table(&id, None, None, &mut random).unwrap();
                assert_eq!(output.len(), 1);
                loot::copy::components_with_patch(
                    data.component_defaults().get(output[0].item.as_ref()),
                    crate::item_data::component_snapshot(&output[0]).as_ref(),
                )
            } else {
                let output = data.roll(&id, &mut random, 0.0, None, None);
                assert_eq!(output.len(), 1);
                loot::copy::stack_components(
                    &output[0],
                    data.component_defaults().get(&output[0].item),
                )
            };
            assert_eq!(
                components.remove(&"minecraft:custom_data".into()),
                expected,
                "{name}, items = {item_engine}"
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn set_custom_data_merges_effective_prototypes_and_respects_removal_in_both_evaluators() {
    let root = std::env::temp_dir().join(format!("sunrise-set-prototype-{}", std::process::id()));
    let folder = root.join("data/test/loot_table");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        root.join("items.json"),
        r#"{"minecraft:stick":{"minecraft:custom_data":{"old":1}}}"#,
    )
    .unwrap();
    let mut programmatic = HashMap::new();
    for (name, removal) in [
        ("prototype", None),
        ("removed", Some(json!({"!minecraft:custom_data":{}}))),
        ("null", Some(json!({"minecraft:custom_data":null}))),
    ] {
        let mut functions = Vec::new();
        if let Some(removal) = removal {
            functions.push(json!({"type":"minecraft:set_components","components":removal}));
        }
        functions.push(json!({"type":"minecraft:set_custom_data","tag":"{fresh:2L}"}));
        // A second update must see the newly written value, not a stale removal marker.
        functions.push(json!({"type":"minecraft:set_custom_data","tag":"{later:3L}"}));
        let table = json!({"pools":[{"rolls":1,"entries":[{
            "type":"minecraft:item","name":"minecraft:stick",
            "modifier":{"type":"minecraft:sequence","functions":functions}
        }]}]});
        if name != "null" {
            std::fs::write(folder.join(format!("{name}.json")), table.to_string()).unwrap();
        }
        // Null forms exercise the permissive evaluator, never the pack codec.
        programmatic.insert(format!("test:{name}"), table);
    }
    let data = loot::LootData::load(
        &root,
        &loot::PackOverlay::NONE,
        &sunrise_world::item_defaults::read(&root).unwrap(),
    )
    .unwrap();
    let mut items = Items::for_tests(HashMap::new());
    items.set_loot_data(&data);
    items.component_defaults = data.component_defaults().clone();
    items.loot = programmatic_tables(&data, &programmatic);
    for name in ["prototype", "removed", "null"] {
        let expected = if name == "prototype" {
            Tag::compound([
                ("old", Tag::Byte(1)),
                ("fresh", Tag::Long(2)),
                ("later", Tag::Long(3)),
            ])
        } else {
            Tag::compound([("fresh", Tag::Long(2)), ("later", Tag::Long(3))])
        };
        for item_engine in [false, true] {
            let mut random = LegacyRandom::new(45);
            let id = format!("test:{name}");
            let actual = if item_engine {
                let mut output = items.roll_loot_table(&id, None, None, &mut random).unwrap();
                assert_eq!(output.len(), 1);
                Tag::Compound(output.remove(0).custom_data().unwrap().clone())
            } else {
                let mut output = data.roll_inline(&programmatic[&id], &mut random);
                assert_eq!(output.len(), 1);
                output
                    .remove(0)
                    .components
                    .into_iter()
                    .find(|(id, _)| id == "minecraft:custom_data")
                    .unwrap()
                    .1
                    .unwrap()
            };
            assert_eq!(actual, expected, "{name}, items = {item_engine}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
