//! Item predicate matching shared by filtered loot functions in both evaluators.
use super::{LootStack, custom_data_matches, int_bounds};
use serde_json::Value;
use sunrise_core::nbt::Tag;

/// Match the predicate against a candidate, borrowing the evaluator's authoritative tags.
pub fn matches(
    predicate: &Value,
    stack: &LootStack,
    contains_tag: impl Fn(&str, &str, &str) -> bool,
) -> bool {
    if !predicate["items"].is_null()
        && !in_set("item", &predicate["items"], &stack.item, &contains_tag)
    {
        return false;
    }
    if !int_bounds(&predicate["count"], stack.count) {
        return false;
    }
    let predicates = &predicate["predicates"];
    if predicates.get("minecraft:map_id").is_some() && !stack.map {
        return false;
    }
    if let Some(pattern) = predicates.get("minecraft:custom_data") {
        let data = stack
            .components
            .iter()
            .rev()
            .find(|(id, _)| id == "minecraft:custom_data")
            .and_then(|(_, value)| value.as_ref());
        let data = match data {
            Some(Tag::Compound(data)) => Some(data),
            _ => None,
        };
        if !custom_data_matches(pattern, data) {
            return false;
        }
    }
    predicates["minecraft:enchantments"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|entry| {
            if entry["enchantments"].is_null() {
                stack
                    .enchantments
                    .iter()
                    .any(|(_, level)| int_bounds(&entry["levels"], *level))
            } else {
                stack.enchantments.iter().any(|(id, _)| {
                    let level = super::item_level(stack, id);
                    level != 0
                        && in_set("enchantment", &entry["enchantments"], id, &contains_tag)
                        && int_bounds(&entry["levels"], level)
                })
            }
        })
}

fn in_set(
    registry: &str,
    set: &Value,
    id: &str,
    contains_tag: &impl Fn(&str, &str, &str) -> bool,
) -> bool {
    match set {
        Value::String(value) => value
            .strip_prefix('#')
            .map_or(value == id, |tag| contains_tag(registry, tag, id)),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .any(|value| value == id),
        _ => false,
    }
}
