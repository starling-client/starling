use std::collections::{BTreeMap, BTreeSet};
use {
    sunrise_core::nbt::Tag,
    sunrise_world::{
        DATA_VERSION,
        registry::BlockRegistry,
        worldgen::{block_feature::FeatureBlockBindings, rule_test::RuleTest},
    },
};
fn registry() -> BlockRegistry {
    let names = [
        "minecraft:air",
        "minecraft:cave_air",
        "minecraft:void_air",
        "minecraft:stone",
    ];
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                names
                    .iter()
                    .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::Compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(id, name)| ((*name).into(), Tag::Int(id as i32)))
                    .collect(),
            ),
        ),
    ]))
    .unwrap()
}
#[test]
fn rule_decoding_bounds_nested_and_branching_data() {
    let registry = registry();
    let bindings = FeatureBlockBindings::new(&registry, BTreeSet::new()).unwrap();
    let leaf = r#"{"predicate_type":"always_true"}"#;
    let mut nested = leaf.to_owned();
    for _ in 0..130 {
        nested = format!(r#"{{"predicate_type":"not","rule":{nested}}}"#);
    }
    assert!(RuleTest::from_json(&nested, &bindings).is_err());
    let branch = format!(
        r#"{{"predicate_type":"all_of","rules":[{}]}}"#,
        vec![leaf; 8192].join(",")
    );
    let tree = format!(
        r#"{{"predicate_type":"all_of","rules":[{}]}}"#,
        [branch.as_str(); 9].join(",")
    );
    assert!(RuleTest::from_json(&tree, &bindings).is_err());
}
#[test]
fn binding_rejects_unknown_tag_blocks_and_duplicate_canonical_tags() {
    let registry = registry();
    let tags = BTreeMap::from([("sunrise:test".into(), vec!["missing".into()])]);
    assert!(FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &tags).is_err());
    let tags = BTreeMap::from([
        ("test".into(), vec!["stone".into()]),
        ("minecraft:test".into(), vec!["air".into()]),
    ]);
    assert!(FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &tags).is_err());
}
#[test]
fn removed_sunrise_rule_tests_are_rejected_without_a_mod() {
    let registry = registry();
    let bindings = FeatureBlockBindings::new(&registry, BTreeSet::new()).unwrap();
    let err = RuleTest::from_json(
        r#"{"predicate_type":"sunrise:matter_phase","phase_to_test_for":"AIR"}"#,
        &bindings,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("sunrise:matter_phase"), "{err}");
    assert!(err.contains("predicate_type"), "{err}");

    for pos_type in [
        "sunrise:y_value",
        "sunrise:piece_origin_axis_aligned_linear_pos",
    ] {
        let value = serde_json::json!({ "predicate_type": pos_type });
        let err = sunrise_world::worldgen::structure::processor::PosRule::decode(Some(&value))
            .unwrap_err()
            .to_string();
        assert!(err.contains(pos_type), "{err}");
        assert!(err.contains("predicate_type"), "{err}");
    }
}
