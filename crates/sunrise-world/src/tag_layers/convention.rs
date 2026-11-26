//! Built-in `c:` and `neoforge:` convention tags embedded in `sunrise-world`.
//!
//! Loaded once from `crates/sunrise-world/data/convention_tags.json` — the same
//! file that generates `mod_api::tags::c::*` — and injected as the lowest layer
//! beneath vanilla files, data packs and mods so `values`, `remove` and
//! `replace: true` layer on top identically to vanilla tags.

use super::TagLayers;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

const DATA: &str = include_str!("../../data/convention_tags.json");

#[derive(Deserialize)]
struct SpecFile {
    registries: Vec<RegistrySpec>,
}

#[derive(Deserialize)]
struct RegistrySpec {
    registry: String,
    tags: Vec<TagSpec>,
}

#[derive(Deserialize)]
struct TagSpec {
    #[serde(rename = "const")]
    const_name: String,
    id: String,
    values: Vec<serde_json::Value>,
}

/// One built-in convention tag from Sunrise's canonical data file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConventionTag {
    /// Rust constant name in `mod_api::tags::c::<module>`.
    pub const_name: String,
    /// Canonical tag identifier (for example `"c:ingots"` or `"neoforge:is_magic"`).
    pub id: String,
    /// Raw tag entries (`"minecraft:iron_ingot"`, `"#c:ingots/iron"`, or
    /// `{"id": "minecraft:milk", "required": false}`), in NeoForge's order.
    pub values: Vec<serde_json::Value>,
}

struct Parsed {
    by_registry: BTreeMap<String, Vec<ConventionTag>>,
    names_by_registry: HashMap<String, HashSet<String>>,
    layers_by_registry: HashMap<String, HashMap<String, TagLayers>>,
    json_by_registry: HashMap<String, Vec<(String, String)>>,
}

static PARSED: OnceLock<Parsed> = OnceLock::new();

fn parsed() -> &'static Parsed {
    PARSED.get_or_init(|| {
        let spec: SpecFile =
            serde_json::from_str(DATA).expect("built-in convention_tags.json must parse");
        let mut by_registry = BTreeMap::new();
        let mut names_by_registry = HashMap::new();
        let mut layers_by_registry = HashMap::new();
        let mut json_by_registry = HashMap::new();
        for reg in spec.registries {
            let mut tags = Vec::with_capacity(reg.tags.len());
            let mut names = HashSet::with_capacity(reg.tags.len());
            let mut layers = HashMap::with_capacity(reg.tags.len());
            let mut jsons = Vec::with_capacity(reg.tags.len());
            for tag in reg.tags {
                let json_value = serde_json::json!({ "values": &tag.values });
                let layer = TagLayers::from_file(json_value.clone())
                    .expect("built-in convention tag entries must be valid");
                let json_text = serde_json::to_string(&json_value)
                    .expect("built-in convention tag JSON must serialize");
                names.insert(tag.id.clone());
                layers.insert(tag.id.clone(), layer);
                jsons.push((tag.id.clone(), json_text));
                tags.push(ConventionTag {
                    const_name: tag.const_name,
                    id: tag.id,
                    values: tag.values,
                });
            }
            by_registry.insert(reg.registry.clone(), tags);
            names_by_registry.insert(reg.registry.clone(), names);
            layers_by_registry.insert(reg.registry.clone(), layers);
            json_by_registry.insert(reg.registry, jsons);
        }
        Parsed {
            by_registry,
            names_by_registry,
            layers_by_registry,
            json_by_registry,
        }
    })
}

/// Every built-in convention tag definition grouped by registry path (`"block"`,
/// `"item"`, `"worldgen/biome"`, ...).
pub fn all() -> &'static BTreeMap<String, Vec<ConventionTag>> {
    &parsed().by_registry
}

/// Whether `id` (for example `"c:ingots"` or `"neoforge:is_magic"`) is a built-in
/// convention tag for registry `kind`.
pub fn has_tag(kind: &str, id: &str) -> bool {
    parsed()
        .names_by_registry
        .get(kind)
        .is_some_and(|names| names.contains(id))
}

/// Base `TagLayers` map for registry `kind`, cloned from the parsed built-in
/// convention definitions so disk files and pack overlays layer on top.
pub fn layers(kind: &str) -> HashMap<String, TagLayers> {
    parsed()
        .layers_by_registry
        .get(kind)
        .cloned()
        .unwrap_or_default()
}

