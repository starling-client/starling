use super::*;
use serde_json::json;

#[test]
fn malformed_builtin_arguments_refuse_with_resource_field_and_type() {
    for (function, data, field) in [
        (true, json!({"type":"minecraft:set_count"}), "count"),
        (
            true,
            json!({"type":"minecraft:set_count","count":true}),
            "count",
        ),
        (
            true,
            json!({"type":"minecraft:set_item","item":"minecraft:air"}),
            "item",
        ),
        (
            true,
            json!({"type":"minecraft:enchant_with_levels","levels":false}),
            "levels",
        ),
        (
            true,
            json!({"type":"minecraft:enchant_randomly","options":3}),
            "options",
        ),
        (
            true,
            json!({"type":"minecraft:set_enchantments","enchantments":{"bad id":1}}),
            "enchantments",
        ),
        (
            true,
            json!({"type":"minecraft:set_custom_data","tag":[]}),
            "tag",
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"minecraft:max_damage":"bad"}}),
            "components",
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"minecraft:custom_data":null}}),
            "components",
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"!minecraft:custom_data":null}}),
            "components",
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"minecraft:creative_slot_lock":{}}}),
            "components",
        ),
        (
            true,
            json!({"type":"minecraft:furnace_smelt","use_input_count":1}),
            "use_input_count",
        ),
        (
            true,
            json!({"type":"minecraft:enchanted_count_increase","enchantment":"test:e","count":1,"limit":"bad"}),
            "limit",
        ),
        (
            true,
            json!({"type":"minecraft:set_damage","damage":false}),
            "damage",
        ),
        (
            true,
            json!({"type":"minecraft:set_attributes","modifiers":[{}]}),
            "modifiers",
        ),
        (true, json!({"type":"minecraft:set_name","name":7}), "name"),
        (
            true,
            json!({"type":"minecraft:set_name","name":{"text":"x","bold":3}}),
            "name",
        ),
        (
            true,
            json!({"type":"minecraft:exploration_map","destination":"#test:places","search_radius":"bad"}),
            "search_radius",
        ),
        (
            true,
            json!({"type":"minecraft:set_stew_effect","effects":[{"type":"test:e","duration":false}]}),
            "duration",
        ),
        (
            true,
            json!({"type":"minecraft:copy_name","source":"not_a_source"}),
            "source",
        ),
        (
            true,
            json!({"type":"minecraft:set_contents","component":"container","entries":[{"type":"minecraft:item"}]}),
            "entries",
        ),
        (
            true,
            json!({"type":"minecraft:set_contents","component":"container","entries":[{"type":"minecraft:loot_table","value":{"pools":[{"entries":[],"rolls":false}]}}]}),
            "rolls",
        ),
        (
            true,
            json!({"type":"minecraft:set_contents","component":"container","entries":[{"type":"minecraft:slots","slot_source":{"type":"minecraft:limit_slots","slot_source":{"type":"minecraft:empty"},"limit":0}}]}),
            "limit",
        ),
        (
            true,
            json!({"type":"minecraft:modify_contents","component":"not_a_component","modifier":[]}),
            "component",
        ),
        (
            true,
            json!({"type":"minecraft:filtered","item_filter":false}),
            "item_filter",
        ),
        (
            true,
            json!({"type":"minecraft:limit_count","limit":{"min":false}}),
            "limit",
        ),
        (
            true,
            json!({"type":"minecraft:apply_bonus","enchantment":"test:e","formula":"uniform_bonus_count","parameters":{"bonusMultiplier":false}}),
            "bonusMultiplier",
        ),
        (
            true,
            json!({"type":"minecraft:set_loot_table","loot_table_id":true}),
            "loot_table_id",
        ),
        (
            true,
            json!({"type":"minecraft:set_lore","lore":[false],"mode":"append"}),
            "lore",
        ),
        (
            true,
            json!({"type":"minecraft:fill_player_head","entity":"tool"}),
            "entity",
        ),
        (
            true,
            json!({"type":"minecraft:copy_custom_data","source":"block_entity","ops":[{"source":"a[","target":"b","op":"replace"}]}),
            "source",
        ),
        (
            true,
            json!({"type":"minecraft:copy_state","block":"test:block","properties":[false]}),
            "properties",
        ),
        (
            true,
            json!({"type":"minecraft:set_banner_pattern","patterns":[],"append":7}),
            "append",
        ),
        (true, json!({"type":"minecraft:set_potion","id":7}), "id"),
        (
            true,
            json!({"type":"minecraft:set_random_dyes","number_of_dyes":false}),
            "number_of_dyes",
        ),
        (
            true,
            json!({"type":"minecraft:set_random_potion","options":[false]}),
            "options",
        ),
        (
            true,
            json!({"type":"minecraft:set_instrument","options":false}),
            "options",
        ),
        (
            true,
            json!({"type":"minecraft:sequence","functions":false}),
            "functions",
        ),
        (
            true,
            json!({"type":"minecraft:sequence","functions":[[]]}),
            "functions",
        ),
        (
            true,
            json!({"type":"minecraft:copy_components","source":"block_entity","include":[7]}),
            "include",
        ),
        (
            true,
            json!({"type":"minecraft:set_fireworks","flight_duration":"bad"}),
            "flight_duration",
        ),
        (
            true,
            json!({"type":"minecraft:set_firework_explosion","shape":"not_a_shape"}),
            "shape",
        ),
        (
            true,
            json!({"type":"minecraft:set_book_cover","generation":4}),
            "generation",
        ),
        (
            true,
            json!({"type":"minecraft:set_written_book_pages","pages":[false],"mode":"append"}),
            "pages",
        ),
        (
            true,
            json!({"type":"minecraft:set_writable_book_pages","pages":[false],"mode":"append"}),
            "pages",
        ),
        (
            true,
            json!({"type":"minecraft:toggle_tooltips","toggles":{"test:x":1}}),
            "toggles",
        ),
        (
            true,
            json!({"type":"minecraft:set_ominous_bottle_amplifier","amplifier":false}),
            "amplifier",
        ),
        (
            true,
            json!({"type":"minecraft:set_custom_model_data","flags":{"values":[7],"mode":"append"}}),
            "flags",
        ),
        (
            false,
            json!({"type":"minecraft:inverted","term":false}),
            "term",
        ),
        (
            false,
            json!({"type":"minecraft:any_of","terms":false}),
            "terms",
        ),
        (
            false,
            json!({"type":"minecraft:all_of","terms":[[]]}),
            "terms",
        ),
        (
            false,
            json!({"type":"minecraft:random_chance","chance":false}),
            "chance",
        ),
        (
            false,
            json!({"type":"minecraft:random_chance_with_enchanted_bonus","unenchanted_chance":2,"enchanted_chance":1,"enchantment":"test:e"}),
            "unenchanted_chance",
        ),
        (
            false,
            json!({"type":"minecraft:entity_properties","entity":"tool"}),
            "entity",
        ),
        (
            false,
            json!({"type":"minecraft:entity_scores","entity":"this","scores":{"x":false}}),
            "scores",
        ),
        (
            false,
            json!({"type":"minecraft:match_block","blocks":false}),
            "blocks",
        ),
        (
            false,
            json!({"type":"minecraft:match_tool","predicate":{"count":false}}),
            "count",
        ),
        (
            false,
            json!({"type":"minecraft:table_bonus","enchantment":"test:e","chances":[]}),
            "chances",
        ),
        (
            false,
            json!({"type":"minecraft:damage_source_properties","predicate":{"tags":false}}),
            "tags",
        ),
        (
            false,
            json!({"type":"minecraft:location_check","offsetX":"bad"}),
            "offsetX",
        ),
        (
            false,
            json!({"type":"minecraft:weather_check","raining":1}),
            "raining",
        ),
        (
            false,
            json!({"type":"minecraft:time_check","clock":"test:clock","value":{},"period":"bad"}),
            "period",
        ),
        (
            false,
            json!({"type":"minecraft:int_value_check","value":1,"test":{"min":false}}),
            "test",
        ),
        (
            false,
            json!({"type":"minecraft:float_value_check","value":1,"test":false}),
            "test",
        ),
        (
            false,
            json!({"type":"minecraft:enchantment_active_check","active":1}),
            "active",
        ),
        (
            false,
            json!({"type":"minecraft:environment_attribute_check","attribute":"minecraft:visual/fog_color","value":true}),
            "value",
        ),
    ] {
        let values = HashMap::from([("test:malformed".into(), data)]);
        let empty = HashMap::new();
        let (predicates, modifiers) = if function {
            (&empty, &values)
        } else {
            (&values, &empty)
        };
        let error = Types::default()
            .validate(std::iter::empty(), predicates, modifiers)
            .expect_err(&format!(
                "malformed codec fields must not publish: {values:?}"
            ))
            .to_string();
        let registry = if function {
            "item_modifier"
        } else {
            "predicate"
        };
        assert!(
            error.contains(&format!("data/test/{registry}/malformed.json"))
                && error.contains(field)
                && error.contains("type "),
            "{error}"
        );
    }
}

