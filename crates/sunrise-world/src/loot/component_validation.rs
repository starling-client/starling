//! Codec-boundary validation before publishing persistent loot and predicates.
use super::*;
use crate::item_defaults::validate_component_map;

fn keyed<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .as_object()
        .and_then(|map| {
            map.iter().find_map(|(key, value)| {
                (key.strip_prefix("minecraft:").unwrap_or(key) == name).then_some(value)
            })
        })
        .unwrap_or(&Value::Null)
}
fn kind(value: &Value) -> &str {
    let name = value
        .get("type")
        .or_else(|| value.get("condition"))
        .or_else(|| value.get("function"))
        .and_then(Value::as_str)
        .unwrap_or("");
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// `DataComponentMatchers.CODEC`, used by item and block predicates. Partial
/// any-value predicates name a type, not a persistent value, and remain valid.
pub fn validate_item_predicate(value: &Value) -> Result<(), String> {
    validate_component_map(&value["components"])?;
    validate_partial_predicates(&value["predicates"])
}
fn validate_partial_predicates(value: &Value) -> Result<(), String> {
    for name in ["container", "bundle_contents", "charged_projectiles"] {
        let collection = &keyed(value, name)["items"];
        for item in collection["contains"].as_array().into_iter().flatten() {
            validate_item_predicate(item)?;
        }
        for count in collection["count"].as_array().into_iter().flatten() {
            validate_item_predicate(&count["test"])?;
        }
    }
    Ok(())
}

/// `BlockPredicate.CODEC`: its component matchers, and its `nbt` read as a compound
/// (`NbtPredicate.CODEC`, SNBT text or a JSON object).
fn validate_block_predicate(value: &Value) -> Result<(), String> {
    validate_item_predicate(value)?;
    match &value["nbt"] {
        Value::Null => Ok(()),
        nbt => match super::nbt_predicate_tag(nbt) {
            Some(Tag::Compound(_)) => Ok(()),
            _ => Err(format!("nbt: not a compound: {nbt}")),
        },
    }
}

/// `EntityPredicate.CODEC`'s registered component-bearing sub-predicates.
/// Unknown/native sub-predicate payloads and NBT are intentionally opaque.
pub fn validate_entity_predicate(value: &Value) -> Result<(), String> {
    validate_component_map(keyed(value, "components"))?;
    validate_partial_predicates(keyed(value, "predicates"))?;
    for name in ["equipment", "slots"] {
        for item in keyed(value, name)
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
        {
            validate_item_predicate(item)?;
        }
    }
    for name in ["vehicle", "passenger", "targeted_entity"] {
        validate_entity_predicate_inner(keyed(value, name))?;
    }
    validate_entity_predicate_inner(&keyed(value, "type_specific/player")["looking_at"])?;
    validate_entity_predicate_inner(&keyed(value, "type_specific/lightning")["entity_struck"])?;
    for name in ["location", "stepping_on", "movement_affected_by"] {
        validate_item_predicate(&keyed(value, name)["block"])?;
    }
    Ok(())
}
fn validate_entity_predicate_inner(value: &Value) -> Result<(), String> {
    if !value.is_null() {
        validate_entity_predicate(value)?;
    }
    Ok(())
}

/// Loot condition dispatch, including nested condition and entity codecs.
pub fn validate_loot_condition(value: &Value) -> Result<(), String> {
    validate_condition(value, false)
}

/// [`validate_loot_condition`], and with `typed` every condition, nested ones included, must
/// name its kind under `type` ([`validate_typed_loot_condition`]).
fn validate_condition(value: &Value, typed: bool) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            validate_condition(value, typed)?;
        }
        return Ok(());
    }
    if typed && value.is_object() && !value.get("type").is_some_and(Value::is_string) {
        return Err("loot condition has no type".into());
    }
    match kind(value) {
        "match_tool" => validate_item_predicate(&value["predicate"]),
        "entity_properties" => validate_entity_predicate(&value["predicate"]),
        // `LootItemCondition.CODEC`: a nested condition dispatches on `type` too.
        "all_of" | "any_of" => validate_condition(&value["terms"], typed),
        "inverted" => validate_condition(&value["term"], typed),
        "damage_source_properties" => {
            validate_entity_predicate(&value["predicate"]["direct_entity"])?;
            validate_entity_predicate(&value["predicate"]["source_entity"])
        }
        "location_check" => {
            super::location_offset(value)?;
            validate_block_predicate(&value["predicate"]["block"])
        }
        "match_block" => validate_block_predicate(value),
        "random_chance" => super::number_validation::number(&value["chance"]),
        "int_value_check" | "float_value_check" => {
            super::number_validation::number(&value["value"])?;
            super::number_validation::range(&value["test"])
        }
        "time_check" => super::number_validation::range(&value["value"]),
        "entity_scores" => {
            for score in value["scores"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
            {
                super::number_validation::range(score)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// A condition of a loot table, an item modifier or a predicate file
/// (`LootItemCondition.DIRECT_CODEC`, which dispatches on `type`): an object without a `type`,
/// such as pre-26.3 data's `{"condition": "minecraft:random_chance"}`, fails to decode, so the
/// file that holds it is refused. A string is a reference to a predicate file. Conditions
/// nested in `all_of`, `any_of`, `inverted` and in number providers (`conditional`,
/// `number_dispatcher`) decode through `LootItemCondition.CODEC` and need their `type` too.
pub fn validate_typed_loot_condition(value: &Value) -> Result<(), String> {
    validate_condition(value, true)
}

/// Item-modifier dispatch; custom function data and `set_custom_data.tag` do
/// not contain component maps just because they have a key named components.
pub fn validate_item_modifier(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            validate_item_modifier(value)?;
        }
        return Ok(());
    }
    validate_typed_loot_condition(&value["condition"])?;
    super::number_validation::modifier(value)?;
    match kind(value) {
        "set_components" => validate_component_map(&value["components"]),
        "set_name" => crate::item_defaults::validate_text(&value["name"]),
        "set_lore" => crate::item_defaults::validate_text(&value["lore"]),
        "set_written_book_pages" => {
            crate::item_defaults::validate_component_text("written_book_content", value)
        }
        "set_attributes" => crate::item_defaults::validate_component_text(
            "attribute_modifiers",
            &value["modifiers"],
        ),
        "sequence" => validate_item_modifier(&value["functions"]),
        "modify_contents" => validate_item_modifier(&value["modifier"]),
        "filtered" => {
            validate_item_predicate(&value["item_filter"])?;
            validate_item_modifier(&value["on_pass"])?;
            validate_item_modifier(&value["on_fail"])
        }
        "set_contents" => {
            for entry in value["entries"].as_array().into_iter().flatten() {
                validate_entry(entry)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn validate_modifiers_and_conditions(value: &Value) -> Result<(), String> {
    validate_item_modifier(&value["modifier"])?;
    validate_typed_loot_condition(&value["condition"])
}
fn validate_entry(value: &Value) -> Result<(), String> {
    validate_modifiers_and_conditions(value)?;
    match kind(value) {
        "alternatives" | "group" | "sequence" => {
            for child in value["children"].as_array().into_iter().flatten() {
                validate_entry(child)?;
            }
        }
        "loot_table" if value["value"].is_object() => validate_loot_table(&value["value"])?,
        _ => {}
    }
    Ok(())
}
pub fn validate_loot_table(value: &Value) -> Result<(), String> {
    validate_item_modifier(&value["modifier"])?;
    for pool in value["pools"].as_array().into_iter().flatten() {
        super::number_validation::number(&pool["rolls"])?;
        super::number_validation::number(&pool["bonus_rolls"])?;
        validate_modifiers_and_conditions(pool)?;
        for entry in pool["entries"].as_array().into_iter().flatten() {
            validate_entry(entry)?;
        }
    }
    Ok(())
}
/// The entries a codec refuses, as `<kind> <id>: <reason>` lines.
pub(super) fn refused_entries<'a>(
    kind: &str,
    entries: impl IntoIterator<Item = (&'a String, &'a Value)>,
    validate: impl Fn(&Value) -> Result<(), String>,
) -> Vec<String> {
    entries
        .into_iter()
        .filter_map(|(id, value)| {
            validate(value)
                .err()
                .map(|error| format!("{kind} {id}: {error}"))
        })
        .collect()
}

/// `RegistryDataLoader.logErrors`: any refused entry fails the whole load, and
/// every one is listed, in order.
pub(super) fn refuse(mut errors: Vec<String>) -> std::io::Result<()> {
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        errors.join("\n"),
    ))
}

/// `SlotSources.TYPED_CODEC`, the registry file codec: an object whose `type` is
/// a registered `SLOT_SOURCE_TYPE`. A string reference and an inline group exist
/// only in nested fields (`SlotSources.CODEC`).
pub fn validate_slot_source(value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("expected a typed slot source object".into());
    }
    validate_nested_slot_source(value)
}

/// `SlotSources.CODEC`: a registered `SLOT_SOURCE_TYPE`, a reference by id or an
/// inline group; only the stock filtered source owns an item predicate.
fn validate_nested_slot_source(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            validate_nested_slot_source(value)?;
        }
        return Ok(());
    }
    if value.is_string() {
        return Ok(());
    }
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "group" => validate_nested_slot_source(&value["terms"]),
        "contents" | "limit_slots" => validate_nested_slot_source(&value["slot_source"]),
        "filtered" => {
            validate_item_predicate(&value["item_filter"])?;
            validate_nested_slot_source(&value["slot_source"])
        }
        "slot_range" | "empty" => Ok(()),
        _ => Err(format!("unknown slot source type {kind:?}")),
    }
}

