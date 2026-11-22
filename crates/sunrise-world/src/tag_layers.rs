//! Fold resource-pack tag layers, then bind the resulting entries to a registry.
mod order;
pub(crate) use order::dependency_order;

use crate::loot::PackOverlay;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::{io, path::Path};
use sunrise_core::identifier::Identifier;

/// A tag's entries over its layers, in pack order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagLayers(Vec<Entry>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    required: bool,
    remove: bool,
}

#[derive(Deserialize)]
struct TagFile {
    values: Vec<Value>,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    remove: Vec<Value>,
}

#[derive(Deserialize)]
struct FileEntry {
    id: String,
    required: Option<bool>,
    #[serde(default)]
    remove: bool,
}

/// A tag file entry's id: a string, or an object's `id` (`TagEntry.CODEC`).
pub fn entry_id(entry: &Value) -> Option<&str> {
    entry.as_str().or_else(|| entry.get("id")?.as_str())
}

impl TagLayers {
    /// The layers of one file.
    pub fn from_file(file: Value) -> io::Result<Self> {
        Self::decode(file).map(|(_, layer)| layer)
    }

    /// Decode the complete file before applying it over any lower layers
    /// (`TagLoader.load`). Inside `values`, a content overlay's
    /// `{"id": ..., "remove": true}` entry removes like a `remove` entry.
    fn decode(file: Value) -> io::Result<(bool, Self)> {
        // Serde also accepts positional arrays for structs; the tag codec requires an object.
        if !file.is_object() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected a tag object",
            ));
        }
        let TagFile {
            values,
            replace,
            remove,
        } = serde_json::from_value(file)?;
        let entries = values
            .into_iter()
            .map(|entry| (entry, false))
            .chain(remove.into_iter().map(|entry| (entry, true)))
            .map(|(entry, removal)| {
                let (id, required, remove) = match entry {
                    Value::String(id) => (id, true, removal),
                    Value::Object(_) => {
                        let FileEntry {
                            id,
                            required,
                            remove,
                        } = serde_json::from_value(entry)?;
                        (id, required.unwrap_or(true), removal || remove)
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "expected a tag entry string or object",
                        ));
                    }
                };
                let (prefix, name) = id
                    .strip_prefix('#')
                    .map_or(("", id.as_str()), |name| ("#", name));
                let name = Identifier::parse(name)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                Ok(Entry {
                    id: format!("{prefix}{name}"),
                    required,
                    remove,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok((replace, Self(entries)))
    }

    fn push(&mut self, (replace, layer): (bool, Self)) {
        if replace {
            self.0.clear();
        }
        self.0.extend(layer.0);
    }

    /// The `#tags` the entries name, adding or removing.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|entry| entry.id.strip_prefix('#'))
    }

    /// Bind one tag to registry IDs and already bound nested tags. A missing
    /// required addition fails the entire tag; missing optional entries and
    /// removals never do. Members retain their insertion order.
    pub fn build<'a>(
        &self,
        contains: impl Fn(&str) -> bool,
        mut members: impl FnMut(&str) -> Option<&'a [String]>,
    ) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for entry in &self.0 {
            let ids = match entry.id.strip_prefix('#') {
                Some(tag) => members(tag),
                None => contains(&entry.id).then_some(std::slice::from_ref(&entry.id)),
            };
            let Some(ids) = ids else {
                if entry.required && !entry.remove {
                    return None;
                }
                continue;
            };
            for id in ids {
                if entry.remove {
                    out.retain(|kept| kept != id);
                } else if !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
        Some(out)
    }

    /// Bind the registry's complete tag stack before publishing it. Admit all
    /// required dependencies before optional ones, dropping edges that would
    /// close a cycle. Build each tag once, after its admitted dependencies.
    pub fn bind(
        kind: &str,
        raw: &HashMap<String, Self>,
        contains: impl Fn(&str) -> bool,
    ) -> HashMap<String, Vec<String>> {
        let mut names: Vec<_> = raw.keys().map(String::as_str).collect();
        // ResourceManager lists tag files in Identifier order (path first,
        // namespace second), including the extension in the path comparison.
        names.sort_by_cached_key(|name| {
            let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
            (format!("{path}.json"), namespace)
        });
        let sorted = dependency_order(names, |name| {
            raw[name].0.iter().filter_map(|entry| {
                Some((entry.id.strip_prefix('#')?, entry.required && !entry.remove))
            })
        });
        let mut bound: HashMap<String, Vec<String>> = HashMap::new();
        for name in sorted {
            let Some(entries) = raw.get(name) else {
                continue;
            };
            match entries.build(&contains, |id| bound.get(id).map(Vec::as_slice)) {
                Some(members) => {
                    bound.insert(name.to_owned(), members);
                }
                None => {
                    log::warn!("Sunrise: couldn't bind {kind} tag {name}: missing required member")
                }
            }
        }
        bound
    }
}

