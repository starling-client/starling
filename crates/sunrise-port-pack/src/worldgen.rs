//! Structure mods' worldgen, carried over: block and fluid states in 26.3's
//! shape, the structure, placement and pool element types of the common
//! structure libraries rewritten as vanilla's, and the loader's convention
//! biome tags (`c:`) the structures name.
//!
//! The libraries' placements and pool elements add rules on top of
//! vanilla's: the placement rules of Repurposed Structures' and Moog's
//! spreads (a minimum distance from the world origin, sets to keep away
//! from) are written under `sunrise:` keys the game reads. Each rewrite is
//! counted in the carry report, and every type left as it is is listed, so
//! the coverage report can say what generates and how closely.
//!
//! A structure type with its own start or layout rules needs the mod that
//! registers it (`ModStructureType`), and is listed as unsupported:
//! Repurposed Structures' and Moog's jigsaw structures, Formations'
//! structures and elements, Mo' Structures' structures with a height range,
//! and YUNG's API's `yungsapi:yung_jigsaw` structures and elements.
//! Lithostitched's jigsaws, elements, modifiers and processors are carried by
//! [`crate::lithostitched`]; the entries CTOV adds to vanilla's structure sets
//! by `legacy_structure_mods`, called from [`crate::carry`].
use crate::lithostitched;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Placements that are vanilla's random spread with extra rules.
const SPREADS: [&str; 2] = [
    "moogs_structures:advanced_random_spread",
    "repurposed_structures:advanced_random_spread",
];
/// Mo' Structures' placement (`mostructures:type`) takes its spacing and
/// separation from the mod's config, not its files: the defaults it writes
/// (MoStructuresConfig), by config key, as (spacing, separation).
const MO_SPREADS: [(&str, i64, i64); 17] = [
    ("abandoned_church", 33, 20),
    ("air_balloon", 22, 12),
    ("barn_house", 32, 16),
    ("big_pyramid", 32, 25),
    ("ice_tower", 28, 20),
    ("jungle_pyramid", 25, 22),
    ("killer_bunny_castle", 32, 22),
    ("lighthouse", 24, 14),
    ("moai", 17, 15),
    ("pillager_factory", 34, 16),
    ("pillager_mines", 40, 27),
    ("pirate_ship", 32, 16),
    ("tavern", 26, 16),
    ("the_castle_in_the_sky", 32, 22),
    ("villager_market", 32, 16),
    ("villager_tower", 26, 16),
    ("sphinx", 24, 16),
];
/// Mo' Structures keeps a start out of this many chunks around a start of
/// each set in its `structure_set_to_avoid`.
const MO_AVOID_CHUNKS: i64 = 3;
/// Moog's Structure Lib spreads its structures 1.65 times as far apart as
/// its files say (AdvancedRandomSpread's constructor).
const MOOGS_SPACING: f64 = 1.65;
/// The libraries' terrain kernel field, the game's
/// `sunrise:enhanced_terrain_adaptation` (docs/modding/src/how-to/content-mods.md).
pub(crate) const ENHANCED_TERRAIN: &str = "enhanced_terrain_adaptation";
pub(crate) const SUNRISE_ENHANCED_TERRAIN: &str = "sunrise:enhanced_terrain_adaptation";
/// Pool elements that are vanilla's with extra fields: (mod type, vanilla type).
const ELEMENTS: [(&str, &str); 3] = [
    (
        "moogs_structures:versioned_single_pool_element",
        "minecraft:single_pool_element",
    ),
    (
        "repurposed_structures:legacy_ocean_bottom_single_pool_element",
        "minecraft:legacy_single_pool_element",
    ),
    ("empty_pool_element", "minecraft:empty_pool_element"),
];

/// What carry-over did to a mod's worldgen files.
#[derive(Debug, Default, serde::Serialize)]
pub struct Upgrade {
    /// Files rewritten to vanilla types.
    pub mapped: usize,
    /// Files whose block and fluid states were written in 26.3's shape.
    pub block_states: usize,
    /// Types left as they are (Sunrise skips what it cannot run), by type.
    pub unsupported: BTreeMap<String, usize>,
    /// Rules of mapped types the game does not run, by field.
    pub rules_lost: BTreeMap<String, usize>,
    /// Processors written in Java, taken out of their lists, by type.
    pub processors_dropped: BTreeMap<String, usize>,
    /// Processors written in Java carried as data the game runs
    /// ([`crate::processors`]), by type.
    pub processors_carried: BTreeMap<String, usize>,
    /// Entries added to other structure sets and template pools
    /// (`sunrise:add_to`), by the set or pool they join.
    pub added_to: BTreeMap<String, usize>,
    /// Lithostitched worldgen modifiers not carried: by type, `conditions`
    /// when they need a mod that is not ported along, `target` when they
    /// name their sets or pools by a tag.
    pub modifiers_left: BTreeMap<String, usize>,
}

