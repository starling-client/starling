use super::*;
use crate::{
    __private::{LootListAccess, LootStackAccess, LootStackEditAccess, LootToolAccess},
    ExplorationMap, LegacyRandomSource, LoadReader, LootExplorationView, OwningLoad, Tag,
    TextFormError,
    callback_owner::CallbackOwner,
};
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
struct TestStack {
    item_id: String,
    count: i32,
    damage: i32,
    map: bool,
    enchantments: Vec<(String, i32)>,
    potion: Option<String>,
    stew_effects: Vec<(String, i32)>,
    ominous_amplifier: Option<i32>,
    exploration: Option<ExplorationMap>,
    names: Vec<(String, Value)>,
    components: Vec<(String, Option<Tag>)>,
}

impl TestStack {
    fn new(item_id: &str, count: i32) -> Self {
        Self {
            item_id: item_id.to_owned(),
            count,
            damage: 0,
            map: false,
            enchantments: Vec::new(),
            potion: None,
            stew_effects: Vec::new(),
            ominous_amplifier: None,
            exploration: None,
            names: Vec::new(),
            components: Vec::new(),
        }
    }
}

impl LootToolAccess for TestStack {
    fn item_id(&self) -> &str {
        &self.item_id
    }
    fn enchantment_at(&self, index: usize) -> Option<(&str, i32)> {
        self.enchantments
            .get(index)
            .map(|(id, level)| (id.as_str(), *level))
    }
}

impl LootStackAccess for TestStack {
    fn count(&self) -> i32 {
        self.count
    }
    fn damage(&self) -> i32 {
        self.damage
    }
    fn map(&self) -> bool {
        self.map
    }
    fn components(&self) -> &[(String, Option<Tag>)] {
        &self.components
    }
    fn enchantments(&self) -> &[(String, i32)] {
        &self.enchantments
    }
    fn potion(&self) -> Option<&str> {
        self.potion.as_deref()
    }
    fn stew_effects(&self) -> &[(String, i32)] {
        &self.stew_effects
    }
    fn ominous_amplifier(&self) -> Option<i32> {
        self.ominous_amplifier
    }
    fn exploration(&self) -> Option<LootExplorationView<'_>> {
        let map = self.exploration.as_ref()?;
        Some(LootExplorationView::new(
            &map.destination,
            &map.decoration,
            map.zoom,
            map.search_radius,
            map.skip_existing_chunks,
        ))
    }
    fn names(&self) -> &[(String, Value)] {
        &self.names
    }
}

impl LootStackEditAccess for TestStack {
    fn accepts_component(&self, id: &str, value: &Tag) -> bool {
        id != "minecraft:trim" || matches!(value, Tag::Compound(_))
    }
    fn as_view(&self) -> &dyn LootStackAccess {
        self
    }
    fn set_item_id(&mut self, id: &str) {
        self.item_id = id.to_owned();
    }
    fn set_count(&mut self, count: i32) {
        self.count = count;
    }
    fn set_damage(&mut self, damage: i32) {
        self.damage = damage;
    }
    fn set_map(&mut self, map: bool) {
        self.map = map;
    }
    fn set_ominous_amplifier(&mut self, amplifier: Option<i32>) {
        self.ominous_amplifier = amplifier;
    }
    fn set_potion_id(&mut self, potion: Option<String>) {
        self.potion = potion;
    }
    fn set_exploration(&mut self, exploration: Option<ExplorationMap>) {
        self.exploration = exploration;
    }
    fn push_enchantment(&mut self, id: String, level: i32) {
        self.enchantments.push((id, level));
    }
    fn set_enchantment(&mut self, id: String, level: i32) {
        match self.enchantments.iter_mut().find(|(name, _)| *name == id) {
            Some(entry) => entry.1 = level,
            None => self.enchantments.push((id, level)),
        }
    }
    fn set_enchantments(&mut self, enchantments: Vec<(String, i32)>) {
        self.enchantments = enchantments;
    }
    fn clear_enchantments(&mut self) {
        self.enchantments.clear();
    }
    fn push_stew_effect(&mut self, id: String, duration_ticks: i32) {
        self.stew_effects.push((id, duration_ticks));
    }
    fn set_stew_effects(&mut self, effects: Vec<(String, i32)>) {
        self.stew_effects = effects;
    }
    fn clear_stew_effects(&mut self) {
        self.stew_effects.clear();
    }
    fn push_name_value(&mut self, id: String, value: Value) {
        self.names.push((id, value));
    }
    fn set_name_value(&mut self, id: String, value: Value) {
        self.names.retain(|(existing, _)| *existing != id);
        self.names.push((id, value));
    }
    fn set_names_values(&mut self, names: Vec<(String, Value)>) {
        self.names = names;
    }
    fn clear_names(&mut self) {
        self.names.clear();
    }
    fn push_component(&mut self, id: String, value: Option<Tag>) {
        self.components.push((id, value));
    }
    fn set_components(&mut self, components: Vec<(String, Option<Tag>)>) {
        self.components = components;
    }
    fn clear_components(&mut self) {
        self.components.clear();
    }
    fn retain_components(&mut self, keep: &mut dyn FnMut(&str, Option<&Tag>) -> bool) {
        self.components
            .retain(|(id, value)| keep(id.as_str(), value.as_ref()));
    }
}

