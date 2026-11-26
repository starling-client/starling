//! Lithostitched's worldgen, carried as the game reads it (clean room:
//! `cleanroom/structures/spec-lithostitched.md`; CTOV's processor:
//! `spec-ctov.md` §4).
//!
//! - `lithostitched:jigsaw` structures become vanilla's jigsaw. Lithostitched
//!   lays jigsaws out its own way (OPEN-L5): Sunrise keeps vanilla's layouts.
//! - `add_template_pool_elements` and `add_processor_list_processors`
//!   modifiers become `sunrise:add_to` pool and processor list files
//!   (`docs/modding/src/how-to/content-mods.md`), with their `priority`, when their load conditions pass.
//! - Its `limited` and `guaranteed` pool elements are the game's own
//!   (`sunrise_world`'s `structure::jigsaw`); `delegating` places its
//!   `delegate`, and what it adds is counted as lost.
use crate::worldgen::Upgrade;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const JIGSAW: &str = "lithostitched:jigsaw";
/// Vanilla's JigsawStructure.MAX_DEPTH.
const MAX_SIZE: i64 = 20;
fn count(map: &mut BTreeMap<String, usize>, name: &str) {
    *map.entry(name.to_owned()).or_default() += 1;
}

/// A `lithostitched:jigsaw` structure as vanilla's jigsaw: `Some(true)`, else
/// `None`.
pub(crate) fn upgrade(
    kind: &str,
    v: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> Option<bool> {
    if v["type"] == "lithostitched:weighted_selector" {
        return Some(weighted_selector(v));
    }
    if kind != "structure" || v["type"] != JIGSAW {
        return None;
    }
    v["type"] = "minecraft:jigsaw".into();
    // `size` may be an int provider (sampled per start): its largest value.
    let size = match &v["size"] {
        Value::Null => None,
        Value::Number(n) => n.as_i64(),
        provider => {
            count(lost, "lithostitched:jigsaw size provider");
            ["max_inclusive", "value"]
                .iter()
                .find_map(|k| provider[k].as_i64())
        }
    };
    if let Some(size) = size {
        if size > MAX_SIZE {
            count(lost, "lithostitched:jigsaw size over 20");
        }
        v["size"] = size.clamp(0, MAX_SIZE).into();
    }
    let o = v.as_object_mut()?;
    match o.remove("start_projection") {
        Some(Value::String(heightmap)) if heightmap != "floor" && heightmap != "ceiling" => {
            o.entry("project_start_to_heightmap")
                .or_insert(heightmap.into());
        }
        Some(_) => count(lost, "lithostitched:jigsaw start_projection"),
        None => {}
    }
    if o.remove("fixed_rotation").is_some_and(|f| f == true) {
        count(lost, "lithostitched:jigsaw fixed_rotation");
    }
    Some(true)
}

/// Lithostitched's pool elements as the element they wrap; whether `element`
/// was one.
pub(crate) fn upgrade_pool_element(
    element: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> bool {
    let Some(kind) = element["element_type"].as_str() else {
        return false;
    };
    let unwrap = match kind {
        "lithostitched:guaranteed" => false,
        // A `limit` is what it adds (spec-lithostitched §3).
        "lithostitched:limited" => element["limit"].as_i64().is_none(),
        "lithostitched:delegating" => true,
        _ => return false,
    };
    if unwrap {
        count(lost, kind);
        let delegate = element["delegate"].take();
        *element = delegate;
        // A delegate wrapping another (only `delegating` allows it).
        upgrade_pool_element(element, lost);
    }
    true
}

/// The ids a modifier targets: an id or a list of ids (`None` for tags and
/// Lithostitched's object forms).
fn ids(v: &Value) -> Option<Vec<String>> {
    let id = |v: &Value| {
        v.as_str()
            .filter(|s| !s.starts_with('#'))
            .map(str::to_owned)
    };
    match v {
        Value::Array(a) => a.iter().map(id).collect(),
        other => Some(vec![id(other)?]),
    }
}

/// NeoForge's load conditions (`neoforge:conditions`): all must pass.
pub(crate) fn conditions_pass(v: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
    fn pass(c: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
        let all = |key: &str| {
            c[key]
                .as_array()
                .into_iter()
                .flatten()
                .all(|c| pass(c, loaded))
        };
        match c["type"].as_str() {
            Some("neoforge:mod_loaded") => c["modid"].as_str().is_some_and(loaded),
            Some("neoforge:always") => true,
            Some("neoforge:never") => false,
            Some("neoforge:not") => !pass(&c["value"], loaded),
            Some("neoforge:and") => all("values"),
            Some("neoforge:or") => {
                (c["values"].as_array().into_iter().flatten()).any(|c| pass(c, loaded))
            }
            _ => false,
        }
    }
    (v["neoforge:conditions"].as_array().into_iter().flatten()).all(|c| pass(c, loaded))
}

/// A `weighted_selector` feature in 26.3's form (`config` unwrapped) as
/// 26.3's `weighted_random_selector`, which reads only `{data, weight}`
/// entries: an entry without a weight weighs 1. Whether it was one.
pub(crate) fn weighted_selector(v: &mut Value) -> bool {
    let Some(features) = (v["type"] == "lithostitched:weighted_selector")
        .then(|| v["features"].as_array_mut())
        .flatten()
    else {
        return false;
    };
    for f in features.iter_mut() {
        if f.get("weight").is_none() || f.get("data").is_none() {
            *f = json!({"data": f.take(), "weight": 1});
        }
    }
    v["type"] = "minecraft:weighted_random_selector".into();
    true
}

/// A worldgen modifier as a file the game merges: `("template_pool", file)`
/// for `add_template_pool_elements`, `("processor_list", file)` for
/// `add_processor_list_processors` (spec-lithostitched §2.1, §2.2); the
/// others are counted in `report.modifiers_left`.
pub(crate) fn modifier(
    v: &Value,
    loaded: &dyn Fn(&str) -> bool,
    report: &mut Upgrade,
) -> Option<(&'static str, Value)> {
    let kind = v["type"].as_str().unwrap_or("no type");
    if !conditions_pass(v, loaded) {
        count(&mut report.modifiers_left, "conditions");
        return None;
    }
    if let Some(action) = kind
        .strip_prefix("lithostitched:")
        .filter(|a| matches!(*a, "add_features" | "remove_features"))
    {
        return biome_modifier(v, action, report);
    }
    // 1.8.1 reads `processor_lists`, not the wiki's `processor_list`.
    let (to, targets, list) = match kind {
        "lithostitched:add_template_pool_elements" => {
            ("template_pool", "template_pools", "elements")
        }
        "lithostitched:add_processor_list_processors" => {
            ("processor_list", "processor_lists", "processors")
        }
        _ => {
            count(&mut report.modifiers_left, kind);
            return None;
        }
    };
    let (Some(targets), Some(entries)) = (ids(&v[targets]), v[list].as_array()) else {
        count(&mut report.modifiers_left, "target");
        return None;
    };
    for target in &targets {
        *report.added_to.entry(target.clone()).or_default() += entries.len();
    }
    let mut file = json!({"sunrise:add_to": targets, list: entries});
    if let Some(priority) = v["priority"].as_i64() {
        file["sunrise:priority"] = priority.into();
    }
    Some((to, file))
}

/// An `add_features` or `remove_features` modifier as NeoForge's biome
/// modifier of the same name (`("biome_modifier", file)`), which the game
/// applies when the content root builds. Its `holder_set` biomes are
/// unwrapped. A mod's own `<ns>:config` predicate runs Java the port cannot,
/// so it is taken at the mod's default, on; any other predicate leaves the
/// modifier out.
fn biome_modifier(v: &Value, action: &str, report: &mut Upgrade) -> Option<(&'static str, Value)> {
    let predicate = v["predicate"]["type"].as_str();
    if predicate.is_some_and(|p| !p.ends_with(":config")) {
        count(&mut report.modifiers_left, "predicate");
        return None;
    }
    let biomes = match &v["biomes"] {
        b if b["type"] == "lithostitched:holder_set" => b["set"].clone(),
        b @ (Value::String(_) | Value::Array(_)) => b.clone(),
        _ => Value::Null,
    };
    let add = action == "add_features";
    if biomes.is_null() || v["features"].is_null() || (add && v["step"].is_null()) {
        count(&mut report.modifiers_left, "target");
        return None;
    }
    let mut file = json!({"type": format!("neoforge:{action}"), "biomes": biomes,
        "features": v["features"]});
    // A removal without a step takes the features out of every step.
    if !v["step"].is_null() {
        file[if add { "step" } else { "steps" }] = v["step"].clone();
    }
    Some(("biome_modifier", file))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regions Unexplored's tree and flower groups: a weighted pick of placed
    /// features, an entry without `weight` weighing 1, is 26.3's own
    /// `weighted_random_selector`, which reads only `{data, weight}` entries.
    #[test]
    fn weighted_selectors_become_vanillas() {
        let mut lost = BTreeMap::new();
        let oak = json!({"feature": "ru:tree/oak", "placement": []});
        let big = json!({"feature": "ru:tree/big_oak", "placement": []});
        let mut f = json!({"type": "lithostitched:weighted_selector",
            "features": [{"data": oak, "weight": 2}, big]});
        assert_eq!(upgrade("configured_feature", &mut f, &mut lost), Some(true));
        assert_eq!(
            f,
            json!({"type": "minecraft:weighted_random_selector",
                "features": [{"data": oak, "weight": 2}, {"data": big, "weight": 1}]})
        );
        assert!(lost.is_empty());
    }

    #[test]
    fn jigsaws_become_vanillas() {
        let mut lost = BTreeMap::new();
        let mut s = json!({"type": JIGSAW, "size": {"type": "uniform", "min_inclusive": 3,
            "max_inclusive": 40}, "start_projection": "WORLD_SURFACE_WG", "fixed_rotation": true});
        assert_eq!(upgrade("structure", &mut s, &mut lost), Some(true));
        assert_eq!(
            s,
            json!({"type": "minecraft:jigsaw", "size": 20,
                "project_start_to_heightmap": "WORLD_SURFACE_WG"})
        );
        assert_eq!(lost.len(), 3);
        assert_eq!(
            upgrade(
                "structure",
                &mut json!({"type": "minecraft:jigsaw"}),
                &mut lost
            ),
            None
        );
    }

    #[test]
    fn limited_and_guaranteed_elements_stay_theirs() {
        let mut lost = BTreeMap::new();
        let delegate = json!({"element_type": "minecraft:single_pool_element", "location": "a:b",
            "projection": "rigid", "processors": "minecraft:empty"});
        for kept in [
            json!({"element_type": "lithostitched:limited", "limit": 1, "delegate": delegate}),
            json!({"element_type": "lithostitched:guaranteed", "count": 1, "min_depth": 3,
                "delegate": delegate}),
        ] {
            let mut e = kept.clone();
            assert!(upgrade_pool_element(&mut e, &mut lost));
            assert_eq!(e, kept);
        }
        assert!(lost.is_empty());
        // Without a limit, and `delegating`: the delegate, the rest counted.
        for kind in ["lithostitched:limited", "lithostitched:delegating"] {
            let mut e = json!({"element_type": kind, "delegate": delegate});
            assert!(upgrade_pool_element(&mut e, &mut lost));
            assert_eq!(e, delegate);
            assert_eq!(lost[kind], 1);
        }
        assert!(!upgrade_pool_element(&mut delegate.clone(), &mut lost));
    }

    #[test]
    fn constant_conditions_gate_modifier_conversion() {
        let mut report = Upgrade::default();
        let mut file = json!({
            "type": "lithostitched:add_template_pool_elements",
            "template_pools": "demo:pool",
            "elements": [],
            "neoforge:conditions": [{"type": "neoforge:always"}],
        });
        assert!(modifier(&file, &|_| false, &mut report).is_some());
        file["neoforge:conditions"][0]["type"] = json!("neoforge:never");
        assert!(modifier(&file, &|_| false, &mut report).is_none());
    }

    #[test]
    fn modifiers_add_to_pools_when_their_mods_are_loaded() {
        let mut report = Upgrade::default();
        let loaded = |m: &str| m == "waystones";
        let add = |modid: &str, pools: Value| {
            json!({"type": "lithostitched:add_template_pool_elements", "template_pools": pools,
                "elements": [{"weight": 2, "element": {"element_type": "minecraft:empty_pool_element"}}],
                "neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": modid}]})
        };
        let (kind, file) = modifier(
            &add("waystones", json!("ctov:village/plains/house")),
            &loaded,
            &mut report,
        )
        .unwrap();
        assert_eq!(kind, "template_pool");
        assert_eq!(file["sunrise:add_to"], json!(["ctov:village/plains/house"]));
        assert_eq!(file["elements"][0]["weight"], 2);
        assert_eq!(report.added_to["ctov:village/plains/house"], 1);
        assert!(modifier(&add("sawmill", json!("a:b")), &loaded, &mut report).is_none());
        assert!(modifier(&add("waystones", json!("#a:pools")), &loaded, &mut report).is_none());
        let aliases = json!({"type": "lithostitched:set_pool_aliases"});
        assert!(modifier(&aliases, &loaded, &mut report).is_none());
        // Processors appended to lists, with the modifier's priority; 1.8.1
        // reads `processor_lists` (spec-lithostitched §2.2).
        let nop = json!([{"processor_type": "minecraft:nop"}]);
        let processors = json!({"type": "lithostitched:add_processor_list_processors",
            "priority": 10, "processor_lists": ["minecraft:mossify_10_percent"], "processors": nop});
        let (kind, file) = modifier(&processors, &loaded, &mut report).unwrap();
        assert_eq!(kind, "processor_list");
        assert_eq!(
            file,
            json!({"sunrise:add_to": ["minecraft:mossify_10_percent"], "processors": nop,
                "sunrise:priority": 10})
        );
        let wiki = json!({"type": "lithostitched:add_processor_list_processors",
            "processor_list": "minecraft:mossify_10_percent", "processors": nop});
        assert!(modifier(&wiki, &loaded, &mut report).is_none());
        assert_eq!(
            report.modifiers_left,
            BTreeMap::from([
                ("conditions".into(), 1),
                ("lithostitched:set_pool_aliases".into(), 1),
                ("target".into(), 2),
            ])
        );
    }

    /// Regions Unexplored's vanilla changes: feature modifiers become
    /// NeoForge's biome modifiers, which the game applies; a mod's own config
    /// predicate is taken at its default (on), any other predicate is left.
    #[test]
    fn feature_modifiers_become_biome_modifiers() {
        let mut report = Upgrade::default();
        let loaded = |_: &str| true;
        let biomes = json!({"type": "lithostitched:holder_set", "set": ["minecraft:badlands"]});
        let add = json!({"type": "lithostitched:add_features", "biomes": biomes,
            "features": "ru:saguaros", "step": "vegetal_decoration",
            "predicate": {"type": "ru:config", "key": "vanilla_changes/saguaros"}});
        let (kind, file) = modifier(&add, &loaded, &mut report).unwrap();
        assert_eq!(kind, "biome_modifier");
        assert_eq!(
            file,
            json!({"type": "neoforge:add_features", "biomes": ["minecraft:badlands"],
                "features": "ru:saguaros", "step": "vegetal_decoration"})
        );
        let remove = json!({"type": "lithostitched:remove_features", "priority": 1000,
            "biomes": {"type": "lithostitched:holder_set", "set": "minecraft:mangrove_swamp"},
            "features": "minecraft:patch_waterlily", "step": "vegetal_decoration"});
        let (_, file) = modifier(&remove, &loaded, &mut report).unwrap();
        assert_eq!(
            file,
            json!({"type": "neoforge:remove_features", "biomes": "minecraft:mangrove_swamp",
                "features": "minecraft:patch_waterlily", "steps": "vegetal_decoration"})
        );
        let mut gated = add.clone();
        gated["predicate"] = json!({"type": "lithostitched:not", "predicate": {}});
        assert!(modifier(&gated, &loaded, &mut report).is_none());
        assert_eq!(
            report.modifiers_left,
            BTreeMap::from([("predicate".into(), 1)])
        );
    }
}
