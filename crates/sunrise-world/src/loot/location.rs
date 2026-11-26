//! What `LocationCheck` and `MatchBlock` read of a roll: its origin and level, and its
//! block entity. Both loot engines evaluate these conditions here, so a block drop, a
//! container fill and `LevelRef::roll_loot` follow the same rules.
use serde_json::Value;
use sunrise_core::nbt::{Compound, Tag};

/// `LootContextParams.ORIGIN` and the level a `LocationCheck` reads at it. A roll
/// without an origin has none, and every location check in it fails.
#[derive(Clone, Copy)]
pub struct LootLocation<'a> {
    /// `ORIGIN`: a broken block's centre, a chest's centre, `/loot`'s position.
    pub origin: [f64; 3],
    /// `LocationPredicate.matches(level, x, y, z)` against the roll's level: position and
    /// dimension, then biome, structure piece, smoke, light, block and fluid with
    /// `Level.isLoaded`'s rule, and `Level.canSeeSky` on its own.
    pub level: &'a dyn Fn(&Value, [f64; 3]) -> bool,
}

impl LootLocation<'_> {
    /// `LocationCheck.test`: true with no predicate, else the predicate at the origin
    /// moved by the check's integer offsets. It draws nothing from the roll's random.
    pub fn check(&self, condition: &Value) -> bool {
        let predicate = &condition["predicate"];
        if predicate.is_null() {
            return true;
        }
        let Ok(offset) = location_offset(condition) else {
            return false;
        };
        let at = std::array::from_fn(|i| self.origin[i] + f64::from(offset[i]));
        (self.level)(predicate, at)
    }
}

/// `LocationCheck`'s `offsetX`, `offsetY` and `offsetZ`: `Codec.INT` fields that default
/// to zero, so a JSON number reads as Java's `Number.intValue` (a fraction truncated, a
/// wide integer wrapped to 32 bits) and anything else fails to load.
pub fn location_offset(condition: &Value) -> Result<[i32; 3], String> {
    let mut offset = [0; 3];
    for (axis, key) in ["offsetX", "offsetY", "offsetZ"].into_iter().enumerate() {
        offset[axis] = match condition.get(key) {
            None => 0,
            Some(Value::Number(number)) => super::number::int_value(number) as i32,
            Some(other) => return Err(format!("{key}: not a number: {other}")),
        };
    }
    Ok(offset)
}

/// The NBT counterpart of [`location_offset`], for a condition admitted from a command:
/// `NbtOps.getNumberValue` takes any numeric tag's `intValue`.
pub fn location_offset_tag(condition: &Tag) -> Result<[i32; 3], String> {
    let mut offset = [0; 3];
    for (axis, key) in ["offsetX", "offsetY", "offsetZ"].into_iter().enumerate() {
        offset[axis] = match condition.get(key) {
            None => 0,
            Some(Tag::Byte(v)) => i32::from(*v),
            Some(Tag::Short(v)) => i32::from(*v),
            Some(Tag::Int(v)) => *v,
            Some(Tag::Long(v)) => *v as i32,
            Some(Tag::Float(bits)) => f32::from_bits(*bits) as i32,
            Some(Tag::Double(bits)) => f64::from_bits(*bits) as i32,
            Some(_) => return Err(format!("{key}: not a number")),
        };
    }
    Ok(offset)
}

/// `BlockPredicate.willMatchBlockEntity`: the predicate has an NBT test or component
/// matchers, so it needs a block entity. Empty matchers do not.
pub fn reads_block_entity(predicate: &Value) -> bool {
    !predicate["nbt"].is_null()
        || matchers_present(&predicate["components"])
        || matchers_present(&predicate["predicates"])
}

fn matchers_present(value: &Value) -> bool {
    value.as_object().is_some_and(|map| !map.is_empty())
}

/// `BlockPredicate.matchesBlockEntity`: the NBT test against the block entity's
/// `saveWithFullMetadata` (`NbtPredicate.matches`, `NbtUtils.compareNbt` with partial
/// lists), then the component matchers against `BlockEntity.collectComponents`. A missing
/// block entity fails any test it would answer; a predicate with neither test holds.
/// `contains_tag(registry, tag, id)` answers the tags the component predicates name.
pub fn block_entity_matches(
    predicate: &Value,
    nbt: Option<&Tag>,
    components: impl FnOnce() -> Option<Compound>,
    contains_tag: &dyn Fn(&str, &str, &str) -> bool,
) -> bool {
    if !predicate["nbt"].is_null() {
        let Some(pattern @ Tag::Compound(_)) = super::nbt_predicate_tag(&predicate["nbt"]) else {
            return false;
        };
        if nbt.is_none() || !sunrise_core::nbt::compare_nbt(&pattern, nbt) {
            return false;
        }
    }
    if !matchers_present(&predicate["components"]) && !matchers_present(&predicate["predicates"]) {
        return true;
    }
    components().is_some_and(|components| components_match(predicate, &components, contains_tag))
}

