//! Structure processors with a vanilla data-pack form (or a no-op), carried
//! over as data the game runs:
//!
//! - Repurposed Structures' `structure_void_processor` becomes vanilla's
//!   `minecraft:block_ignore` on `minecraft:structure_void`.
//! - Moog's vanilla loot swap only acts when its mod's "replace vanilla"
//!   preset is on; one that ships off is a no-op and is taken out.
//!
//! Any other non-`minecraft:` processor requires a library mod that registers
//! its `STRUCTURE_PROCESSOR` type and is taken out of its list later and
//! counted (`worldgen::upgrade`). Entity NBT is upgraded to 26.3's shape (an
//! `equipment` map, not 1.21.1's `HandItems` and `ArmorItems`).
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek},
};

/// What the processors read from the mod's own data files.
#[derive(Debug, Default)]
pub struct ProcessorData {
    /// Moog's "replace vanilla" presets that are on by default:
    /// `(mod id, vanilla_key)`.
    pub replace_vanilla: BTreeSet<(String, String)>,
}

impl ProcessorData {
    pub fn read<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Self {
        let mut out = Self::default();
        let mut budget = crate::entry::Budget::archive();
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let Ok(name) = entry.name().map(|n| n.into_owned()) else {
                continue;
            };
            let Some((ns, rest)) = name
                .strip_prefix("data/")
                .and_then(|r| r.strip_suffix(".json"))
                .and_then(|r| r.split_once('/'))
            else {
                continue;
            };
            if rest != "moogs_structures/replace_vanilla" {
                continue;
            }
            let declared = entry.size();
            let Ok(v) = budget
                .read_text(&mut entry, declared, crate::entry::MAX_TEXT)
                .map_err(|_| ())
                .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|_| ()))
            else {
                continue;
            };
            for preset in v["presets"].as_array().into_iter().flatten() {
                if preset["default_enabled"].as_bool() != Some(true) {
                    continue;
                }
                for r in preset["replacements"].as_array().into_iter().flatten() {
                    if let Some(key) = r["vanilla_key"].as_str() {
                        out.replace_vanilla.insert((ns.to_owned(), key.to_owned()));
                    }
                }
            }
        }
        out
    }
}

/// Calls `f` on each processor list in a worldgen file: a `processor_list`
/// file, and the inline lists of a template pool's elements (list elements
/// included). Whether any call changed something.
pub fn each_list(kind: &str, v: &mut Value, f: &mut dyn FnMut(&mut Value) -> bool) -> bool {
    match kind {
        "processor_list" => f(v),
        "template_pool" => {
            let mut changed = false;
            for e in v
                .get_mut("elements")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                // Indexing would panic on an element that isn't an object.
                let Some(element) = e.get_mut("element") else {
                    continue;
                };
                if let Some(list) = element.get_mut("elements").and_then(Value::as_array_mut) {
                    for inner in list {
                        if let Some(p) = inner.get_mut("processors") {
                            changed |= f(p);
                        }
                    }
                } else if let Some(p) = element.get_mut("processors") {
                    changed |= f(p);
                }
            }
            changed
        }
        _ => false,
    }
}

/// The processors of a list: `{"processors": [...]}` or a bare list (an id
/// names another file and is left alone).
pub fn processors(list: &mut Value) -> Option<&mut Vec<Value>> {
    match list {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get_mut("processors").and_then(Value::as_array_mut),
        _ => None,
    }
}

/// Rewrites the processors of a worldgen file that have a data form; each is
/// counted by type in `carried`. Whether anything changed.
pub fn carry(
    kind: &str,
    v: &mut Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> bool {
    each_list(kind, v, &mut |list| {
        let Some(list) = processors(list) else {
            return false;
        };
        let (out, changed) = carry_all(std::mem::take(list), data, carried);
        *list = out;
        changed
    })
}

/// The processors of one list carried: each one with a vanilla data form
/// replaced by it. Whether anything changed.
pub(crate) fn carry_all(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    let mut changed = false;
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        let kind = p["processor_type"].as_str().unwrap_or_default().to_owned();
        match as_data(&kind, &p, data) {
            Some(replacement) => {
                *carried.entry(kind).or_default() += 1;
                out.extend(replacement);
                changed = true;
            }
            None => out.push(p),
        }
    }
    (out, changed)
}