/// Where a mod's additions to other sets and pools go in its carry:
/// `data/<id>/worldgen/<kind>/sunrise_added/<name>.json`.
pub const ADDED: &str = "sunrise_added";

/// Rewrites one `data/<ns>/worldgen/<kind>/...` file in place; whether it
/// changed.
/// `set_tags` are the mod's structure set tags, expanded
/// ([`structure_set_tags`]).
pub fn upgrade(
    kind: &str,
    v: &mut Value,
    report: &mut Upgrade,
    set_tags: &BTreeMap<String, Vec<String>>,
) -> bool {
    let states = block_states(v);
    report.block_states += usize::from(states);
    let features = features(kind, v, report);
    let renamed = crate::carry::renamed_ids(v);
    let dropped = crate::processors::each_list(kind, v, &mut |list| {
        drop_java_processors(list, &mut report.processors_dropped)
    });
    crate::processors::report_unsupported_rule_tests(kind, v, &mut report.unsupported);
    upgrade_types(kind, v, report, set_tags) || states || features || dropped || renamed
}

/// A 1.21.1 configured or placed feature in 26.3's form, as world packs get
/// it ([`crate::upgrade`]): `config` unwrapped (26.3 skips a feature that
/// keeps it), patches and placements rewritten. Whether it changed.
fn features(kind: &str, v: &mut Value, report: &mut Upgrade) -> bool {
    let mut r = crate::upgrade::Report::default();
    let new = match kind {
        // Every 1.21.1 configured feature has `config`; one without is 26.3's.
        "configured_feature" if v.get("config").is_some() => {
            crate::upgrade::features::feature(v, &mut r)
        }
        "placed_feature" => crate::upgrade::features::placed(v, &mut r),
        _ => return false,
    };
    for (t, n) in r.unknown {
        *report.unsupported.entry(t).or_default() += n;
    }
    let changed = new != *v;
    *v = new;
    changed
}

/// Takes non-`minecraft:` processors out of a processor list: `{"processors":
/// [...]}`, a bare list, or an id (left as it is). Any processor with a
/// vanilla data form was rewritten before ([`crate::processors::carry`]);
/// remaining third-party types require a library mod that registers them via
/// `STRUCTURE_PROCESSOR`. The game, like vanilla, refuses a list holding a
/// type it does not know, and with it every pool that names the list, so
/// without this the structure does not generate at all. Whether anything was
/// taken out.
fn drop_java_processors(list: &mut Value, dropped: &mut BTreeMap<String, usize>) -> bool {
    let Some(processors) = crate::processors::processors(list) else {
        return false;
    };
    let before = processors.len();
    processors.retain(|p| {
        let t = p["processor_type"].as_str().unwrap_or_default();
        let keep = t.starts_with("minecraft:");
        if !keep {
            *dropped.entry(t.to_owned()).or_default() += 1;
        }
        keep
    });
    processors.len() != before
}

/// 1.21.1 writes a block or fluid state as `{"Name": ..., "Properties":
/// {...}}`; 26.3's codec reads `{"id": ..., "properties": {...}}` and
/// refuses the old shape, which fails the processor list, feature or pool
/// holding it. Whether anything changed.
fn block_states(v: &mut Value) -> bool {
    match v {
        Value::Object(o) => {
            let state = o.get("Name").is_some_and(Value::is_string)
                && o.keys().all(|k| k == "Name" || k == "Properties");
            if state {
                if let Some(name) = o.remove("Name") {
                    o.insert("id".into(), name);
                }
                if let Some(properties) = o.remove("Properties") {
                    o.insert("properties".into(), properties);
                }
                return true;
            }
            o.values_mut()
                .fold(false, |changed, x| block_states(x) | changed)
        }
        Value::Array(a) => a
            .iter_mut()
            .fold(false, |changed, x| block_states(x) | changed),
        _ => false,
    }
}