/// `DataComponentMatchers.test` against a collected component map.
///
/// Every exact component (`DataComponentExactPredicate`) must be present and equal the
/// expected one as decoded values (`from_component_tag`, the component's own codec), so
/// `"stone"` and `"minecraft:stone"` are one id, while `minecraft:custom_data` keeps its tag
/// types and tries no other number type. The partial predicates
/// (`DataComponentPredicate`s) go through the evaluator item predicates use
/// (`item_predicates::matches`), over the map as a stack's components.
pub fn components_match(
    matchers: &Value,
    components: &Compound,
    contains_tag: &dyn Fn(&str, &str, &str) -> bool,
) -> bool {
    let exact = matchers["components"].as_object().into_iter().flatten();
    for (id, want) in exact {
        let id = full_id(id);
        let Some(have) = components.get(&id.as_str().into()) else {
            return false;
        };
        let want = if id == "minecraft:custom_data" {
            super::nbt_predicate_tag(want)
        } else {
            super::json_tag(want)
        };
        let decode = |tag: &Tag| crate::component_value::source::from_component_tag(&id, tag).ok();
        match (want.as_ref().and_then(decode), decode(have)) {
            (Some(want), Some(have)) if want == have => {}
            _ => return false,
        }
    }
    let Some(partial) = matchers["predicates"]
        .as_object()
        .filter(|map| !map.is_empty())
    else {
        return true;
    };
    let predicates: serde_json::Map<String, Value> = partial
        .iter()
        .map(|(id, pattern)| (full_id(id), pattern.clone()))
        .collect();
    let predicate = serde_json::json!({ "predicates": predicates });
    super::item_predicates::matches(&predicate, &component_stack(components), contains_tag)
}

/// The collected components as the item-predicate evaluator reads a stack: every component
/// in its patch, its `minecraft:enchantments` levels and whether it has a map id.
fn component_stack(components: &Compound) -> super::LootStack {
    let mut stack = super::LootStack::new("minecraft:air");
    stack.map = components.contains_key(&"minecraft:map_id".into());
    stack.enchantments = super::item_predicates::enchantment_levels(
        components.get(&"minecraft:enchantments".into()),
    );
    stack.components = components
        .iter()
        .map(|(id, value)| (id.to_string_lossy(), Some(value.clone())))
        .collect();
    stack
}