#[cfg(test)]
mod slot_source_tests {
    use super::validate_slot_source;
    use serde_json::json;

    #[test]
    fn a_registry_file_is_a_typed_object() {
        for file in [
            json!({"type": "minecraft:empty"}),
            json!({"type": "minecraft:group", "terms": ["example:a", [{"type": "minecraft:empty"}]]}),
        ] {
            assert_eq!(validate_slot_source(&file), Ok(()), "{file}");
        }
        for file in [
            json!("example:a"),
            json!([{"type": "minecraft:empty"}]),
            json!({"function": "minecraft:empty"}),
            json!({"condition": "minecraft:empty"}),
            json!({"type": "example:nope"}),
        ] {
            assert!(validate_slot_source(&file).is_err(), "{file}");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// `LootItemCondition.DIRECT_CODEC` dispatches on `type` (#5378): a table, an item modifier
    /// or a predicate file whose condition names its kind under the pre-26.3 `condition` key is
    /// refused; the same condition with `type`, or a reference to a predicate, loads.
    #[test]
    fn a_condition_without_a_type_refuses_its_file() {
        let old = json!({"condition":"minecraft:random_chance","chance":0.1});
        let typed = json!({"type":"minecraft:random_chance","chance":0.1});
        let table = |condition| {
            json!({"pools":[{"rolls":1,"condition":condition,
                "entries":[{"type":"minecraft:item","name":"minecraft:stone"}]}]})
        };
        assert!(super::validate_loot_table(&table(old.clone())).is_err());
        assert!(super::validate_loot_table(&table(typed.clone())).is_ok());
        assert!(super::validate_loot_table(&table(json!("minecraft:p"))).is_ok());
        let modifier =
            |condition| json!({"type":"minecraft:set_count","count":2,"condition":condition});
        assert!(super::validate_item_modifier(&modifier(old.clone())).is_err());
        assert!(super::validate_item_modifier(&modifier(typed.clone())).is_ok());
        assert!(super::validate_typed_loot_condition(&json!([typed, old])).is_err());
    }

    /// A condition nested in another (`all_of`, `any_of`, `inverted`) or in a number provider
    /// (`conditional`, `number_dispatcher`) decodes through `LootItemCondition.CODEC` and needs
    /// its `type` as well, however deep.
    #[test]
    fn a_nested_condition_without_a_type_refuses_its_file() {
        let old = json!({"condition":"minecraft:random_chance","chance":0.1});
        let typed = json!({"type":"minecraft:random_chance","chance":0.1});
        let positions: [fn(serde_json::Value) -> serde_json::Value; 5] = [
            |c| json!({"type":"minecraft:all_of","terms":[c]}),
            |c| json!({"type":"minecraft:any_of","terms":[c]}),
            |c| json!({"type":"minecraft:inverted","term":c}),
            |c| {
                json!({"type":"minecraft:random_chance","chance":
                {"type":"minecraft:conditional","condition":c,"on_true":0.5,"on_false":0.1}})
            },
            |c| {
                json!({"type":"minecraft:random_chance","chance":
                {"type":"minecraft:number_dispatcher","cases":[{"condition":c,"value":0.5}],
                 "default":0.1}})
            },
        ];
        for wrap in positions {
            assert!(super::validate_typed_loot_condition(&wrap(typed.clone())).is_ok());
            assert!(super::validate_typed_loot_condition(&wrap(old.clone())).is_err());
            // Two levels down.
            let inner = json!({"type":"minecraft:inverted","term":old.clone()});
            assert!(super::validate_typed_loot_condition(&wrap(inner)).is_err());
        }
    }
}