impl LootListAccess for Vec<TestStack> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&self, index: usize) -> Option<&dyn LootStackAccess> {
        self.as_slice()
            .get(index)
            .map(|stack| stack as &dyn LootStackAccess)
    }
    fn get_mut(&mut self, index: usize) -> Option<&mut dyn LootStackEditAccess> {
        self.as_mut_slice()
            .get_mut(index)
            .map(|stack| stack as &mut dyn LootStackEditAccess)
    }
    fn push_empty(&mut self, item_id: &str) -> &mut dyn LootStackEditAccess {
        self.push(TestStack::new(item_id, 1));
        self.last_mut().expect("just pushed")
    }
    fn insert_empty(
        &mut self,
        index: usize,
        item_id: &str,
    ) -> Option<&mut dyn LootStackEditAccess> {
        if index > self.len() {
            return None;
        }
        self.insert(index, TestStack::new(item_id, 1));
        Some(&mut self[index])
    }
    fn remove(&mut self, index: usize) -> bool {
        if index >= self.len() {
            return false;
        }
        Vec::remove(self, index);
        true
    }
    fn clear(&mut self) {
        Vec::clear(self);
    }
    fn truncate(&mut self, len: usize) {
        Vec::truncate(self, len);
    }
    fn swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.len() || b >= self.len() {
            return false;
        }
        self.as_mut_slice().swap(a, b);
        true
    }
    fn reverse(&mut self) {
        self.as_mut_slice().reverse();
    }
    fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.len() || to >= self.len() {
            return false;
        }
        match from.cmp(&to) {
            std::cmp::Ordering::Less => self[from..=to].rotate_left(1),
            std::cmp::Ordering::Greater => self[to..=from].rotate_right(1),
            std::cmp::Ordering::Equal => {}
        }
        true
    }
}

fn test_loot_list(stacks: &mut Vec<TestStack>) -> LootList<'_> {
    LootList::new(
        stacks,
        |_| Err(TextFormError::Unavailable),
        |_| Err(TextFormError::Unavailable),
    )
}

struct Roll(LegacyRandomSource);
impl LootModifierContext for Roll {
    fn params(&self) -> LootParamsView<'_> {
        LootParamsView::new(
            crate::__private::LootParamsParts {
                queried_table_id: Some("test:root"),
                ..Default::default()
            },
            |_| Err(TextFormError::Unavailable),
        )
    }
    fn random(&mut self) -> &mut dyn RandomSource {
        self
    }
    fn roll_raw(
        &mut self,
        table: &Identifier,
        visit: &mut dyn FnMut(LootStackView<'_>),
    ) -> Result<(), LootModifierError> {
        assert_eq!(table.to_string(), "test:raw");
        let raw = TestStack::new("minecraft:gold_ingot", 0);
        visit(LootStackView::new(&raw, |_| {
            Err(TextFormError::Unavailable)
        }));
        Ok(())
    }
    fn append_raw(
        &mut self,
        table: &Identifier,
        generated: &mut LootList<'_>,
    ) -> Result<(), LootModifierError> {
        assert_eq!(table.to_string(), "test:raw");
        generated
            .access_mut()
            .push_empty("minecraft:gold_ingot")
            .set_count(0);
        Ok(())
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
        generated: &mut LootList<'_>,
    ) -> Result<(), LootModifierError> {
        assert_eq!(context.params().queried_table_id(), "test:root");
        generated.reverse();
        let draw = context.random().next_int_bound(2);
        let mut second = generated.get_mut(1).unwrap();
        let count = second.view().count() - draw;
        second.set_count(count);
        let raw_id = Identifier::parse("test:raw").unwrap();
        let mut seen = Vec::new();
        context.roll_raw(&raw_id, &mut |stack| {
            seen.push((stack.item_id().to_owned(), stack.count()));
        })?;
        assert_eq!(seen, [("minecraft:gold_ingot".to_owned(), 0)]);
        context.append_raw(&raw_id, generated)?;
        assert!(!generated.remove(99));
        assert!(!generated.reorder(0, 99));
        assert!(generated.get(99).is_none());
        assert!(generated.get_mut(99).is_none());
        Ok(())
    }
}