/// The library types of one file rewritten as vanilla's; whether it changed.
fn upgrade_types(
    kind: &str,
    v: &mut Value,
    report: &mut Upgrade,
    set_tags: &BTreeMap<String, Vec<String>>,
) -> bool {
    let lost = &mut report.rules_lost;
    // Lithostitched's types are converted apart from this code.
    if let Some(changed) = lithostitched::upgrade(kind, v, lost) {
        report.mapped += usize::from(changed);
        return changed;
    }
    let mut unsupported = |t: &str| *report.unsupported.entry(t.to_owned()).or_default() += 1;
    let changed = match kind {
        "structure" => match v["type"].as_str() {
            // Mo' Structures' ModStructure without a height range (-1) is
            // vanilla's jigsaw; with one it starts only where the terrain
            // under the 5×5 chunks around varies by no more than that, a
            // rule of its own.
            Some("mostructures:generic")
                if v["heightRange"].as_i64().is_none_or(|range| range < 0) =>
            {
                if let Some(o) = v.as_object_mut() {
                    o.remove("heightRange");
                }
                v["type"] = "minecraft:jigsaw".into();
                true
            }
            Some(t) if !t.starts_with("minecraft:") => {
                unsupported(t);
                false
            }
            _ => false,
        },
        "structure_set" => match v["placement"]["type"].as_str() {
            Some(t) if SPREADS.contains(&t) => {
                let moogs = t.starts_with("moogs_structures:");
                let p = &mut v["placement"];
                if moogs {
                    moogs_spacing(p, lost);
                }
                placement_rules(p, set_tags, lost);
                p["type"] = "minecraft:random_spread".into();
                true
            }
            Some("mostructures:type") => mo_placement(&mut v["placement"], lost),
            Some(t) if !t.starts_with("minecraft:") => {
                unsupported(t);
                false
            }
            _ => false,
        },
        "template_pool" => {
            let mut changed = false;
            let mut elements: Vec<&mut Value> = Vec::new();
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
                // A list element holds elements of its own.
                if element.get("elements").is_some_and(Value::is_array) {
                    elements.extend(element["elements"].as_array_mut().into_iter().flatten());
                } else {
                    elements.push(element);
                }
            }
            for element in elements {
                if lithostitched::upgrade_pool_element(element, lost) {
                    changed = true;
                    continue;
                }
                let Some(t) = element["element_type"].as_str() else {
                    continue;
                };
                if let Some((_, to)) = ELEMENTS.iter().find(|(from, _)| *from == t) {
                    if t.starts_with("moogs_structures:") {
                        let own = element
                            .as_object_mut()
                            .and_then(|o| o.remove(ENHANCED_TERRAIN));
                        if let Some(kernel) = own.and_then(|a| moogs_terrain(&a, lost)) {
                            element[SUNRISE_ENHANCED_TERRAIN] = kernel;
                        }
                    }
                    element["element_type"] = (*to).into();
                    changed = true;
                } else if !t.starts_with("minecraft:") {
                    unsupported(t);
                }
            }
            changed
        }
        _ => false,
    };
    report.mapped += usize::from(changed);
    changed
}

/// MoogsStructureLib's terrain adaptation: already the game's shape (with
/// its band), when it has the fields the game needs (else it is lost, as
/// the library's codec would refuse it).
pub(crate) fn moogs_terrain(a: &Value, lost: &mut BTreeMap<String, usize>) -> Option<Value> {
    let ok = ["kernel_size", "kernel_distance"]
        .iter()
        .all(|k| a[k].is_u64())
        && ["top", "bottom"].iter().all(|k| a[k].is_string());
    if !ok {
        *lost.entry(ENHANCED_TERRAIN.into()).or_default() += 1;
    }
    ok.then(|| a.clone())
}

/// AdvancedRandomSpread's spacing and separation, as Moog's computes them
/// with its default config: 1.65 times the file's, rounded, the separation
/// below the spacing. `when_replacing` (other numbers while a preset
/// replaces a vanilla structure) is lost.
fn moogs_spacing(p: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let scaled = |k: &str| (p[k].as_f64().unwrap_or(0.) * MOOGS_SPACING).round() as i64;
    let spacing = scaled("spacing").max(1);
    let separation = scaled("separation").min(spacing - 1);
    p["spacing"] = spacing.into();
    p["separation"] = separation.into();
    if let Some(o) = p.as_object_mut()
        && o.remove("when_replacing").is_some()
    {
        *lost.entry("when_replacing".into()).or_default() += 1;
    }
}

