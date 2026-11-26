//! Item predicate matching shared by filtered loot functions in both evaluators.
use super::{LootStack, LootStackAccess, custom_data_matches, int_bounds};
use serde_json::Value;
use sunrise_core::nbt::Tag;

/// Match the predicate against a candidate, borrowing the evaluator's authoritative tags.
pub fn matches(
    predicate: &Value,
    stack: &dyn LootStackAccess,
    contains_tag: impl Fn(&str, &str, &str) -> bool,
) -> bool {
    if !predicate["items"].is_null()
        && !in_set("item", &predicate["items"], stack.item_id(), &contains_tag)
    {
        return false;
    }
    if !int_bounds(&predicate["count"], stack.count()) {
        return false;
    }
    let predicates = &predicate["predicates"];
    if predicates.get("minecraft:map_id").is_some() && !stack.map() {
        return false;
    }
    if let Some(pattern) = predicates.get("minecraft:custom_data") {
        let data = stack
            .components()
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
    if let Some(container) = predicates.get("minecraft:container")
        && !container_matches(&container["items"], stack, &contains_tag)
    {
        return false;
    }
    let enchantments = stack.enchantments();
    predicates["minecraft:enchantments"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|entry| {
            if entry["enchantments"].is_null() {
                enchantments
                    .iter()
                    .any(|(_, level)| int_bounds(&entry["levels"], *level))
            } else {
                enchantments.iter().any(|(id, _)| {
                    let level = enchantments
                        .iter()
                        .find(|(name, _)| name == id)
                        .map_or(0, |(_, level)| *level);
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

/// `ContainerPredicate`, a `SingleComponentItemPredicate`: a stack without the
/// `minecraft:container` component fails. Its `items` is a `CollectionPredicate` over the
/// container's non-empty stacks: every `contains` test holds for some stack
/// (`CollectionContentsPredicate`), each `count` test holds for a number of stacks in its
/// bounds (`CollectionCountsPredicate`), and the stack count is in `size`.
fn container_matches(
    items: &Value,
    stack: &dyn LootStackAccess,
    contains_tag: &dyn Fn(&str, &str, &str) -> bool,
) -> bool {
    let Some(Some(Tag::List(entries))) = stack
        .components()
        .iter()
        .rev()
        .find(|(id, _)| id == "minecraft:container")
        .map(|(_, value)| value.as_ref())
    else {
        return false;
    };
    let stacks: Vec<LootStack> = entries
        .iter()
        .filter_map(|entry| contained_stack(entry.get("item")?))
        .collect();
    let test = |predicate: &Value, stack: &LootStack| matches(predicate, stack, contains_tag);
    let contains = items["contains"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|p| stacks.iter().any(|stack| test(p, stack)));
    let counts = items["count"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|entry| {
            let n = stacks
                .iter()
                .filter(|stack| test(&entry["test"], stack))
                .count();
            int_bounds(&entry["count"], n as i32)
        });
    contains && counts && int_bounds(&items["size"], stacks.len() as i32)
}

/// A stack saved in a container component (`ItemStack.CODEC`: `id`, `count` defaulting to
/// 1, `components`), as the evaluator reads it. Air and empty stacks are left out.
fn contained_stack(item: &Tag) -> Option<LootStack> {
    let Some(Tag::String(id)) = item.get("id") else {
        return None;
    };
    let count = match item.get("count") {
        None => 1,
        Some(Tag::Int(n)) => *n,
        Some(Tag::Byte(n)) => i32::from(*n),
        Some(Tag::Short(n)) => i32::from(*n),
        Some(_) => return None,
    };
    let mut stack = LootStack::new(&id.to_string_lossy());
    stack.count = count;
    if let Some(Tag::Compound(components)) = item.get("components") {
        stack.enchantments = enchantment_levels(components.get(&"minecraft:enchantments".into()));
        stack.map = components.contains_key(&"minecraft:map_id".into());
        stack.components = components
            .iter()
            .map(|(id, value)| (id.to_string_lossy(), Some(value.clone())))
            .collect();
    }
    (!stack.is_empty()).then_some(stack)
}

/// The levels of a `minecraft:enchantments` component (`ItemEnchantments.CODEC`: enchantment
/// to level).
pub(super) fn enchantment_levels(component: Option<&Tag>) -> Vec<(String, i32)> {
    let Some(Tag::Compound(levels)) = component else {
        return Vec::new();
    };
    levels
        .iter()
        .filter_map(|(id, level)| match level {
            Tag::Int(level) => Some((id.to_string_lossy(), *level)),
            _ => None,
        })
        .collect()
}
