use super::*;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn registered(count: Arc<AtomicUsize>) -> LoadConditions {
    LoadConditions::new(["testmod", "othermod"]).with_lookup([(
        "testmod:flag",
        move |args: &Value| {
            count.fetch_add(1, Ordering::SeqCst);
            args["value"]
                .as_bool()
                .ok_or_else(|| "value must be a boolean".into())
        },
    )])
}
fn variants(name: &str) -> Vec<Value> {
    let leaf = json!({"type":name});
    vec![
        leaf.clone(),
        json!({"type":"neoforge:not", "value":{"type":"neoforge:not", "value":leaf}}),
        json!({"type":"neoforge:and", "values":[{"type":"neoforge:mod_loaded","modid":"absent"},leaf]}),
        json!({"type":"neoforge:or", "values":[{"type":"neoforge:always"},leaf]}),
    ]
}
#[test]
fn unknown_ids_fail_before_any_test_regardless_of_namespace_or_nesting() {
    for id in [
        "testmod:nope",
        "othermod:nope",
        "absent:nope",
        "minecraft:nope",
        "neoforge:nope",
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        let conditions = registered(count.clone());
        for (index, node) in variants(id).into_iter().enumerate() {
            let path = format!("data/example/predicate/file_{index}.json");
            let file = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}, node]});
            assert!(!conditions.allows_file(&file, &path));
            assert_eq!(
                conditions.allows_file_isolated(&file, &path),
                Err(format!("unknown load conditions [{id}]"))
            );
            let error = conditions.check_errors().unwrap_err();
            assert!(
                error.contains(&format!("{path}: unknown load conditions [{id}]")),
                "{error}"
            );
        }
        assert_eq!(
            conditions.check_errors().unwrap_err().matches(id).count(),
            4
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn registered_conditions_cache_each_entry_and_new_reload_evaluates_again() {
    let count = Arc::new(AtomicUsize::new(0));
    let yes = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]});
    let no = json!({"neoforge:conditions":[{"type":"testmod:flag","value":false}]});
    for expected in [2, 4] {
        let conditions = registered(count.clone());
        for _ in 0..3 {
            assert!(conditions.allows_id(&yes, "recipe", "testmod:enabled"));
            assert!(!conditions.allows_id(&no, "recipe", "testmod:disabled"));
        }
        assert_eq!(count.load(Ordering::SeqCst), expected);
    }
    let conditions = registered(count);
    assert!(conditions.allows_file(&json!({"neoforge:conditions":[{"type":"neoforge:and","values":[{"type":"testmod:flag","value":true},{"type":"neoforge:or","values":[{"type":"testmod:flag","value":false},{"type":"neoforge:always"}]}]}]}), "nested.json"));
    assert!(!conditions.allows_file(
        &json!({"neoforge:conditions":[{"type":"testmod:flag","value":"bad"}]}),
        "bad.json"
    ));
    assert_eq!(
        conditions.check_errors().unwrap_err(),
        "bad.json: value must be a boolean"
    );
}
#[test]
fn unavailable_context_does_not_hide_a_reached_handler_error() {
    let conditions = registered(Arc::new(AtomicUsize::new(0)));
    assert!(!conditions.allows_file(
        &json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
            {"type":"neoforge:tag_empty","tag":"testmod:empty"},
            {"type":"testmod:flag","value":"bad"}
        ]}]}),
        "bad.json"
    ));
    assert_eq!(
        conditions.check_errors().unwrap_err(),
        "bad.json: value must be a boolean"
    );
}