/// An advanced random spread's minimum distance from the world origin and
/// its super exclusion zone (sets named by a tag) under `sunrise:` keys.
pub(crate) fn placement_rules(
    p: &mut Value,
    set_tags: &BTreeMap<String, Vec<String>>,
    lost: &mut BTreeMap<String, usize>,
) {
    let Some(o) = p.as_object_mut() else {
        return;
    };
    if let Some(d) = o.remove("min_distance_from_world_origin") {
        o.insert("sunrise:min_distance_from_world_origin".into(), d);
    }
    let zone = ["super_exclusion_zone", "enhanced_exclusion_zone"]
        .into_iter()
        .find_map(|k| Some((k, o.remove(k)?)));
    if let Some((key, zone)) = zone {
        let sets: Vec<String> = match &zone["other_set"] {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => set_tags.get(tag).cloned().unwrap_or_default(),
                None => vec![s.clone()],
            },
            Value::Array(a) => a
                .iter()
                .filter_map(|s| Some(s.as_str()?.to_owned()))
                .collect(),
            _ => Vec::new(),
        };
        if sets.is_empty() {
            *lost.entry(key.into()).or_default() += 1;
        } else {
            o.insert(
                "sunrise:exclusion_zone".into(),
                serde_json::json!({"other_sets": sets, "chunk_count": zone["chunk_count"]}),
            );
        }
    }
}

/// Mo' Structures' placement as vanilla's random spread: the spacing and
/// separation of its config key, and its sets to avoid as
/// `sunrise:exclusion_zone` tested by their spread alone
/// (ModStructurePlacement.exclusionZoneCheck). A key the config does not
/// have fails the mod's codec, and the set stays as it is.
fn mo_placement(p: &mut Value, lost: &mut BTreeMap<String, usize>) -> bool {
    let key = p["config_key"].as_str().unwrap_or_default();
    let Some(&(_, spacing, separation)) = MO_SPREADS.iter().find(|(k, ..)| *k == key) else {
        *lost.entry("config_key".into()).or_default() += 1;
        return false;
    };
    let Some(o) = p.as_object_mut() else {
        return false;
    };
    o.remove("config_key");
    o.insert("spacing".into(), spacing.into());
    o.insert("separation".into(), separation.into());
    let avoid = o.remove("structure_set_to_avoid");
    let sets: Vec<Value> = avoid
        .iter()
        .flat_map(|a| a.as_array().into_iter().flatten())
        .filter(|s| s.is_string())
        .cloned()
        .collect();
    if !sets.is_empty() {
        o.insert(
            "sunrise:exclusion_zone".into(),
            serde_json::json!({"other_sets": sets, "chunk_count": MO_AVOID_CHUNKS,
                "their_zones": false}),
        );
    }
    o.insert("type".into(), "minecraft:random_spread".into());
    true
}

/// The mod's structure set tags (`data/<ns>/tags/worldgen/structure_set/`),
/// by tag id, with the tags they include expanded. Optional entries
/// (`{"id": ..., "required": false}`) count as their id.
pub fn structure_set_tags<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> BTreeMap<String, Vec<String>> {
    let mut raw: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut budget = crate::entry::Budget::archive();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let Some(id) = entry
            .name()
            .ok()
            .as_deref()
            .and_then(|n| n.strip_prefix("data/"))
            .and_then(|r| r.split_once("/tags/worldgen/structure_set/"))
            .and_then(|(ns, path)| Some(format!("{ns}:{}", path.strip_suffix(".json")?)))
        else {
            continue;
        };
        let declared = entry.size();
        let Ok(v) = budget
            .read_text(&mut entry, declared, crate::entry::MAX_TEXT)
            .map_err(drop)
            .and_then(|text| {
                serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).map_err(drop)
            })
        else {
            continue;
        };
        let values = v["values"].as_array().into_iter().flatten();
        raw.insert(
            id,
            values
                .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()).map(str::to_owned))
                .collect(),
        );
    }
    let expand = |tag: &str| {
        let (mut out, mut seen, mut todo) = (Vec::new(), BTreeSet::new(), vec![tag.to_owned()]);
        while let Some(t) = todo.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            for e in raw.get(&t).into_iter().flatten() {
                match e.strip_prefix('#') {
                    Some(inner) => todo.push(inner.to_owned()),
                    None if !out.contains(e) => out.push(e.clone()),
                    None => {}
                }
            }
        }
        out
    };
    raw.keys().map(|t| (t.clone(), expand(t))).collect()
}

/// The `#c:` biome tags `v` names.
pub fn convention_tags(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(tag) = s.strip_prefix("#c:") {
                out.insert(tag.to_owned());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| convention_tags(x, out)),
        Value::Object(o) => o.values().for_each(|x| convention_tags(x, out)),
        _ => {}
    }
}