#[test]
fn builtin_shapes_preserve_compact_forms_optional_nulls_and_unknown_fields() {
    for (function, data) in [
        // Contextual endpoints are independent providers, not an ordered MinMaxBounds codec.
        (
            false,
            json!({"type":"minecraft:int_value_check","value":1,"test":{"min":4,"max":3}}),
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"minecraft:intangible_projectile":{}}}),
        ),
        (
            true,
            json!({"type":"minecraft:set_count","count":2.0,"add":null,"ignored":{"type":"not:a_codec"}}),
        ),
        (
            true,
            json!({"type":"set_count","count":{"type":"uniform","min":1e1,"max":20}}),
        ),
        (
            true,
            json!({"type":"minecraft:set_custom_data","tag":{"nested":{"value":1}}}),
        ),
        (
            true,
            json!({"type":"minecraft:set_custom_data","tag":"{nested:{value:1L}}"}),
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"!minecraft:custom_data":{}}}),
        ),
        (
            true,
            json!({"type":"minecraft:set_components","components":{"minecraft:custom_data":"{value:1L}","minecraft:entity_data":"{id:'minecraft:pig'}"}}),
        ),
        (
            true,
            json!({"type":"minecraft:furnace_smelt","use_input_count":null}),
        ),
        (
            true,
            json!({"type":"minecraft:set_name","name":["a",{"text":"b","bold":null}]}),
        ),
        (
            true,
            json!({"type":"minecraft:set_name","name":{"type":"nbt","nbt":"foo[0]","storage":"test:store","interpret":"lenient false"}}),
        ),
        (
            true,
            json!({"type":"minecraft:exploration_map","destination":"#test:places","zoom":2.0,"search_radius":1e1}),
        ),
        (
            true,
            json!({"type":"minecraft:set_stew_effect","effects":null}),
        ),
        (
            true,
            json!({"type":"minecraft:set_contents","component":"container","entries":[{"type":"minecraft:group","children":null}]}),
        ),
        (
            true,
            json!({"type":"minecraft:set_contents","component":"container","entries":[{"type":"minecraft:loot_table","value":{"type":false,"pools":[]}}]}),
        ),
        (
            true,
            json!({"type":"minecraft:set_contents","component":"container","entries":[{"type":"minecraft:slots","slot_source":[{"type":"minecraft:empty"}]}]}),
        ),
        (
            true,
            json!({"type":"minecraft:modify_contents","component":"bundle_contents","modifier":[]}),
        ),
        (
            true,
            json!({"type":"minecraft:filtered","item_filter":{},"on_pass":[],"on_fail":null}),
        ),
        (
            true,
            json!({"type":"minecraft:limit_count","limit":{"min":1.0,"max":{"type":"minecraft:constant","value":5}}}),
        ),
        (
            true,
            json!({"type":"minecraft:apply_bonus","enchantment":"test:e","formula":"minecraft:ore_drops","ignored":false}),
        ),
        (
            true,
            json!({"type":"minecraft:set_loot_table","loot_table_id":"test:table","seed":-5.5}),
        ),
        (
            true,
            json!({"type":"minecraft:set_lore","lore":[],"mode":"append"}),
        ),
        (
            true,
            json!({"type":"minecraft:copy_custom_data","source":"block_entity","ops":[{"source":"foo[]","target":"bar","op":"merge"}]}),
        ),
        (
            true,
            json!({"type":"minecraft:copy_custom_data","source":{"type":"storage","source":":cache"},"ops":[]}),
        ),
        (
            true,
            json!({"type":"minecraft:set_banner_pattern","patterns":[],"append":false}),
        ),
        (
            true,
            json!({"type":"minecraft:sequence","functions":"test:no_op"}),
        ),
        (
            true,
            json!({"type":"minecraft:copy_components","source":"tool","include":null,"exclude":[]}),
        ),
        (
            true,
            json!({"type":"minecraft:set_fireworks","flight_duration":2.5}),
        ),
        (
            true,
            json!({"type":"minecraft:set_firework_explosion","colors":[],"fade_colors":null}),
        ),
        (
            true,
            json!({"type":"minecraft:set_book_cover","title":{"raw":"title","filtered":null},"generation":3.9}),
        ),
        (
            true,
            json!({"type":"minecraft:set_written_book_pages","pages":[{"raw":"page","filtered":null}],"mode":"append"}),
        ),
        (
            true,
            json!({"type":"minecraft:set_writable_book_pages","pages":["page"],"mode":"append"}),
        ),
        (
            true,
            json!({"type":"minecraft:toggle_tooltips","toggles":{}}),
        ),
        (
            true,
            json!({"type":"minecraft:set_custom_model_data","strings":{"values":[],"mode":"append"}}),
        ),
        (true, json!({"type":"minecraft:discard","condition":null})),
        (
            false,
            json!({"type":"minecraft:inverted","term":"test:yes"}),
        ),
        (false, json!({"type":"minecraft:any_of","terms":"test:yes"})),
        (
            false,
            json!({"type":"minecraft:all_of","terms":{"type":"minecraft:killed_by_player"}}),
        ),
        (
            false,
            json!({"type":"minecraft:random_chance","chance":{"type":"minecraft:constant","value":2}}),
        ),
        (
            false,
            json!({"type":"minecraft:entity_properties","entity":"this","predicate":null}),
        ),
        (
            false,
            json!({"type":"minecraft:entity_scores","entity":"this","scores":{}}),
        ),
        (
            false,
            json!({"type":"minecraft:match_block","blocks":null,"state":{}}),
        ),
        (
            false,
            json!({"type":"minecraft:match_tool","predicate":null}),
        ),
        (
            false,
            json!({"type":"minecraft:table_bonus","enchantment":"test:e","chances":[-1,2]}),
        ),
        (
            false,
            json!({"type":"minecraft:survives_explosion","ignored":false}),
        ),
        (
            false,
            json!({"type":"minecraft:damage_source_properties","predicate":null}),
        ),
        (
            false,
            json!({"type":"minecraft:location_check","predicate":null,"offsetX":1.5}),
        ),
        (
            false,
            json!({"type":"minecraft:weather_check","raining":null}),
        ),
        (
            false,
            json!({"type":"minecraft:time_check","clock":"test:clock","value":{},"period":null}),
        ),
        (
            false,
            json!({"type":"minecraft:int_value_check","value":2.0,"test":{"min":2,"max":3}}),
        ),
        (
            false,
            json!({"type":"minecraft:float_value_check","value":2,"test":2}),
        ),
        (
            false,
            json!({"type":"minecraft:enchantment_active_check","active":false}),
        ),
    ] {
        let mut predicates = HashMap::from([(
            "test:yes".into(),
            json!({"type":"minecraft:all_of","terms":[]}),
        )]);
        let mut modifiers = HashMap::from([(
            "test:no_op".into(),
            json!({"type":"minecraft:sequence","functions":[]}),
        )]);
        let label = data.to_string();
        if function {
            modifiers.insert("test:valid".into(), data);
        } else {
            predicates.insert("test:valid".into(), data);
        }
        Types::default()
            .validate(std::iter::empty(), &predicates, &modifiers)
            .unwrap_or_else(|error| panic!("{label}: {error}"));
    }
}