/// Read a registry's resource and world-pack tag stack, then bind it against
/// the registry admitted by the owning load. Conditions do not gate tag files.
pub fn load(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay<'_>,
    contains: impl Fn(&str) -> bool,
) -> io::Result<HashMap<String, Vec<String>>> {
    Ok(TagLayers::bind(
        kind,
        &layers(root, kind, overlay)?,
        contains,
    ))
}

pub(crate) fn layers(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay<'_>,
) -> io::Result<HashMap<String, TagLayers>> {
    let mut raw = HashMap::<_, TagLayers>::new();
    let data = root.join("data");
    for namespace in crate::json_folder::namespaces(&data) {
        let dir = data.join(&namespace).join("tags").join(kind);
        let folder = match crate::json_folder::read(&dir) {
            Ok(folder) => folder,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for file in &folder.files {
            match file
                .bytes()
                .and_then(|bytes| TagLayers::decode(serde_json::from_slice(bytes)?))
            {
                Ok((_, layer)) => {
                    raw.insert(format!("{namespace}:{}", file.name), layer);
                }
                Err(error) => log::warn!(
                    "Sunrise: couldn't read tag file {}: {error}",
                    file.path.display()
                ),
            }
        }
    }
    for (id, files) in overlay.tag_kind(kind) {
        for bytes in files {
            let layer = serde_json::from_slice(bytes)
                .map_err(io::Error::from)
                .and_then(TagLayers::decode);
            match layer {
                Ok(layer) => raw.entry(id.clone()).or_default().push(layer),
                Err(error) => log::warn!("Sunrise: couldn't read {kind} tag {id}: {error}"),
            }
        }
    }
    Ok(raw)
}

/// Registry IDs from the prepared game report, which includes admitted
/// content-mod entries. A synthetic root may not have a report.
pub fn builtin_ids(root: &Path) -> io::Result<HashMap<String, HashSet<String>>> {
    let path = root.join("registries.json");
    let value = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error),
    };
    Ok(value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(kind, value)| {
            let entries = value["entries"].as_object()?;
            Some((
                kind.strip_prefix("minecraft:").unwrap_or(kind).to_owned(),
                entries.keys().cloned().collect(),
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removals_apply_in_layer_order_and_replace_drops_them() {
        let mut things =
            TagLayers::from_file(json!({"values": ["stone", "dirt", "#test:other"]})).unwrap();
        let decode = |file| TagLayers::decode(file).unwrap();
        things.push(decode(
            json!({"values": [], "remove": ["dirt", "#test:other"]}),
        ));
        things.push(decode(
            json!({"values": ["gravel", "sand", {"id":"stone", "remove":true}]}),
        ));
        let other = ["minecraft:sand".into()];
        let members = |tag: &str| (tag == "test:other").then_some(other.as_slice());
        assert_eq!(
            things.build(|_| true, members).unwrap(),
            ["minecraft:gravel", "minecraft:sand"]
        );
        things.push(decode(json!({"replace": true, "values": ["dirt"]})));
        assert_eq!(things.build(|_| true, members).unwrap(), ["minecraft:dirt"]);
    }

    #[test]
    fn malformed_replacements_leave_lower_entity_tag_layers_bound() {
        let key = "tags/test:entity_type/living";
        let lower = br#"{"values":["minecraft:cow"]}"#;
        let root = std::path::PathBuf::new();
        for malformed in [
            br#"{"replace":true,"values":"minecraft:pig"}"#.as_slice(),
            br#"[["minecraft:pig"],true,[]]"#,
            br#"{"replace":true,"values":[["minecraft:pig",true,false]]}"#,
            br#"{"replace":true,"values":["minecraft:pig",false]}"#,
            br#"{"replace":true,"values":["minecraft:pig",{"id":"INVALID"}]}"#,
            br##"{"replace":true,"values":["minecraft:pig","#INVALID"]}"##,
            br#"{"replace":true,"values":[],"remove":{}}"#,
        ] {
            let files = [(key.into(), vec![lower.to_vec(), malformed.to_vec()])];
            let bound = load(&root, "entity_type", &PackOverlay::new(&files), |_| true).unwrap();
            assert_eq!(bound["test:living"], ["minecraft:cow"]);
        }
    }

    #[test]
    fn unresolved_required_entries_omit_tags_and_optional_entries_do_not() {
        let raw = [
            ("test:empty", json!({"values":[]})),
            ("test:required", json!({"values":["test:pinned", {"id":"test:absent", "required":true}]})),
            ("test:dependent", json!({"values":["#test:required"]})),
            ("test:optional", json!({"values":[{"id":"test:absent", "required":false}, {"id":"#test:required", "required":false}], "remove":["#test:required", "test:absent"]})),
            ("test:admitted", json!({"values":["#test:empty", "test:pinned", "test:pinned"]})),
        ].into_iter().map(|(name, file)| (name.to_owned(), TagLayers::from_file(file).unwrap())).collect();
        assert_eq!(
            TagLayers::bind("item", &raw, |id| id == "test:pinned"),
            HashMap::from([
                ("test:empty".into(), vec![]),
                ("test:optional".into(), vec![]),
                ("test:admitted".into(), vec!["test:pinned".into()]),
            ])
        );
    }

    #[test]
    fn shared_nested_tags_are_built_once() {
        let mut raw = HashMap::new();
        for i in 0..128 {
            let child = format!("#test:{}", i + 1);
            raw.insert(
                format!("test:{i}"),
                TagLayers::from_file(json!({"values":[child,child]})).unwrap(),
            );
        }
        raw.insert(
            "test:128".into(),
            TagLayers::from_file(json!({"values":["minecraft:stick", "minecraft:potion"]}))
                .unwrap(),
        );
        let calls = std::cell::Cell::new(0);
        let tags = TagLayers::bind("item", &raw, |_| {
            calls.set(calls.get() + 1);
            assert!(calls.get() <= 2);
            true
        });
        assert_eq!(tags["test:0"], ["minecraft:stick", "minecraft:potion"]);
    }

    #[test]
    fn an_optional_cycle_binds_in_java_identifier_bucket_order() {
        let raw = HashMap::from([
            (
                "test:a".into(),
                TagLayers::from_file(
                    json!({"values": ["sand", {"id":"#test:b", "required":false}]}),
                )
                .unwrap(),
            ),
            (
                "test:b".into(),
                TagLayers::from_file(
                    json!({"values": ["dirt", {"id":"#test:a", "required":false}]}),
                )
                .unwrap(),
            ),
        ]);
        let bound = TagLayers::bind("block", &raw, |_| true);
        // b is in bucket 2 and a in bucket 13; b's optional edge wins.
        assert_eq!(bound["test:a"], ["minecraft:sand"]);
        assert_eq!(bound["test:b"], ["minecraft:dirt", "minecraft:sand"]);
    }

    #[test]
    fn required_dependencies_take_priority_over_optional_cycles() {
        let raw = HashMap::from([
            (
                "test:a".into(),
                TagLayers::from_file(json!({"values": ["#test:b", "sand"]})).unwrap(),
            ),
            (
                "test:b".into(),
                TagLayers::from_file(
                    json!({"values": [{"id":"#test:a", "required":false}, "dirt"]}),
                )
                .unwrap(),
            ),
        ]);
        let bound = TagLayers::bind("block", &raw, |_| true);
        assert_eq!(bound["test:a"], ["minecraft:dirt", "minecraft:sand"]);
        assert_eq!(bound["test:b"], ["minecraft:dirt"]);
    }
}