/// The loader's own convention biome tags (`data/c/tags/worldgen/biome/`),
/// by path, read from the loader jar `fetch` keeps in `<work>/loader/`.
pub fn loader_biome_tags(work: &Path) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Ok(dir) = std::fs::read_dir(work.join("loader")) else {
        return out;
    };
    for jar in dir.flatten().map(|e| e.path()) {
        let Some(mut zip) = std::fs::File::open(&jar)
            .ok()
            .and_then(|f| crate::zip_intake::open(f).ok())
        else {
            continue;
        };
        let mut budget = crate::entry::Budget::archive();
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let Some(path) = entry
                .name()
                .ok()
                .as_deref()
                .and_then(|n| n.strip_prefix("data/c/tags/worldgen/biome/"))
                .and_then(|p| p.strip_suffix(".json"))
                .map(str::to_owned)
            else {
                continue;
            };
            let declared = entry.size();
            if let Ok(text) = budget.read_text(&mut entry, declared, crate::entry::MAX_TEXT)
                && let Ok(v) = serde_json::from_str(&text)
            {
                out.insert(path, v);
            }
        }
    }
    out
}

/// `tags` and the convention tags they include, in `conventions`.
pub fn with_included(
    tags: &BTreeSet<String>,
    conventions: &BTreeMap<String, Value>,
) -> BTreeSet<String> {
    let mut all = BTreeSet::new();
    let mut todo: Vec<String> = tags.iter().cloned().collect();
    while let Some(tag) = todo.pop() {
        if !conventions.contains_key(&tag) || !all.insert(tag.clone()) {
            continue;
        }
        let mut more = BTreeSet::new();
        convention_tags(&conventions[&tag], &mut more);
        todo.extend(more);
    }
    all
}

