use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Once},
};
use sunrise_world::loot::{
    self, LootStack,
    types::{
        BoundCondition, BoundFunction, DecodedCondition, DecodedFunction, Registry, RollRandom,
        Types,
    },
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

struct RefusingFunction;
impl BoundFunction for RefusingFunction {
    fn apply(
        &self,
        _: &loot::LootContext<'_>,
        _: &mut dyn RollRandom,
        _: &dyn loot::LootTags,
        _: &mut dyn loot::LootStackEditAccess,
    ) -> Result<(), String> {
        Err("mod test: function refused".into())
    }
}

struct RefusingCondition;
impl BoundCondition for RefusingCondition {
    fn test(
        &self,
        _: &loot::LootContext<'_>,
        _: &mut dyn RollRandom,
        _: &dyn loot::LootTags,
    ) -> Result<bool, String> {
        Err("mod test: condition refused".into())
    }
}

struct RefusingRegistry;
impl Registry for RefusingRegistry {
    fn contains(&self, kind: &str, _: bool) -> bool {
        kind == "test:broken"
    }
    fn conditional(&self, _: &str) -> bool {
        true
    }
    fn decode_function(&self, _: &str, _: &Value) -> Result<DecodedFunction, String> {
        Ok(DecodedFunction {
            rule: Arc::new(RefusingFunction),
            referenced_params: Vec::new(),
        })
    }
    fn decode_condition(&self, _: &str, _: &Value) -> Result<DecodedCondition, String> {
        Ok(DecodedCondition {
            rule: Arc::new(RefusingCondition),
            referenced_params: Vec::new(),
        })
    }
}

struct EmptyTags;
impl loot::LootTags for EmptyTags {
    fn contains(&self, _: &str, _: &str, _: &str) -> bool {
        false
    }
    fn members(&self, _: &str, _: &str) -> Option<Vec<String>> {
        None
    }
    fn item_tag_names(&self) -> Option<&loot::ItemTagNames> {
        None
    }
}

#[test]
fn ordinary_callback_refusals_report_each_call_with_type_location_and_reason() {
    capture();
    let types = Types::new(Arc::new(RefusingRegistry));
    let predicates = Arc::new(HashMap::from([(
        "test:pred".into(),
        json!({"type":"test:broken"}),
    )]));
    let modifiers = Arc::new(HashMap::from([(
        "test:mod".into(),
        json!({"type":"test:broken"}),
    )]));
    let bindings = types
        .bind_generation(
            Arc::default(),
            Arc::clone(&predicates),
            Arc::clone(&modifiers),
            Vec::new(),
            Default::default(),
        )
        .unwrap();
    let ctx = loot::LootContext {
        queried_table_id: None,
        queried_stack: None,
        entity: None,
        damage: None,
        player: None,
        interacting_entity: None,
        tool: None,
        block: None,
        luck: 0.0,
        biome: None,
        place: None,
    };
    let mut random = sunrise_core::random::LegacyRandom::new(1);
    let mut stack = LootStack {
        item: "minecraft:stick".into(),
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
    for _ in 0..2 {
        bindings
            .function(None, "test:broken", &modifiers["test:mod"])
            .unwrap()
            .apply(&ctx, &mut random, &EmptyTags, &mut stack);
        assert!(
            !bindings
                .condition(None, "test:broken", &predicates["test:pred"])
                .unwrap()
                .test(&ctx, &mut random, &EmptyTags,)
        );
    }
    RECORDS.with_borrow(|records| {
        assert_eq!(records.len(), 4, "{records:?}");
        assert!(
            records[0].contains("loot function type test:broken")
                && records[0].contains("item_modifier test:mod")
                && records[0].contains("mod test: function refused"),
            "{}",
            records[0]
        );
        assert_eq!(records[0], records[2]);
        assert!(
            records[1].contains("loot condition type test:broken")
                && records[1].contains("predicate test:pred")
                && records[1].contains("mod test: condition refused"),
            "{}",
            records[1]
        );
        assert_eq!(records[1], records[3]);
    });
}

struct NeedsAttacker;
struct AlwaysTrue;
impl BoundCondition for AlwaysTrue {
    fn test(
        &self,
        _: &loot::LootContext<'_>,
        _: &mut dyn RollRandom,
        _: &dyn loot::LootTags,
    ) -> Result<bool, String> {
        Ok(true)
    }
}
impl Registry for NeedsAttacker {
    fn contains(&self, kind: &str, function: bool) -> bool {
        !function && kind == "test:condition"
    }
    fn conditional(&self, _: &str) -> bool {
        true
    }
    fn decode_function(&self, kind: &str, _: &Value) -> Result<DecodedFunction, String> {
        Err(format!("unknown loot function type {kind}"))
    }
    fn decode_condition(&self, _: &str, _: &Value) -> Result<DecodedCondition, String> {
        Ok(DecodedCondition {
            rule: Arc::new(AlwaysTrue),
            referenced_params: vec!["minecraft:attacking_entity".into()],
        })
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