#[test]
fn preparation_caches_metadata_and_application_keeps_signed_output_and_roll_random() {
    let reads = Arc::new(AtomicUsize::new(0));
    let codec = owned_serializer(
        CallbackOwner::for_mod("test_glm_meta"),
        Arc::new(Codec(reads.clone())),
    );
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
    let mut stacks = vec![
        TestStack::new("minecraft:stick", -7),
        TestStack::new("minecraft:diamond", 130),
    ];
    let mut list = test_loot_list(&mut stacks);
    prepared.apply(&mut roll, &mut list).unwrap();
    assert_eq!(
        stacks
            .iter()
            .map(|stack| (stack.item_id.as_str(), stack.count))
            .collect::<Vec<_>>(),
        [
            ("minecraft:diamond", 130),
            ("minecraft:stick", -8),
            ("minecraft:gold_ingot", 0),
        ]
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
        _: &mut LootList<'_>,
    ) -> Result<(), LootModifierError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("native fixture callback panics");
    }
}
#[test]
fn a_native_panic_disables_the_registration_owner_across_instances_and_reloads() {
    let calls = Arc::new(AtomicUsize::new(0));
    let owner = CallbackOwner::for_mod("test_glm_panic");
    let codec = owned_serializer(owner.clone(), Arc::new(Panics(calls.clone())));
    let reloaded = owned_serializer(owner, Arc::new(Panics(calls.clone())));
    let load = OwningLoad::new(crate::DataLoadOrigin::ServerDataLoad, &Reader);
    let first = codec.decode(&json!({}), &load).unwrap();
    let second = codec.decode(&json!({}), &load).unwrap();
    let mut roll = Roll(LegacyRandomSource::new(0));
    let mut stacks = Vec::new();
    let mut list = test_loot_list(&mut stacks);
    assert!(
        first
            .apply(&mut roll, &mut list)
            .unwrap_err()
            .reason
            .contains("panicked")
    );
    assert!(
        first
            .apply(&mut roll, &mut list)
            .unwrap_err()
            .reason
            .contains("disabled")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        second
            .apply(&mut roll, &mut list)
            .unwrap_err()
            .reason
            .contains("disabled")
    );
    assert!(
        reloaded
            .decode(&json!({}), &load)
            .err()
            .unwrap()
            .reason
            .contains("disabled")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct PartialRefusal(Arc<AtomicUsize>);
impl GlobalLootModifierSerializer for PartialRefusal {
    fn decode(
        &self,
        _: &Value,
        _: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        Ok(Arc::new(PartialRefusal(self.0.clone())))
    }
}
impl GlobalLootModifier for PartialRefusal {
    fn apply(
        &self,
        _: &mut dyn LootModifierContext,
        generated: &mut LootList<'_>,
    ) -> Result<(), LootModifierError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let mut first = generated.get_mut(0).unwrap();
        let next = first.view().count() - 1;
        first.set_count(next);
        assert!(generated.reorder(0, 1));
        Err(LootModifierError::new("apply", "ordinary refusal"))
    }
}
#[test]
fn ordinary_refusal_reports_each_time_and_keeps_prior_valid_edits() {
    let calls = Arc::new(AtomicUsize::new(0));
    let owner = CallbackOwner::for_mod("test_glm_refusal");
    let codec = owned_serializer(owner, Arc::new(PartialRefusal(calls.clone())));
    let load = OwningLoad::new(crate::DataLoadOrigin::ServerDataLoad, &Reader);
    let prepared = codec.decode(&json!({}), &load).unwrap();
    let mut roll = Roll(LegacyRandomSource::new(0));
    let mut stacks = vec![
        TestStack::new("minecraft:stick", 10),
        TestStack::new("minecraft:diamond", 20),
    ];
    for _ in 0..2 {
        let mut list = test_loot_list(&mut stacks);
        let error = prepared.apply(&mut roll, &mut list).unwrap_err();
        assert_eq!(error, LootModifierError::new("apply", "ordinary refusal"));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        stacks
            .iter()
            .map(|stack| (stack.item_id.as_str(), stack.count))
            .collect::<Vec<_>>(),
        [("minecraft:stick", 9), ("minecraft:diamond", 19)]
    );
}

/// A component patch the evaluator cannot encode or apply is refused before any mutation:
/// an embedded `Tag::End` and a payload shape the modeled component rejects.
#[test]
fn unencodable_or_misshapen_component_patches_leave_the_candidate_unchanged() {
    use crate::{LootStackEditError, LootStackEditor};
    let mut stack = TestStack::new("minecraft:stick", 1);
    let mut editor = LootStackEditor::new(
        &mut stack,
        |_| Err(TextFormError::Unavailable),
        |_| Err(TextFormError::Unavailable),
    );
    let embedded_end = Tag::compound([("inner", Tag::End)]);
    let end_in_list = Tag::List(vec![Tag::End]);
    for bad in [
        embedded_end.clone(),
        end_in_list,
        Tag::String("minecraft:quartz".into()),
    ] {
        let id = if matches!(bad, Tag::String(_)) {
            "minecraft:trim"
        } else {
            "test:value"
        };
        assert_eq!(
            editor.push_component(id, bad.clone()),
            Err(LootStackEditError::InvalidPatch)
        );
        assert_eq!(
            editor.set_components([(id.to_owned(), Some(bad))]),
            Err(LootStackEditError::InvalidPatch)
        );
    }
    assert_eq!(editor.view().components().len(), 0);
    assert!(
        editor
            .push_component(
                "minecraft:trim",
                Tag::compound([("material", Tag::Byte(1))])
            )
            .is_ok()
    );
}