/// Records non-`minecraft:` `predicate_type` ids found in a worldgen file's
/// processor lists under `unsupported`. Custom rule tests and position rule
/// tests require a mod that registers them via `RULE_TEST` / `POS_RULE_TEST`
/// (`register_rule_test_type` / `register_pos_rule_test_type`).
pub fn report_unsupported_rule_tests(
    kind: &str,
    v: &mut Value,
    unsupported: &mut BTreeMap<String, usize>,
) {
    fn scan(v: &Value, unsupported: &mut BTreeMap<String, usize>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::String(t)) = o.get("predicate_type")
                    && t.contains(':')
                    && !t.starts_with("minecraft:")
                {
                    *unsupported.entry(t.clone()).or_default() += 1;
                }
                for value in o.values() {
                    scan(value, unsupported);
                }
            }
            Value::Array(a) => {
                for value in a {
                    scan(value, unsupported);
                }
            }
            _ => {}
        }
    }
    each_list(kind, v, &mut |list| {
        scan(list, unsupported);
        false
    });
}

/// The data form of a Java processor: the processors to put in its place
/// (none for a no-op), or `None` when it has none.
fn as_data(kind: &str, p: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    kind.split_once(':')?;
    Some(match kind {
        // Its structure voids keep the world's block, as vanilla's
        // block_ignore does (the templates hold them; vanilla places them).
        "repurposed_structures:structure_void_processor" => vec![json!({
            "processor_type": "minecraft:block_ignore",
            "blocks": ["minecraft:structure_void"]})],
        "moogs_structures:vanilla_loot_swap_processor" => {
            let key = (
                p["modid"].as_str()?.to_owned(),
                p["vanilla_key"].as_str()?.to_owned(),
            );
            if data.replace_vanilla.contains(&key) {
                return None;
            }
            Vec::new()
        }
        _ => return None,
    })
}

/// An entity's NBT in 26.3's shape, in place: 1.21.1's `ArmorItems` (feet
/// to head), `HandItems` (main hand, off hand) and their drop chances become
/// `equipment` and `drop_chances` by slot; empty items and the default drop
/// chance (0.085) are left out, as 26.3 saves them. Whether it changed.
pub fn upgrade_entity(entity: &mut Value) -> bool {
    let Some(o) = entity.as_object_mut() else {
        return false;
    };
    const ARMOR: [&str; 4] = ["feet", "legs", "chest", "head"];
    const HANDS: [&str; 2] = ["mainhand", "offhand"];
    let mut changed = false;
    for (items, chances, slots) in [
        ("ArmorItems", "ArmorDropChances", &ARMOR[..]),
        ("HandItems", "HandDropChances", &HANDS[..]),
    ] {
        if let Some(Value::Array(list)) = o.remove(items) {
            changed = true;
            for (slot, item) in slots.iter().zip(list) {
                if item.get("id").is_some() {
                    let equipment = o.entry("equipment").or_insert_with(|| json!({}));
                    equipment[*slot] = item;
                }
            }
        }
        if let Some(Value::Array(list)) = o.remove(chances) {
            changed = true;
            for (slot, chance) in slots.iter().zip(list) {
                if chance.as_f64().is_some_and(|c| (c - 0.085).abs() > 1e-6) {
                    let drop = o.entry("drop_chances").or_insert_with(|| json!({}));
                    drop[*slot] = chance;
                }
            }
        }
    }
    changed
}