/// Built-in convention tags for registry `kind` as `(tag_id, json_text)` pairs,
/// for loaders that apply raw JSON tag definitions (`damage_source`, `worldgen`).
pub fn json_entries(kind: &str) -> &'static [(String, String)] {
    parsed()
        .json_by_registry
        .get(kind)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use std::path::Path;

    const NEOFORGE_COMMIT: &str = "bfed31772c6bc92db201d36421b1a90e428227af";
    const NEOFORGE_TARBALL_URL: &str = "https://github.com/neoforged/NeoForge/archive/bfed31772c6bc92db201d36421b1a90e428227af.tar.gz";
    const NEOFORGE_TARBALL_SHA256: &str =
        "d1717a220ed3c4c897a6f37d85a749945c8e0733f1b3b7ac36259e8bb248cb0c";

    type TagValuesMap = BTreeMap<(String, String), Vec<serde_json::Value>>;

    fn parse_tag_rel_path(rel_under_data: &str) -> Option<(String, String)> {
        let rest = rel_under_data.strip_suffix(".json")?;
        let (ns, after_tags) = rest
            .strip_prefix("c/tags/")
            .map(|p| ("c", p))
            .or_else(|| rest.strip_prefix("neoforge/tags/").map(|p| ("neoforge", p)))?;
        let mut parts = after_tags.split('/');
        let first = parts.next()?;
        let (registry, tag_path) = if first == "worldgen" {
            let second = parts.next()?;
            let tail: Vec<&str> = parts.collect();
            if tail.is_empty() {
                return None;
            }
            (format!("{first}/{second}"), tail.join("/"))
        } else {
            let tail: Vec<&str> = parts.collect();
            if tail.is_empty() {
                return None;
            }
            (first.to_owned(), tail.join("/"))
        };
        Some((registry, format!("{ns}:{tag_path}")))
    }

    fn parse_values(bytes: &[u8], label: &str) -> Vec<serde_json::Value> {
        let doc: serde_json::Value =
            serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{label}: invalid JSON: {e}"));
        let obj = doc
            .as_object()
            .unwrap_or_else(|| panic!("{label}: expected JSON object"));
        assert_eq!(
            obj.len(),
            1,
            "{label}: expected only 'values' key, got {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        obj.get("values")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("{label}: missing 'values' array"))
            .clone()
    }

    fn load_from_local_root(root: &Path) -> TagValuesMap {
        let data_root = root.join("src/generated/resources/data");
        assert!(
            data_root.is_dir(),
            "SUNRISE_NEOFORGE_ROOT missing generated data dir: {}",
            data_root.display()
        );
        let mut out = BTreeMap::new();
        let mut stack = Vec::new();
        for ns in ["c", "neoforge"] {
            let dir = data_root.join(ns).join("tags");
            if dir.is_dir() {
                stack.push(dir);
            }
        }
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let rel = path
                    .strip_prefix(&data_root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let Some(key) = parse_tag_rel_path(&rel) else {
                    continue;
                };
                let bytes =
                    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                let values = parse_values(&bytes, &rel);
                assert!(
                    out.insert(key.clone(), values).is_none(),
                    "duplicate NeoForge tag {key:?}"
                );
            }
        }
        out
    }

    fn load_from_pinned_tarball() -> TagValuesMap {
        let response = ureq::get(NEOFORGE_TARBALL_URL)
            .call()
            .unwrap_or_else(|e| panic!("fetch NeoForge {NEOFORGE_COMMIT} tarball: {e}"));
        let mut archive_bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut archive_bytes)
            .unwrap_or_else(|e| panic!("read NeoForge {NEOFORGE_COMMIT} tarball: {e}"));
        let actual_sha256: String = Sha256::digest(&archive_bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            actual_sha256, NEOFORGE_TARBALL_SHA256,
            "NeoForge {NEOFORGE_COMMIT} tarball SHA-256 mismatch"
        );

        let gz = flate2::read::GzDecoder::new(archive_bytes.as_slice());
        let mut archive = tar::Archive::new(gz);
        let mut out = BTreeMap::new();
        for entry in archive.entries().expect("read NeoForge tar entries") {
            let mut entry = entry.expect("read NeoForge tar entry");
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry
                .path()
                .expect("tar entry path")
                .to_string_lossy()
                .replace('\\', "/");
            let Some(rel) = path
                .split_once('/')
                .and_then(|(_, rest)| rest.strip_prefix("src/generated/resources/data/"))
            else {
                continue;
            };
            let Some(key) = parse_tag_rel_path(rel) else {
                continue;
            };
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("read tag JSON bytes");
            let values = parse_values(&bytes, rel);
            assert!(
                out.insert(key.clone(), values).is_none(),
                "duplicate NeoForge tag {key:?}"
            );
        }
        out
    }

    /// Compares `crates/sunrise-world/data/convention_tags.json` against NeoForge 26.3's
    /// generated `data/{c,neoforge}/tags/` files at pinned commit `bfed31772c6b`: exact
    /// id set, exact values (`#` references and `required: false` objects preserved), and
    /// exact entry order.
    #[test]
    #[ignore = "daily"]
    fn convention_tags_match_neoforge_26_3_generated_data() {
        let expected = match std::env::var_os("SUNRISE_NEOFORGE_ROOT") {
            Some(root) => load_from_local_root(Path::new(&root)),
            None => load_from_pinned_tarball(),
        };
        assert_eq!(
            expected.len(),
            540,
            "expected 540 generated convention tags in NeoForge {NEOFORGE_COMMIT}"
        );

        let mut actual = BTreeMap::new();
        for (registry, tags) in all() {
            for tag in tags {
                let key = (registry.clone(), tag.id.clone());
                assert!(
                    actual.insert(key.clone(), tag.values.clone()).is_none(),
                    "duplicate Sunrise convention tag {key:?}"
                );
            }
        }

        let missing: Vec<_> = expected
            .keys()
            .filter(|k| !actual.contains_key(*k))
            .collect();
        let extra: Vec<_> = actual
            .keys()
            .filter(|k| !expected.contains_key(*k))
            .collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "convention tag id mismatch vs NeoForge {NEOFORGE_COMMIT}: missing={missing:?}, extra={extra:?}"
        );

        let mut diffs = Vec::new();
        for (key, expected_values) in &expected {
            let actual_values = &actual[key];
            if actual_values != expected_values {
                diffs.push((key, expected_values, actual_values));
            }
        }
        assert!(
            diffs.is_empty(),
            "{} convention tag(s) differ from NeoForge {NEOFORGE_COMMIT}; first: {:?}",
            diffs.len(),
            diffs.first()
        );
    }
}