#[test]
fn only_typed_context_reads_partition_cached_conditions() {
    let count = Arc::new(AtomicUsize::new(0));
    let load = LoadConditions::new([]).with_lookup([("test:flag", {
        let count = count.clone();
        move |args: &Value| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(args["value"] == "neoforge:registered")
        }
    })]);
    let literal = json!({"neoforge:conditions":[
        {"type":"test:flag","value":"neoforge:registered"}
    ]});
    let file = json!({"neoforge:conditions":[{"type":"neoforge:and","values":[
        {"type":"neoforge:registered","registry":"test:spells","value":"test:flame"},
        {"type":"neoforge:not","value":{"type":"neoforge:tag_empty","tag":"test:fuels"}},
        {"type":"test:flag","value":"neoforge:registered"}
    ]}]});
    let empty = load
        .clone()
        .with_registries([("test:spells", ["test:flame"])])
        .with_tags([("item", ["test:other"])]);
    let filled = empty.with_tags([("item", ["test:fuels"])]);
    let missing_spell = filled
        .clone()
        .with_registries([("test:spells", ["test:spark"])]);
    assert!(empty.allows_file(&literal, "literal.json"));
    assert_eq!(
        filled.allows_file_isolated(&literal, "literal.json"),
        Ok(true)
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(empty.allows_file_isolated(&file, "nested.json"), Ok(false));
    assert_eq!(
        missing_spell.allows_file_isolated(&file, "nested.json"),
        Ok(false)
    );
    assert_eq!(filled.allows_file_isolated(&file, "nested.json"), Ok(true));
    assert!(filled.allows_file(&file, "nested.json"));
    assert_eq!(count.load(Ordering::SeqCst), 2);

    let literal_program = load.decode_file_conditions(&literal).unwrap();
    assert!(matches!(
        literal_program
            .evaluation()
            .advance(&filled, |registry| panic!(
                "unexpected registry read: {registry}"
            )),
        Ok(ConditionProgress::Complete(true))
    ));
    let program = load.decode_file_conditions(&file).unwrap();
    let mut evaluation = program.evaluation();
    assert!(matches!(
        evaluation.advance(&load, |_| true),
        Ok(ConditionProgress::NeedsRegistry("test:spells"))
    ));
    assert!(matches!(
        evaluation.advance(&empty, |registry| registry == "minecraft:item"),
        Ok(ConditionProgress::NeedsRegistry("minecraft:item"))
    ));
    assert!(matches!(
        evaluation.advance(&filled, |_| false),
        Ok(ConditionProgress::Complete(true))
    ));
    assert_eq!(count.load(Ordering::SeqCst), 4);
    assert_eq!(load.check_errors(), Ok(()));
}
#[test]
fn no_mod_conditions_do_not_construct_file_context_for_known_nodes() {
    let conditions = LoadConditions::new([]);
    for (node, expected) in [
        (json!({"type":"neoforge:always"}), true),
        (
            json!({"type":"neoforge:mod_loaded","modid":"absent"}),
            false,
        ),
        (
            json!({"type":"neoforge:not","value":{"type":"neoforge:mod_loaded","modid":"absent"}}),
            true,
        ),
    ] {
        assert_eq!(
            conditions.allows_context(&json!({"neoforge:conditions":[node]}), || panic!(
                "fast path formatted a file context"
            )),
            expected
        );
    }
}
#[test]
fn concurrent_registered_readers_evaluate_each_entry_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let conditions = registered(count.clone());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                assert!(conditions.allows_file(
                    &json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]}),
                    "concurrent.json"
                ))
            });
        }
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn inventory_changes_invalidate_condition_answers_and_errors_without_mutating_old_view() {
    let old = registered(Arc::default());
    let file = json!({"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"create"}]});
    assert!(!old.allows_file(&file, "inventory.json"));
    let new = old.with_mod_inventory(["create"]);
    assert!(new.allows_file(&file, "inventory.json"));
    assert!(!old.allows_file(&file, "inventory.json"));
    assert!(!old.allows_file(
        &json!({"neoforge:conditions":[{"type":"testmod:nope"}]}),
        "bad.json"
    ));
    let changed = old.with_mod_inventory(["create"]);
    assert_eq!(changed.check_errors(), Ok(()));
    assert!(old.check_errors().is_err());
}

#[test]
fn inventory_partitions_are_shared_by_readers_of_the_same_candidate() {
    let count = Arc::new(AtomicUsize::new(0));
    let original = registered(count.clone());
    let one = original.with_mod_inventory(["create"]);
    let two = original.with_mod_inventory(["create"]);
    let file = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]});
    assert!(one.allows_file(&file, "shared.json"));
    assert!(two.allows_file(&file, "shared.json"));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
