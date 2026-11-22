use super::*;
use serde_json::json;

#[test]
fn references_are_validated_transitively_and_cycles_refuse_by_file_and_path() {
    let types = Types::default();
    for function in [false, true] {
        let mut references = HashMap::from([
            ("test:a".into(), json!("test:b")),
            ("test:b".into(), json!("test:a")),
        ]);
        let empty = HashMap::new();
        let (predicates, modifiers) = if function {
            (&empty, &references)
        } else {
            (&references, &empty)
        };
        let error = types
            .validate(std::iter::empty(), predicates, modifiers)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(if function {
                "item_modifier"
            } else {
                "predicate"
            }) && error.contains("test:a")
                && error.contains("test:b")
                && error.contains("reference cycle"),
            "{error}"
        );
        references.insert(
            "test:b".into(),
            if function {
                json!({"type":"minecraft:set_count","count":3})
            } else {
                json!({"type":"minecraft:all_of","terms":[]})
            },
        );
        let (predicates, modifiers) = if function {
            (&empty, &references)
        } else {
            (&references, &empty)
        };
        types
            .validate(std::iter::empty(), predicates, modifiers)
            .unwrap();
    }
}
#[test]
fn unknown_types_and_bad_composites_name_the_exact_entry() {
    let types = Types::default();
    let empty = HashMap::new();
    for (modifier, named) in [
        (json!({"type":"example:typo"}), "example:typo"),
        (json!({"type":"minecraft:typo"}), "minecraft:typo"),
        (
            json!({"type":"minecraft:sequence","functions":false}),
            "minecraft:sequence",
        ),
        (
            json!({"type":"minecraft:set_count","count":1,"condition":{"type":"minecraft:all_of","terms":[false]}}),
            "<missing>",
        ),
    ] {
        let table = json!({"pools":[{"entries":[{"type":"minecraft:item","name":"minecraft:stick","modifier":modifier}]}]});
        let error = types
            .validate(std::iter::once(("test:root", &table)), &empty, &empty)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("loot_table test:root")
                && error.contains("data/test/loot_table/root.json")
                && error.contains("$.pools[0].entries[0].modifier")
                && error.contains(named),
            "{error}"
        );
    }
}
#[test]
fn pack_predicates_bind_for_validation_and_world_evaluation() {
    let root = std::env::temp_dir().join(format!("sunrise-pack-predicate-{}", std::process::id()));
    let dir = root.join("data/test/loot_table");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(root.join("items.json"), "{}").unwrap();
    std::fs::write(
        dir.join("root.json"),
        json!({"pools":[{
            "rolls":1,"condition":"pack:enabled",
            "entries":[{"type":"minecraft:item","name":"minecraft:stick"}]
        }]})
        .to_string(),
    )
    .unwrap();
    let files = vec![(
        "predicate/pack:enabled".into(),
        vec![br#"{"type":"minecraft:random_chance","chance":1.0}"#.to_vec()],
    )];
    let defaults = crate::item_defaults::read(&root).unwrap();
    let data = crate::loot::LootData::load_with_types(
        &root,
        &crate::loot::PackOverlay::new(&files),
        &defaults,
        Types::default(),
    )
    .unwrap();
    let output = data.roll(
        "test:root",
        &mut sunrise_core::random::LegacyRandom::new(1),
        0.0,
        None,
        None,
    );
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].item, "minecraft:stick");
    let error = crate::loot::LootData::load_with_types(
        &root,
        &crate::loot::PackOverlay::NONE,
        &defaults,
        Types::default(),
    )
    .err()
    .expect("an unbound registry holder refuses loading")
    .to_string();
    assert!(
        error.contains("pack:enabled") && error.contains("unknown reference"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn conditional_function_with_null_condition_loads_and_applies() {
    let root = std::env::temp_dir().join(format!("sunrise-null-function-{}", std::process::id()));
    let dir = root.join("data/test/loot_table");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(root.join("items.json"), "{}").unwrap();
    std::fs::write(
        dir.join("root.json"),
        json!({"pools":[{
            "rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick",
                "modifier":{"type":"minecraft:set_count","count":7,"condition":null}}]
        }]})
        .to_string(),
    )
    .unwrap();
    let data = crate::loot::LootData::load_with_types(
        &root,
        &crate::loot::PackOverlay::NONE,
        &crate::item_defaults::read(&root).unwrap(),
        Types::default(),
    )
    .unwrap();
    let output = data.roll(
        "test:root",
        &mut sunrise_core::random::LegacyRandom::new(1),
        0.0,
        None,
        None,
    );
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].item, "minecraft:stick");
    assert_eq!(output[0].count, 7);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn selecting_a_pack_never_falls_back_from_malformed_winning_loot_data() {
    let root = std::env::temp_dir().join(format!("sunrise-codec-selection-{}", std::process::id()));
    std::fs::create_dir_all(root.join("data/test/predicate")).unwrap();
    std::fs::write(
        root.join("data/test/predicate/value.json"),
        b"malformed lower data",
    )
    .unwrap();
    let valid = br#"{"type":"minecraft:all_of","terms":[]}"#.to_vec();
    let files = vec![("predicate/test:value".into(), vec![valid.clone()])];
    let overlay = super::super::PackOverlay::new(&files);
    assert_eq!(
        super::super::load_predicates(&root, &overlay).unwrap()["test:value"],
        json!({"type":"minecraft:all_of","terms":[]})
    );
    let files = vec![(
        "predicate/test:value".into(),
        vec![valid, b"invalid winner".to_vec()],
    )];
    let error = super::super::load_predicates(&root, &super::super::PackOverlay::new(&files))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("predicate test:value") && error.contains("undecodable"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