fn full_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `LocationCheck`'s offset codec is `Codec.INT`: a fraction truncates, a 33-bit value
    /// wraps, a negative one stays negative, and a string refuses to load.
    #[test]
    fn offsets_read_as_java_ints() {
        let read = |v: Value| location_offset(&v);
        assert_eq!(read(json!({})), Ok([0, 0, 0]));
        assert_eq!(
            read(json!({"offsetX": -3, "offsetY": 1.75, "offsetZ": 4_294_967_297_i64})),
            Ok([-3, 1, 1])
        );
        assert!(read(json!({"offsetX": "1"})).is_err());
    }

    /// The check moves the origin by whole blocks before asking the level, holds with no
    /// predicate, and asks nothing for it.
    #[test]
    fn check_moves_the_origin_by_the_offsets() {
        let asked = std::cell::Cell::new(None);
        let level = |_: &Value, at: [f64; 3]| {
            asked.set(Some(at));
            true
        };
        let location = LootLocation {
            origin: [0.5, 64.5, -0.5],
            level: &level,
        };
        assert!(location.check(&json!({"type": "minecraft:location_check"})));
        assert_eq!(asked.get(), None);
        assert!(location.check(&json!({"predicate": {}, "offsetX": -1, "offsetZ": 2})));
        assert_eq!(asked.get(), Some([-0.5, 64.5, 1.5]));
    }

    /// Exact components compare tags with their own types: `{k:1b}` is not `{k:1}`, and a
    /// JSON object's whole numbers take the smallest type, as `JsonOps` gives them.
    #[test]
    fn exact_custom_data_keeps_its_tag_types() {
        let components: Compound = [(
            "minecraft:custom_data".into(),
            Tag::compound([("k", Tag::Byte(1))]),
        )]
        .into();
        let matches = |v: Value| components_match(&v, &components, &|_, _, _| false);
        assert!(matches(json!({"components": {"custom_data": "{k:1b}"}})));
        assert!(matches(
            json!({"components": {"minecraft:custom_data": {"k": 1}}})
        ));
        assert!(!matches(json!({"components": {"custom_data": "{k:1}"}})));
        assert!(matches(json!({"predicates": {"custom_data": "{k:1b}"}})));
        assert!(!matches(json!({"predicates": {"custom_data": "{k:2b}"}})));
    }

    /// No block entity fails a predicate that reads one and holds one that does not.
    #[test]
    fn a_missing_block_entity_fails_only_tests_that_read_it() {
        let none = || None;
        let tags = &|_: &str, _: &str, _: &str| false;
        assert!(block_entity_matches(
            &json!({"blocks": "minecraft:chest"}),
            None,
            none,
            tags
        ));
        assert!(!reads_block_entity(&json!({"components": {}})));
        assert!(!block_entity_matches(
            &json!({"nbt": "{}"}),
            None,
            none,
            tags
        ));
        assert!(!block_entity_matches(
            &json!({"predicates": {"custom_data": "{}"}}),
            None,
            none,
            tags
        ));
        let chest = Tag::compound([("id", Tag::String("minecraft:chest".into()))]);
        assert!(block_entity_matches(
            &json!({"nbt": "{id:\"minecraft:chest\"}"}),
            Some(&chest),
            none,
            tags
        ));
    }

    /// Exact components compare decoded values: JSON's `3` (a byte as `JsonOps` gives it)
    /// is the saved `damage` int 3, which a tag comparison would refuse; another value or an
    /// absent component does not match.
    #[test]
    fn exact_components_compare_decoded_values() {
        let components: Compound = [
            ("minecraft:damage".into(), Tag::Int(3)),
            ("minecraft:base_color".into(), Tag::String("red".into())),
        ]
        .into();
        let matches = |v: Value| components_match(&v, &components, &|_, _, _| false);
        assert_eq!(super::super::json_tag(&json!(3)), Some(Tag::Byte(3)));
        assert!(matches(
            json!({"components": {"damage": 3, "minecraft:base_color": "red"}})
        ));
        assert!(!matches(json!({"components": {"minecraft:damage": 4}})));
        assert!(!matches(
            json!({"components": {"minecraft:base_color": "blue"}})
        ));
        assert!(!matches(
            json!({"components": {"minecraft:repair_cost": 3}})
        ));
    }

    /// A chest's `minecraft:container` answers the container predicate through the item
    /// predicate evaluator: `contains`, `count` and `size` over its stacks.
    #[test]
    fn a_container_predicate_reads_the_containers_stacks() {
        let stack = |slot: i32, id: &str, count: i32| {
            Tag::compound([
                ("slot", Tag::Int(slot)),
                (
                    "item",
                    Tag::compound([("id", Tag::String(id.into())), ("count", Tag::Int(count))]),
                ),
            ])
        };
        let components: Compound = [(
            "minecraft:container".into(),
            Tag::List(vec![
                stack(0, "minecraft:diamond", 3),
                stack(4, "minecraft:stone", 1),
            ]),
        )]
        .into();
        let matches = |v: Value| components_match(&v, &components, &|_, _, _| false);
        assert!(matches(
            json!({"predicates": {"minecraft:container": {"items": {
            "contains": [{"items": "minecraft:diamond", "count": {"min": 2}}]}}}})
        ));
        assert!(!matches(json!({"predicates": {"container": {"items": {
            "contains": [{"items": "minecraft:emerald"}]}}}})));
        assert!(matches(
            json!({"predicates": {"container": {"items": {"size": 2,
            "count": [{"test": {"items": "minecraft:stone"}, "count": 1}]}}}})
        ));
        assert!(!matches(
            json!({"predicates": {"container": {"items": {"size": {"min": 3}}}}})
        ));
        // `SingleComponentItemPredicate`: no container component, no match.
        assert!(!components_match(
            &json!({"predicates": {"container": {"items": {"size": 0}}}}),
            &Compound::new(),
            &|_, _, _| false
        ));
    }

    /// Invalid offsets and block-entity NBT fail on the loot load path, not in a roll.
    #[test]
    fn invalid_predicate_data_fails_to_load() {
        let check = crate::loot::validate_loot_condition;
        assert!(check(&json!({"type": "minecraft:location_check", "offsetX": "1"})).is_err());
        assert!(check(&json!({"type": "minecraft:location_check", "offsetY": -2})).is_ok());
        assert!(check(&json!({"type": "minecraft:match_block", "nbt": "{a:"})).is_err());
        assert!(check(&json!({"type": "minecraft:match_block", "nbt": "{a:1b}"})).is_ok());
        assert!(
            check(&json!({"type": "minecraft:location_check",
                "predicate": {"block": {"nbt": "[1]"}}}))
            .is_err()
        );
    }
}
