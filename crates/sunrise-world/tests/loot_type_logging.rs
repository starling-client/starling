use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Once},
};
use sunrise_world::loot::{
    self,
    types::{Registry, RollRandom, Types},
};

thread_local! {
    static RECORDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
struct Capture;
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        RECORDS.with_borrow_mut(|records| records.push(record.args().to_string()));
    }
    fn flush(&self) {}
}
static CAPTURE: Capture = Capture;
static INSTALL: Once = Once::new();
fn capture() {
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
    });
    RECORDS.with_borrow_mut(Vec::clear);
}

#[test]
fn callback_refusals_warn_once_per_registry_type_and_data_generation() {
    capture();
    let generation = Types::default();
    for _ in 0..3 {
        generation.warn_once("test:broken", true, "function refused");
        generation
            .clone()
            .warn_once("test:broken", true, "function refused");
        generation.warn_once("test:broken", false, "condition refused");
    }
    Types::default().warn_once("test:broken", true, "new generation refused");
    RECORDS.with_borrow(|records| {
        assert_eq!(
            records.as_slice(),
            [
                "loot function type test:broken: function refused",
                "loot condition type test:broken: condition refused",
                "loot function type test:broken: new generation refused",
            ]
        )
    });
}

struct NeedsAttacker;
impl Registry for NeedsAttacker {
    fn contains(&self, kind: &str, function: bool) -> bool {
        !function && kind == "test:condition"
    }
    fn validate(&self, _: &str, _: &Value, _: bool) -> Result<Vec<String>, String> {
        Ok(vec!["minecraft:attacking_entity".into()])
    }
    fn conditional(&self, _: &str) -> bool {
        true
    }
    fn apply(
        &self,
        _: &str,
        _: &Value,
        _: &loot::LootContext<'_>,
        _: &mut dyn RollRandom,
        _: &dyn loot::LootTags,
        stack: &loot::LootStack,
    ) -> Result<loot::LootStack, String> {
        Ok(stack.clone())
    }
    fn test(
        &self,
        _: &str,
        _: &Value,
        _: &loot::LootContext<'_>,
        _: &mut dyn RollRandom,
        _: &dyn loot::LootTags,
    ) -> Result<bool, String> {
        Ok(true)
    }
}

#[test]
fn referenced_tables_keep_the_referring_context_and_stop_at_cycles() {
    capture();
    let types = Types::new(Arc::new(NeedsAttacker));
    let reference =
        |id: &str| json!({"pools":[{"entries":[{"type":"minecraft:loot_table","value":id}]}]});
    let mut block = reference("test:generic");
    block["type"] = json!("minecraft:block");
    let mut generic = reference("test:block");
    generic["type"] = json!("minecraft:generic");
    generic["modifier"] =
        json!({"type":"minecraft:set_count","count":1,"condition":{"type":"test:condition"}});
    let empty = HashMap::new();
    types
        .validate(
            [("test:block", &block), ("test:generic", &generic)].into_iter(),
            &empty,
            &empty,
        )
        .unwrap();
    RECORDS.with_borrow(|records| {
        assert_eq!(records.len(), 1, "{records:?}");
        let warning = &records[0];
        assert!(
            warning.contains("loot_table test:block")
                && warning.contains("loot_table test:generic")
                && warning.contains("test:condition")
                && warning.contains("minecraft:attacking_entity"),
            "{warning}"
        );
    });
}