/// A 1.21.1 trial spawner config (`trial_spawner/`) in 26.3's shape, in
/// place: its mobs' NBT ([`upgrade_entity`]). Whether it changed.
pub fn upgrade_trial_spawner(config: &mut Value) -> bool {
    let mut changed = false;
    for p in config["spawn_potentials"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        changed |= upgrade_entity(&mut p["data"]["entity"]);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> ProcessorData {
        let mut d = ProcessorData::default();
        d.replace_vanilla.insert(("mvs".into(), "village".into()));
        d
    }

    #[test]
    fn disabled_loot_swap_is_dropped_and_mod_processors_pass_through_to_drop() {
        let mut list = json!({"processors": [
            {"processor_type": "moogs_structures:spawner_randomizing_processor",
             "weighted_entities": [{"entity": "minecraft:blaze", "weight": 2}]},
            {"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": 0},
            {"processor_type": "moogs_structures:vanilla_loot_swap_processor",
             "modid": "mns", "vanilla_key": "fortress", "loot_table_mapping": {}},
            {"processor_type": "moogs_structures:vanilla_loot_swap_processor",
             "modid": "mvs", "vanilla_key": "village", "loot_table_mapping": {}}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        assert_eq!(p.len(), 3, "{p:#?}");
        // Third-party processor, vanilla processor, and an enabled loot swap stay
        // (non-vanilla ones are dropped later); the disabled swap is removed as a no-op.
        assert_eq!(
            p[0]["processor_type"],
            "moogs_structures:spawner_randomizing_processor"
        );
        assert_eq!(p[1]["processor_type"], "minecraft:gravity");
        assert_eq!(p[2]["modid"], "mvs");
        assert_eq!(
            carried,
            BTreeMap::from([("moogs_structures:vanilla_loot_swap_processor".into(), 1)])
        );
    }

    #[test]
    fn trial_spawner_mobs_take_26_3s_equipment() {
        let mut config = json!({"spawn_potentials": [{"weight": 1, "data": {"entity": {
            "id": "minecraft:skeleton",
            "ArmorItems": [{}, {}, {}, {"id": "minecraft:iron_helmet", "count": 1}],
            "ArmorDropChances": [0.085, 0.085, 0.085, 0.0],
            "HandItems": [{"id": "minecraft:bow", "count": 1}, {}],
            "HandDropChances": [0.085, 0.085]}}}]});
        assert!(upgrade_trial_spawner(&mut config));
        assert_eq!(
            config["spawn_potentials"][0]["data"]["entity"],
            json!({"id": "minecraft:skeleton",
                   "equipment": {"head": {"id": "minecraft:iron_helmet", "count": 1},
                                 "mainhand": {"id": "minecraft:bow", "count": 1}},
                   "drop_chances": {"head": 0.0}})
        );
        assert!(!upgrade_trial_spawner(&mut config));
    }

    #[test]
    fn non_vanilla_rule_tests_are_reported_unsupported_and_left_for_their_mod() {
        let rule = |input: Value, position: Value| {
            json!({"input_predicate": input, "location_predicate": {"predicate_type":
                "repurposed_structures:matter_phase_rule_test", "phase_to_test_for": "AIR"},
                "position_predicate": position, "output_state": {"id": "minecraft:lava"}})
        };
        let mut pool = json!({"elements": [{"weight": 1, "element": {
            "element_type": "minecraft:single_pool_element", "processors": {"processors": [
                {"processor_type": "minecraft:rule", "rules": [rule(
                    json!({"predicate_type": "always_true"}),
                    json!({"predicate_type": "repurposed_structures:y_value_pos_rule_test",
                        "min_y_value": 0, "max_y_value": 31}))]},
                {"processor_type": "minecraft:capped", "limit": 2, "delegate": {
                    "processor_type": "minecraft:rule", "rules": [rule(
                        json!({"predicate_type": "minecraft:block_match", "block": "minecraft:rail"}),
                        json!({"predicate_type":
                            "repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test",
                            "axis": "y", "min_dist": 0, "max_dist": 1}))]}}]}}}]});
        let before = pool.clone();
        let mut unsupported = BTreeMap::new();
        report_unsupported_rule_tests("template_pool", &mut pool, &mut unsupported);
        assert_eq!(pool, before, "custom rule tests stay intact for their mod");
        assert_eq!(
            unsupported.get("repurposed_structures:matter_phase_rule_test"),
            Some(&2)
        );
        assert_eq!(
            unsupported.get("repurposed_structures:y_value_pos_rule_test"),
            Some(&1)
        );
        assert_eq!(
            unsupported.get("repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test"),
            Some(&1)
        );
        assert!(!unsupported.contains_key("always_true"));
        assert!(!unsupported.contains_key("minecraft:block_match"));

        // Removed sunrise: rule tests are also reported unsupported.
        let mut legacy = json!({"processors": [{"processor_type": "minecraft:rule", "rules": [{
            "input_predicate": {"predicate_type": "sunrise:matter_phase", "phase_to_test_for": "AIR"},
            "location_predicate": {"predicate_type": "minecraft:always_true"},
            "position_predicate": {"predicate_type": "sunrise:y_value"},
            "output_state": {"id": "minecraft:stone"}
        }]}]});
        let mut legacy_unsupported = BTreeMap::new();
        report_unsupported_rule_tests("processor_list", &mut legacy, &mut legacy_unsupported);
        assert_eq!(legacy_unsupported.get("sunrise:matter_phase"), Some(&1));
        assert_eq!(legacy_unsupported.get("sunrise:y_value"), Some(&1));

        // Non-processor files are not scanned.
        let mut unrelated = BTreeMap::new();
        report_unsupported_rule_tests(
            "structure",
            &mut json!({"predicate_type": "repurposed_structures:y_value_pos_rule_test"}),
            &mut unrelated,
        );
        assert!(unrelated.is_empty());
    }

    #[test]
    fn repurposed_structure_voids_keep_the_world() {
        let mut list = json!({"processors": [
            {"processor_type": "repurposed_structures:structure_void_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "minecraft:block_ignore",
                "blocks": ["minecraft:structure_void"]}])
        );
        assert_eq!(carried["repurposed_structures:structure_void_processor"], 1);
    }
}
