//! Read adapters for the evaluator's existing parameter rows.
use super::{LootDamage, LootEntity, LootLevel, LootStack};
use mod_api::{
    __private::{
        LootDamageAccess, LootEntityAccess, LootLevelAccess, LootListAccess, LootStackAccess,
        LootStackEditAccess,
    },
    ExplorationMap as ApiExplorationMap, LootExplorationView,
};
use serde_json::Value;
use sunrise_core::nbt::Tag;

impl mod_api::__private::LootBlockProperties for crate::chunk::BlockProperties {
    fn property(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

impl super::LootToolAccess for LootStack {
    fn item_id(&self) -> &str {
        &self.item
    }

    fn enchantment_at(&self, index: usize) -> Option<(&str, i32)> {
        self.enchantments
            .get(index)
            .map(|(id, level)| (id.as_str(), *level))
    }
}

impl LootStackAccess for LootStack {
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
        let map = self.exploration.as_deref()?;
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

impl LootStackEditAccess for LootStack {
    fn as_view(&self) -> &dyn LootStackAccess {
        self
    }

    fn set_item_id(&mut self, id: &str) {
        self.item = id.to_owned();
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

    fn set_exploration(&mut self, exploration: Option<ApiExplorationMap>) {
        self.exploration = exploration.map(|map| {
            Box::new(super::ExplorationMap {
                destination: map.destination,
                decoration: map.decoration,
                zoom: map.zoom,
                search_radius: map.search_radius,
                skip_existing_chunks: map.skip_existing_chunks,
            })
        });
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

/// World-owned backing for the world evaluator's live output list.
#[derive(Default)]
pub struct LootStacks(pub Vec<LootStack>);

impl LootListAccess for LootStacks {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, index: usize) -> Option<&dyn LootStackAccess> {
        self.0
            .as_slice()
            .get(index)
            .map(|stack| stack as &dyn LootStackAccess)
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut dyn LootStackEditAccess> {
        self.0
            .as_mut_slice()
            .get_mut(index)
            .map(|stack| stack as &mut dyn LootStackEditAccess)
    }

    fn push_empty(&mut self, item_id: &str) -> &mut dyn LootStackEditAccess {
        self.0.push(LootStack::new(item_id));
        self.0.last_mut().expect("just pushed")
    }

    fn insert_empty(
        &mut self,
        index: usize,
        item_id: &str,
    ) -> Option<&mut dyn LootStackEditAccess> {
        if index > self.len() {
            return None;
        }
        self.0.insert(index, LootStack::new(item_id));
        Some(&mut self.0[index])
    }

    fn remove(&mut self, index: usize) -> bool {
        if index >= self.len() {
            return false;
        }
        self.0.remove(index);
        true
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    fn swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.len() || b >= self.len() {
            return false;
        }
        self.0.as_mut_slice().swap(a, b);
        true
    }

    fn reverse(&mut self) {
        self.0.as_mut_slice().reverse();
    }

    fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.len() || to >= self.len() {
            return false;
        }
        match from.cmp(&to) {
            std::cmp::Ordering::Less => self.0[from..=to].rotate_left(1),
            std::cmp::Ordering::Greater => self.0[to..=from].rotate_right(1),
            std::cmp::Ordering::Equal => {}
        }
        true
    }
}

impl LootEntityAccess for LootEntity {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn living(&self) -> bool {
        self.living
    }

    fn on_fire(&self) -> bool {
        self.on_fire
    }

    fn baby(&self) -> bool {
        self.baby
    }

    fn vehicle(&self) -> Option<&dyn LootEntityAccess> {
        self.vehicle
            .as_deref()
            .map(|entity| entity as &dyn LootEntityAccess)
    }

    fn cube_size(&self) -> Option<i32> {
        self.cube_size
    }

    fn sheep(&self) -> Option<(&str, bool)> {
        self.sheep
            .as_ref()
            .map(|(color, sheared)| (color.as_str(), *sheared))
    }

    fn captain(&self) -> Option<bool> {
        self.captain
    }

    fn frog_variant(&self) -> Option<&str> {
        self.frog_variant.as_deref()
    }

    fn chicken_variant(&self) -> Option<&str> {
        self.chicken_variant.as_deref()
    }

    fn powered(&self) -> bool {
        self.powered
    }

    fn equipment(&self, slot: &str) -> Option<&dyn LootStackAccess> {
        self.equipment
            .iter()
            .find(|(name, _)| name == slot)
            .map(|(_, stack)| stack as &dyn LootStackAccess)
    }

    fn equipment_at(&self, index: usize) -> Option<(&str, &dyn LootStackAccess)> {
        self.equipment
            .get(index)
            .map(|(slot, stack)| (slot.as_str(), stack as &dyn LootStackAccess))
    }
}

impl LootDamageAccess for LootDamage {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn source(&self) -> Option<&dyn LootEntityAccess> {
        self.source
            .as_ref()
            .map(|entity| entity as &dyn LootEntityAccess)
    }

    fn direct(&self) -> Option<&dyn LootEntityAccess> {
        self.direct
            .as_ref()
            .map(|entity| entity as &dyn LootEntityAccess)
    }
}

impl LootLevelAccess for LootLevel {
    fn dimension(&self) -> &str {
        &self.dimension
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn day_time(&self) -> i64 {
        self.day_time
    }

    fn raining(&self) -> bool {
        self.raining
    }

    fn thundering(&self) -> bool {
        self.thundering
    }
}
