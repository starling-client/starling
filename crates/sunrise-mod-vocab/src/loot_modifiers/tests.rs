use super::*;
use crate::{ItemKey, LegacyRandomSource, LoadReader, OwningLoad};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn common_fields_decode_defaults_limits_and_predicate_holders() {
    let defaults = LootModifierSettings::decode(&json!({"type":"test:modifier"})).unwrap();
    assert_eq!(defaults.priority, 1000);
    assert!(defaults.condition.is_none());
    let settings =
        LootModifierSettings::decode(&json!({"priority":-17,"condition":"can_silk_touch"}))
            .unwrap();
    assert_eq!(settings.priority, -17);
    assert_eq!(settings.condition, Some(json!("minecraft:can_silk_touch")));
    for priority in [
        json!(true),
        json!(1.5),
        json!(2147483648_i64),
        json!(-2147483649_i64),
        json!("1000"),
    ] {
        assert_eq!(
            LootModifierSettings::decode(&json!({"priority":priority}))
                .unwrap_err()
                .field,
            "priority"
        );
    }
    for condition in [json!(null), json!([]), json!(7), json!("bad namespace:id")] {
        assert_eq!(
            LootModifierSettings::decode(&json!({"condition":condition}))
                .unwrap_err()
                .field,
            "condition"
        );
    }
    let inline = json!({"type":"neoforge:loot_table_id","loot_table_id":"test:root"});
    assert_eq!(
        LootModifierSettings::decode(&json!({"condition":inline}))
            .unwrap()
            .condition,
        Some(inline)
    );
}

struct Reader;
impl LoadReader for Reader {
    fn registry(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        None
    }
    fn setting(&self, _: &str, _: &str) -> Option<&str> {
        None
    }
}
struct Roll(LegacyRandomSource);
impl LootModifierContext for Roll {
    fn snapshot(&self) -> LootContext {
        let mut context = LootContext::new(0);
        context.queried_table_id = "test:root".into();
        context
    }
    fn random(&mut self) -> &mut dyn RandomSource {
        self
    }
    fn roll_raw(&mut self, table: &Identifier) -> Result<Vec<LootStack>, LootModifierError> {
        assert_eq!(table.to_string(), "test:raw");
        Ok(vec![LootStack::new(ItemKey(2), 0)])
    }
}
impl RandomSource for Roll {
    fn next_int(&mut self) -> i32 {
        self.0.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.0.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.next_boolean()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.next_gaussian()
    }
}
struct Codec(Arc<AtomicUsize>);
impl GlobalLootModifierSerializer for Codec {
    fn decode(
        &self,
        _: &Value,
        load: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        assert_eq!(load.origin(), crate::DataLoadOrigin::ServerDataLoad);
        Ok(Arc::new(Modifier(self.0.clone())))
    }
}
struct Modifier(Arc<AtomicUsize>);
impl GlobalLootModifier for Modifier {
    fn priority(&self) -> Result<i32, LootModifierError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(37)
    }
    fn condition(&self) -> Result<Option<Value>, LootModifierError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Some(
            json!({"type":"neoforge:loot_table_id","loot_table_id":"test:root"}),
        ))
    }
    fn apply(
        &self,
        context: &mut dyn LootModifierContext,
        mut generated: Vec<LootStack>,
    ) -> Result<Vec<LootStack>, LootModifierError> {
        assert_eq!(context.snapshot().queried_table_id, "test:root");
        generated.reverse();
        generated[1].count -= context.random().next_int_bound(2);
        generated.extend(context.roll_raw(&Identifier::parse("test:raw").unwrap())?);
        Ok(generated)
    }
}

#[test]
fn preparation_caches_metadata_and_application_keeps_signed_output_and_roll_random() {
    let reads = Arc::new(AtomicUsize::new(0));
    let codec = owned_serializer("test", 0, Arc::new(Codec(reads.clone())));
    let load = OwningLoad::new(crate::DataLoadOrigin::ServerDataLoad, &Reader);
    let prepared = codec.decode(&json!({}), &load).unwrap();
    drop(codec);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    for _ in 0..2 {
        assert_eq!(prepared.priority().unwrap(), 37);
        assert!(prepared.condition().unwrap().is_some());
    }
    let mut roll = Roll(LegacyRandomSource::new(0));
    let mut expected = LegacyRandomSource::new(0);
    assert_eq!(expected.next_int_bound(2), 1);
    let output = prepared
        .apply(
            &mut roll,
            vec![
                LootStack::new(ItemKey(0), -7),
                LootStack::new(ItemKey(1), 130),
            ],
        )
        .unwrap();
    assert_eq!(
        output
            .iter()
            .map(|stack| (stack.item, stack.count))
            .collect::<Vec<_>>(),
        [(ItemKey(1), 130), (ItemKey(0), -8), (ItemKey(2), 0)]
    );
    assert_eq!(
        <(u64, Option<f64>)>::from(&roll.0),
        <(u64, Option<f64>)>::from(&expected)
    );
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

struct Panics(Arc<AtomicUsize>);
impl GlobalLootModifierSerializer for Panics {
    fn decode(
        &self,
        _: &Value,
        _: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        Ok(Arc::new(Panics(self.0.clone())))
    }
}
impl GlobalLootModifier for Panics {
    fn apply(
        &self,
        _: &mut dyn LootModifierContext,
        _: Vec<LootStack>,
    ) -> Result<Vec<LootStack>, LootModifierError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("native fixture callback panics");
    }
}
#[test]
fn a_native_panic_disables_only_that_prepared_instance() {
    let calls = Arc::new(AtomicUsize::new(0));
    let codec = owned_serializer("test", 0, Arc::new(Panics(calls.clone())));
    let load = OwningLoad::new(crate::DataLoadOrigin::ServerDataLoad, &Reader);
    let first = codec.decode(&json!({}), &load).unwrap();
    let second = codec.decode(&json!({}), &load).unwrap();
    let mut roll = Roll(LegacyRandomSource::new(0));
    assert!(
        first
            .apply(&mut roll, vec![])
            .unwrap_err()
            .reason
            .contains("panicked")
    );
    assert_eq!(
        first.apply(&mut roll, vec![]).unwrap_err().reason,
        "loot modifier owner is disabled"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        second
            .apply(&mut roll, vec![])
            .unwrap_err()
            .reason
            .contains("panicked")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
