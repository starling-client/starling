use super::*;
use mod_api::{Hooks, Identifier, LootTypeContext, Registrar, RegistrationContext};
use serde_json::json;
use std::sync::Mutex;
#[derive(Clone)]
struct Codec(Arc<Mutex<Vec<(String, i32)>>>);
impl ModLootFunction for Codec {
    fn validate(&self, data: &LootEntryData) -> Result<(), String> {
        data.value()["factor"]
            .as_i64()
            .map(|_| ())
            .ok_or_else(|| "factor: expected integer".into())
    }
    fn apply(
        &self,
        data: &LootEntryData,
        mut stack: mod_api::LootStack,
        context: &mut dyn mod_api::LootTypeContext,
    ) -> Result<mod_api::LootStack, String> {
        assert!(context.snapshot().place.is_none());
        assert_eq!(context.snapshot().seed, 0);
        let draw = context.random().next_int_bound(5);
        self.0
            .lock()
            .unwrap()
            .push((data.value()["label"].as_str().unwrap().into(), draw));
        stack.count = stack
            .count
            .wrapping_mul(data.value()["factor"].as_i64().unwrap() as i32)
            .wrapping_add(draw);
        Ok(stack)
    }
}
impl ModLootCondition for Codec {
    fn validate(&self, data: &LootEntryData) -> Result<(), String> {
        data.value()["pass"]
            .as_bool()
            .map(|_| ())
            .ok_or_else(|| "pass: expected boolean".into())
    }
    fn referenced_context_params(&self) -> Vec<Identifier> {
        vec![Identifier::parse("minecraft:block_entity").unwrap()]
    }
    fn test(
        &self,
        data: &LootEntryData,
        context: &mut dyn LootTypeContext,
    ) -> Result<bool, String> {
        assert!(context.snapshot().place.is_none());
        let draw = context.random().next_int_bound(5);
        self.0
            .lock()
            .unwrap()
            .push((data.value()["label"].as_str().unwrap().into(), draw));
        Ok(data.value()["pass"].as_bool().unwrap())
    }
}
#[mod_api::__subscriptions]
impl Hooks for Codec {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.item_key("minecraft:stick");
        r.register_loot_function_type(&Identifier::parse("test:scale").unwrap(), self.clone());
        r.register_loot_condition_type(&Identifier::parse("test:gate").unwrap(), self.clone());
    }
}
fn gate(label: &str, pass: bool) -> serde_json::Value {
    json!({"type":"test:gate", "label":label, "pass":pass})
}
fn function(label: &str, factor: i32) -> serde_json::Value {
    json!({"type":"test:scale", "label":label, "factor":factor})
}

#[test]
fn registered_types_preserve_both_evaluators_order_short_circuit_and_selected_random() {
    use sunrise_core::random::{LegacyRandom, RandomSource};
    let observations = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "test",
            Box::new(Codec(observations.clone())),
        )]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("data/test/loot_table");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                root.path().join("items.json"),
                r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
            )
            .unwrap();
            let mut skipped = function("skipped body", 9);
            skipped["condition"] = gate("gate false", false);
            let mut scaled = function("entry", 3);
            scaled["condition"] = gate("gate true", true);
            let table = json!({"type":"minecraft:empty", "pools":[{"rolls":1,
            "condition":{"type":"minecraft:all_of", "terms":[gate("pool", true), {"type":"minecraft:any_of", "terms":[gate("any",true), gate("skipped any",false)]}]},
            "entries":[{"type":"minecraft:item","name":"minecraft:stick",
                "condition":{"type":"minecraft:any_of","terms":[gate("entry false",false), gate("entry true",true), gate("skipped entry",false)]},
                "modifier":{"type":"minecraft:sequence","functions":[{"type":"minecraft:set_count","count":2},skipped,scaled,{"type":"minecraft:filtered","item_filter":{"items":"minecraft:stick"},"on_pass":function("sequence",2),"on_fail":function("skipped filter fail",9)},{"type":"minecraft:filtered","item_filter":{"items":"minecraft:diamond"},"on_pass":function("skipped filter pass",9),"on_fail":function("filtered fail",1)}]}}],
            "modifier":function("pool function",2)}], "modifier":function("table function",2)});
            std::fs::write(folder.join("root.json"), table.to_string()).unwrap();
            let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
            let data = loot::LootData::load_with_types(
                root.path(),
                &loot::PackOverlay::NONE,
                &defaults,
                capture(),
            )
            .unwrap();
            let mut items = sunrise_items::items::Items::for_tests(HashMap::new());
            items.set_loot_data(&data);
            let mut expected_random = LegacyRandom::new(123);
            let labels = [
                "pool",
                "any",
                "entry false",
                "entry true",
                "gate false",
                "gate true",
                "entry",
                "sequence",
                "filtered fail",
                "pool function",
                "table function",
            ];
            let expected: Vec<_> = labels
                .iter()
                .map(|label| {
                    (
                        label.to_string(),
                        RandomSource::next_int_bound(&mut expected_random, 5).unwrap(),
                    )
                })
                .collect();
            let count = ((((2 * 3 + expected[6].1) * 2 + expected[7].1) + expected[8].1) * 2
                + expected[9].1)
                * 2
                + expected[10].1;
            for item_engine in [false, true] {
                observations.lock().unwrap().clear();
                let mut random = LegacyRandom::new(123);
                let total = if item_engine {
                    items
                        .loot_table("test:root", false, None, &mut random)
                        .iter()
                        .map(|stack| stack.count as i32)
                        .sum::<i32>()
                } else {
                    data.roll("test:root", &mut random, 0.0, None, None)
                        .iter()
                        .map(|stack| stack.count)
                        .sum::<i32>()
                };
                assert_eq!(total, count);
                assert_eq!(*observations.lock().unwrap(), expected);
                assert_eq!(
                    RandomSource::next_long(&mut random),
                    RandomSource::next_long(&mut expected_random.clone())
                );
            }
            for (entry, expected) in [
                (json!({"type":"test:absent"}), "unknown loot type"),
                (json!({"type":"test:scale","factor":false}), "factor"),
                (json!({"type":"minecraft:absent"}), "unknown loot type"),
            ] {
                let mut bad = table.clone();
                bad["modifier"] = entry;
                std::fs::write(folder.join("root.json"), bad.to_string()).unwrap();
                let error = loot::LootData::load_with_types(
                    root.path(),
                    &loot::PackOverlay::NONE,
                    &defaults,
                    capture(),
                )
                .err()
                .unwrap()
                .to_string();
                assert!(
                    error.contains("loot_table test:root")
                        && error.contains("[data/test/loot_table/root.json]")
                        && error.contains("$.modifier")
                        && error.contains(expected),
                    "{error}"
                );
                // Refusing a candidate never mutates the already published generation.
                observations.lock().unwrap().clear();
                assert!(
                    !data
                        .roll("test:root", &mut LegacyRandom::new(123), 0.0, None, None)
                        .is_empty()
                );
            }
        },
    );
}