#[test]
fn nonnegative_float_components_preserve_signed_zero_ordering_after_narrowing() {
    for (component, field) in [
        ("minecraft:weapon", "disable_blocking_for_seconds"),
        ("minecraft:use_effects", "speed_multiplier"),
    ] {
        for (number, accepted) in [(-0.0, false), (-1e-50, false), (0.0, true), (1e-50, true)] {
            let modifiers = HashMap::from([(
                "test:signed_zero".into(),
                json!({
                    "type":"minecraft:set_components", "components":{(component):{(field):number}}
                }),
            )]);
            let result = Types::default().validate(std::iter::empty(), &HashMap::new(), &modifiers);
            assert_eq!(
                result.is_ok(),
                accepted,
                "{component}.{field} = {number:?}: {result:?}"
            );
            if let Err(error) = result {
                let error = error.to_string();
                assert!(
                    error.contains(field) && error.contains("minecraft:set_components"),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn compact_holder_sets_normalize_without_entering_opaque_custom_data() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-loot-shape-normalization-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for (path, value) in [
        (
            "item_modifier/two.json",
            json!({"type":"minecraft:set_count","count":2}),
        ),
        (
            "item_modifier/three.json",
            json!({"type":"minecraft:set_count","count":3}),
        ),
        ("item_modifier/tagged.json", json!("#edits")),
        (
            "tags/item_modifier/edits.json",
            json!({"values":["test:two","test:three"]}),
        ),
        (
            "predicate/yes.json",
            json!({"type":"minecraft:all_of","terms":[]}),
        ),
        ("tags/predicate/checks.json", json!({"values":["test:yes"]})),
        (
            "loot_table/root.json",
            json!({
                "modifier":{"type":"minecraft:set_custom_data","tag":{
                    "functions":"#test:missing","condition":{"type":"bad:type"},"modifier":"test:missing"
                }},
                "pools":[{"rolls":1,"condition":{"type":"minecraft:any_of","terms":{
                    "type":"minecraft:all_of","terms":"#checks"
                }},"entries":[{"type":"minecraft:item","name":"minecraft:stick",
                    "modifier":{"type":"minecraft:sequence","functions":"test:tagged"}
                }]}]
            }),
        ),
    ] {
        let namespace = if path.starts_with("tags/") {
            "minecraft"
        } else {
            "test"
        };
        let path = root.join("data").join(namespace).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }
    std::fs::write(
        root.join("items.json"),
        r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
    )
    .unwrap();
    let defaults = crate::item_defaults::read(&root).unwrap();
    let data = crate::loot::LootData::load_with_types(
        &root,
        &crate::loot::PackOverlay::NONE,
        &defaults,
        Types::default(),
    )
    .unwrap();
    let drops = data.roll(
        "test:root",
        &mut sunrise_core::random::LegacyRandom::new(0),
        0.0,
        None,
        None,
    );
    assert_eq!(drops.len(), 1);
    assert_eq!(
        (drops[0].item.as_str(), drops[0].count),
        ("minecraft:stick", 3)
    );
    use sunrise_core::nbt::Tag;
    let expected = Tag::compound([
        ("functions", Tag::String("#test:missing".into())),
        (
            "condition",
            Tag::compound([("type", Tag::String("bad:type".into()))]),
        ),
        ("modifier", Tag::String("test:missing".into())),
    ]);
    assert_eq!(
        drops[0]
            .components
            .iter()
            .find(|(id, _)| id == "minecraft:custom_data")
            .and_then(|(_, value)| value.as_ref()),
        Some(&expected)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn prepared_vanilla_loot_arguments_decode() {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .or_else(|| std::env::var_os("SUNRISE_RESOURCES"))
            .expect("prepared resources"),
    );
    let defaults = crate::item_defaults::read(&root).unwrap();
    let data = crate::loot::LootData::load_with_types(
        &root,
        &crate::loot::PackOverlay::NONE,
        &defaults,
        Types::default(),
    )
    .expect("prepared vanilla loot codec arguments");
    let tables = data.tables().ids().count();
    let predicates = data.predicates().len();
    let modifiers = data.modifiers().len();
    assert!(tables > 0);
    println!(
        "Decoded {tables} loot tables, {predicates} predicates and {modifiers} item modifiers; {} files decoded, 0 refused.",
        tables + predicates + modifiers
    );
}