/// Where `fetch` keeps the pack's loader jar, and where it comes from:
/// NeoForge's own Maven repository. Only NeoForge packs need it today.
pub fn loader_jar(work: &Path, loader: &str, version: &str) -> Option<(PathBuf, String)> {
    // The version comes from the modpack: only a plain version (`21.1.77`, `21.1.0-beta`) makes a
    // file name or a URL, never `/`, `..` or a `#` that would end the URL early.
    let plain = (1..=64).contains(&version.len())
        && !version.contains("..")
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b));
    (loader == "neoforge" && plain).then(|| {
        (
            work.join("loader").join(format!("neoforge-{version}-universal.jar")),
            format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-universal.jar"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_plain_neoforge_version_makes_a_loader_path() {
        let work = Path::new("/work");
        let (dest, url) = loader_jar(work, "neoforge", "21.1.77").unwrap();
        assert_eq!(dest, work.join("loader/neoforge-21.1.77-universal.jar"));
        assert!(url.ends_with("/21.1.77/neoforge-21.1.77-universal.jar"));
        assert!(loader_jar(work, "neoforge", "21.1.0-beta").is_some());
        for bad in [
            "",
            "21.1.1/neoforge-21.1.1-universal.jar#/../../../../x",
            "../x",
            "21.1.1#",
            "a/b",
            "a\\b",
        ] {
            assert_eq!(loader_jar(work, "neoforge", bad), None, "{bad:?}");
        }
        assert_eq!(loader_jar(work, "fabric", "0.16.0"), None);
    }

    const NO_TAGS: BTreeMap<String, Vec<String>> = BTreeMap::new();

    /// Structure types with start or layout rules of their own, and Formations'
    /// element, need their mod: each is left as written and listed once as
    /// unsupported.
    #[test]
    fn structure_types_with_rules_of_their_own_are_unsupported() {
        let mut report = Upgrade::default();
        let types = [
            "formations:pieced",
            "formations:simple",
            "moogs_structures:moogs_structures_generic_jigsaw_structure",
            "moogs_structures:moogs_structures_generic_nether_jigsaw_structure",
            "mostructures:generic",
            "repurposed_structures:city_nether_structure",
            "repurposed_structures:generic_jigsaw_structure",
            "repurposed_structures:generic_mineshaft",
            "repurposed_structures:generic_nether_jigsaw_structure",
            "repurposed_structures:shipwreck_nether_structure",
            "repurposed_structures:stronghold_end",
        ];
        for t in types {
            let mut structure = json!({"type": t, "start_pool": "m:start", "size": 7,
                "cannot_spawn_in_liquid": true, "heightRange": 25});
            let written = structure.clone();
            assert!(
                !upgrade("structure", &mut structure, &mut report, &NO_TAGS),
                "{t}"
            );
            assert_eq!(structure, written, "{t}");
        }
        let mut pool = json!({"fallback": "minecraft:empty", "elements": [
            {"weight": 1, "element": {"element_type": "formations:single_pool_element",
                "location": "f:a", "projection": "rigid", "ground_level": 6, "processors": []}}
        ]});
        let written = pool.clone();
        assert!(!upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(pool, written);
        let mut expected: BTreeMap<String, usize> =
            types.iter().map(|t| (t.to_string(), 1)).collect();
        expected.insert("formations:single_pool_element".into(), 1);
        assert_eq!(report.unsupported, expected);
    }

    #[test]
    fn maps_library_types_to_vanillas() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "lithostitched:jigsaw", "start_pool": "a:b"});
        assert!(upgrade("structure", &mut s, &mut report, &NO_TAGS));
        assert_eq!(s["type"], "minecraft:jigsaw");
        let mut other = json!({"type": "somelib:structure"});
        assert!(!upgrade("structure", &mut other, &mut report, &NO_TAGS));
        let mut set = json!({"placement": {"type": "moogs_structures:advanced_random_spread",
            "spacing": 10, "separation": 5, "min_distance_from_world_origin": 250}});
        assert!(upgrade("structure_set", &mut set, &mut report, &NO_TAGS));
        assert_eq!(set["placement"]["type"], "minecraft:random_spread");
        // Moog's spreads 1.65 times as far as its file says.
        assert_eq!(set["placement"]["spacing"], 17);
        assert_eq!(set["placement"]["separation"], 8);
        assert_eq!(
            set["placement"]["sunrise:min_distance_from_world_origin"],
            250
        );
        assert!(
            set["placement"]
                .get("min_distance_from_world_origin")
                .is_none()
        );
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "moogs_structures:versioned_single_pool_element", "location": "m:x",
                "enhanced_terrain_adaptation": {"kernel_size": 16, "kernel_distance": 8, "top": "carve",
                    "bottom": "carve", "band": {"bottom": 17, "top": 23}}}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "empty_pool_element"}]}},
            {"weight": 1, "element": {"element_type": "somelib:element"}}
        ]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(
            pool["elements"][0]["element"]["element_type"],
            "minecraft:single_pool_element"
        );
        // Its own terrain kernel, band and all, under the game's key.
        assert_eq!(
            pool["elements"][0]["element"]["sunrise:enhanced_terrain_adaptation"]["band"],
            json!({"bottom": 17, "top": 23})
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["element_type"],
            "minecraft:empty_pool_element"
        );
        assert_eq!(report.mapped, 3);
        assert_eq!(report.block_states, 0);
        assert_eq!(
            report.unsupported,
            BTreeMap::from([
                ("somelib:element".into(), 1),
                ("somelib:structure".into(), 1),
            ])
        );
    }

    /// YUNG's API's structure type and element need YUNG's API itself: each is left as written
    /// and listed once as unsupported.
    #[test]
    fn yungs_jigsaw_and_element_are_unsupported() {
        let mut report = Upgrade::default();
        let mut structure = json!({"type": "yungsapi:yung_jigsaw", "start_pool": "m:start",
            "size": 30, "biomes": "#minecraft:is_overworld"});
        let written = structure.clone();
        assert!(!upgrade("structure", &mut structure, &mut report, &NO_TAGS));
        assert_eq!(structure, written);
        let mut pool = json!({"fallback": "minecraft:empty", "elements": [
            {"weight": 1, "element": {"element_type": "yungsapi:yung_single_element",
                "location": "m:room", "projection": "rigid", "processors": "minecraft:empty"}}
        ]});
        let written = pool.clone();
        assert!(!upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(pool, written);
        assert_eq!(
            report.unsupported,
            BTreeMap::from([
                ("yungsapi:yung_jigsaw".into(), 1),
                ("yungsapi:yung_single_element".into(), 1),
            ])
        );
    }

    #[test]
    fn mo_structures_become_vanillas_jigsaw_and_spread() {
        let mut report = Upgrade::default();
        // Without a height range (-1) it starts anywhere, as vanilla's jigsaw.
        let mut anywhere = json!({"type": "mostructures:generic", "start_pool": "mostructures:a/base",
            "size": 7, "heightRange": -1, "use_expansion_hack": true,
            "start_height": {"absolute": 0}, "max_distance_from_center": 80});
        assert!(upgrade(
            "structure",
            &mut anywhere,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(
            anywhere,
            json!({"type": "minecraft:jigsaw", "start_pool": "mostructures:a/base",
                "size": 7, "use_expansion_hack": true,
                "start_height": {"absolute": 0}, "max_distance_from_center": 80})
        );

        // The config's spacing and separation, not the file's.
        let mut set = json!({"placement": {"type": "mostructures:type", "config_key": "moai",
            "salt": 12994829, "spacing": 17, "separation": 15,
            "structure_set_to_avoid": ["minecraft:villages", "mostructures:lighthouse"]}});
        assert!(upgrade(
            "structure_set",
            &mut set,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(
            set["placement"],
            json!({"type": "minecraft:random_spread", "salt": 12994829, "spacing": 17,
                "separation": 15, "sunrise:exclusion_zone": {"chunk_count": 3,
                "other_sets": ["minecraft:villages", "mostructures:lighthouse"],
                "their_zones": false}})
        );
        let mut set = json!({"placement": {"type": "mostructures:type",
            "config_key": "jungle_pyramid", "salt": 1, "spacing": 16, "separation": 14}});
        assert!(upgrade(
            "structure_set",
            &mut set,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(
            (
                &set["placement"]["spacing"],
                &set["placement"]["separation"]
            ),
            (&json!(25), &json!(22))
        );
        assert!(set["placement"].get("sunrise:exclusion_zone").is_none());
        let mut unknown = json!({"placement": {"type": "mostructures:type", "config_key": "x"}});
        assert!(!upgrade(
            "structure_set",
            &mut unknown,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(report.rules_lost["config_key"], 1);
    }

    #[test]
    fn super_exclusion_zones_name_their_sets() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, text) in [
            (
                "data/rs/tags/worldgen/structure_set/avoid.json",
                r##"{"values": ["minecraft:villages", "#rs:more", {"id": "x:y", "required": false}]}"##,
            ),
            (
                "data/rs/tags/worldgen/structure_set/more.json",
                r##"{"values": ["minecraft:villages", "rs:cities", "#rs:avoid"]}"##,
            ),
        ] {
            zip.start_file(name, opts).unwrap();
            std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
        }
        let mut zip = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let tags = structure_set_tags(&mut zip);
        assert_eq!(tags["rs:avoid"], ["minecraft:villages", "x:y", "rs:cities"]);
        let mut report = Upgrade::default();
        let mut set = json!({"placement": {"type": "repurposed_structures:advanced_random_spread",
            "spacing": 30, "separation": 10,
            "super_exclusion_zone": {"chunk_count": 4, "other_set": "#rs:avoid"}}});
        assert!(upgrade("structure_set", &mut set, &mut report, &tags));
        assert_eq!(set["placement"]["spacing"], 30);
        assert_eq!(
            set["placement"]["sunrise:exclusion_zone"],
            json!({"other_sets": ["minecraft:villages", "x:y", "rs:cities"], "chunk_count": 4})
        );
        let mut unknown = json!({"placement": {"type": "repurposed_structures:advanced_random_spread",
            "spacing": 30, "separation": 10,
            "super_exclusion_zone": {"chunk_count": 4, "other_set": "#rs:none"}}});
        assert!(upgrade("structure_set", &mut unknown, &mut report, &tags));
        assert_eq!(report.rules_lost["super_exclusion_zone"], 1);
    }

    #[test]
    fn drops_processors_written_in_java() {
        let mut report = Upgrade::default();
        let mut list = json!({"processors": [
            {"processor_type": "repurposed_structures:bottom_pillar_processor"},
            {"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": -1},
            {"processor_type": "moogs_structures:spawner_randomizing_processor"}]});
        assert!(upgrade("processor_list", &mut list, &mut report, &NO_TAGS));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "minecraft:gravity",
            "heightmap": "WORLD_SURFACE_WG", "offset": -1}])
        );
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "minecraft:single_pool_element",
                "processors": {"processors": [{"processor_type": "lithostitched:block_swap"}]}}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "minecraft:single_pool_element", "processors": "x:named"}]}}]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(
            pool["elements"][0]["element"]["processors"],
            json!({"processors": []})
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["processors"],
            "x:named"
        );
        let mut vanilla = json!({"processors": [{"processor_type": "minecraft:nop"}]});
        assert!(!upgrade(
            "processor_list",
            &mut vanilla,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(
            report.processors_dropped,
            BTreeMap::from([
                ("lithostitched:block_swap".into(), 1),
                ("moogs_structures:spawner_randomizing_processor".into(), 1),
                ("repurposed_structures:bottom_pillar_processor".into(), 1),
            ])
        );
    }

    #[test]
    fn worldgen_files_of_an_unexpected_shape_are_left_alone() {
        let mut report = Upgrade::default();
        let none = BTreeMap::new();
        for mut v in [
            json!([1, 2]),
            json!({"elements": [1, {"weight": 1}]}),
            json!({"elements": 3}),
            json!({}),
        ] {
            let before = v.clone();
            upgrade("template_pool", &mut v, &mut report, &none);
            assert_eq!(v, before);
        }
    }

    #[test]
    fn writes_block_states_in_26_3_shape() {
        let mut report = Upgrade::default();
        let mut list = json!({"processors": [{"processor_type": "minecraft:rule", "rules": [{
            "input_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": {"Name": "minecraft:oak_stairs", "Properties": {"facing": "east"}}}]}]});
        assert!(upgrade("processor_list", &mut list, &mut report, &NO_TAGS));
        assert_eq!(
            list["processors"][0]["rules"][0]["output_state"],
            json!({"id": "minecraft:oak_stairs", "properties": {"facing": "east"}})
        );
        let mut spring = json!({"type": "minecraft:spring_feature", "config": {
            "state": {"Name": "minecraft:water"}, "valid_blocks": ["minecraft:stone"]}});
        assert!(upgrade(
            "configured_feature",
            &mut spring,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(spring["state"], json!({"id": "minecraft:water"}));
        // An object with other keys is not a state; a 26.3 state is left as it is.
        let mut other = json!({"Name": "x", "weight": 1, "state": {"id": "minecraft:stone"}});
        assert!(!upgrade(
            "configured_feature",
            &mut other,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(report.block_states, 2);
        assert_eq!(report.mapped, 0);
    }

    /// A mod's 1.21.1 configured feature keeps its fields under `config`;
    /// 26.3 reads them on the feature itself and skips the old shape, so
    /// the mod's ores never generate.
    #[test]
    fn mod_features_unwrap_their_config() {
        let mut report = Upgrade::default();
        let mut ore = json!({"type": "minecraft:ore", "config": {
            "discard_chance_on_air_exposure": 0.0, "size": 5, "targets": [{
                "state": {"Name": "x:silver_ore"},
                "target": {"predicate_type": "minecraft:tag_match",
                    "tag": "minecraft:stone_ore_replaceables"}}]}});
        assert!(upgrade(
            "configured_feature",
            &mut ore,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(
            ore,
            json!({"type": "minecraft:ore", "discard_chance_on_air_exposure": 0.0, "size": 5,
                "targets": [{"state": {"id": "x:silver_ore"}, "target": {
                    "predicate_type": "minecraft:tag_match",
                    "tag": "minecraft:stone_ore_replaceables"}}]})
        );
        // A mod's own feature type is unwrapped too.
        let mut selector = json!({"type": "lithostitched:weighted_selector",
            "config": {"features": ["x:tree"]}});
        assert!(upgrade(
            "configured_feature",
            &mut selector,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(selector.get("config"), None);
        assert!(selector["features"].is_array());
        // A placed feature's 1.21.1 placements are 26.3's.
        let mut placed = json!({"feature": "x:ore", "placement": [
            {"type": "minecraft:random_offset", "xz_spread": 2, "y_spread": 1}]});
        assert!(upgrade(
            "placed_feature",
            &mut placed,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(
            placed["placement"][0],
            json!({"type": "minecraft:offset", "x": 2, "y": 1, "z": 2})
        );
    }

    #[test]
    fn convention_tags_include_theirs() {
        let conventions = BTreeMap::from([
            (
                "is_hot".to_owned(),
                json!({"values": ["#c:is_desert", "#minecraft:is_badlands"]}),
            ),
            (
                "is_desert".to_owned(),
                json!({"values": ["minecraft:desert"]}),
            ),
            ("is_cold".to_owned(), json!({"values": []})),
        ]);
        let mut used = BTreeSet::new();
        convention_tags(
            &json!({"biomes": ["#c:is_hot", "#c:unknown", "#minecraft:x"]}),
            &mut used,
        );
        assert_eq!(used, BTreeSet::from(["is_hot".into(), "unknown".into()]));
        assert_eq!(
            with_included(&used, &conventions),
            BTreeSet::from(["is_hot".into(), "is_desert".into()])
        );
    }
}
