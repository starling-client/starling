//! Version-pinned tag codecs, resource-stack merging and dependency resolution.
use super::{DecodeError, Object, optional, required};
use crate::tag_layers::dependency_order;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntry {
    pub id: String,
    pub tag: bool,
    pub required: bool,
}
impl TagEntry {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let raw: Box<RawValue> = serde_json::from_str(json)?;
        let (name, mandatory) = if raw.get().starts_with('"') {
            (serde_json::from_str::<String>(raw.get())?, true)
        } else {
            let object: Object = serde_json::from_str(raw.get())?;
            (
                serde_json::from_str(required(&object, "id")?.get())?,
                optional(&object, "required")
                    .map(|v| serde_json::from_str(v.get()))
                    .transpose()?
                    .unwrap_or(true),
            )
        };
        let (tag, name) = name
            .strip_prefix('#')
            .map_or((false, name.as_str()), |s| (true, s));
        Ok(Self {
            id: identifier(name)?,
            tag,
            required: mandatory,
        })
    }
    pub fn display(&self) -> String {
        format!(
            "{}{}{}",
            if self.tag { "#" } else { "" },
            self.id,
            if self.required { "" } else { "?" }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagFile {
    pub entries: Vec<TagEntry>,
    pub replace: bool,
    /// NeoForge's `TagFile.remove`: entries taken out after this file's `entries`.
    pub remove: Vec<TagEntry>,
    entry_removals: Vec<bool>,
}
impl TagFile {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let list = |raw: &RawValue| -> Result<Vec<TagEntry>, DecodeError> {
            let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
            values
                .iter()
                .map(|v| TagEntry::from_json(v.get()))
                .collect()
        };
        let raw_entries: Vec<serde_json::Value> =
            serde_json::from_str(required(&object, "values")?.get())?;
        let entry_removals = raw_entries
            .iter()
            .map(|entry| entry.get("remove").and_then(serde_json::Value::as_bool) == Some(true))
            .collect();
        let entries = list(required(&object, "values")?)?;
        let replace = optional(&object, "replace")
            .map(|v| serde_json::from_str(v.get()))
            .transpose()?
            .unwrap_or(false);
        let remove = optional(&object, "remove").map(list).transpose()?;
        Ok(Self {
            entries,
            replace,
            remove: remove.unwrap_or_default(),
            entry_removals,
        })
    }
}
// Identifier.CODEC permits the empty path as well as an omitted namespace.
pub(crate) fn identifier(name: &str) -> Result<String, DecodeError> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid) || !path.bytes().all(|c| valid(c) || c == b'/') {
        return Err(DecodeError::Registry(format!(
            "invalid tag identifier: {name}"
        )));
    }
    Ok(format!("{namespace}:{path}"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntryWithSource {
    pub entry: TagEntry,
    pub source: String,
    /// A `remove` entry (NeoForge's `TagLoader.EntryWithSource.remove`).
    pub remove: bool,
}
#[derive(Clone, Debug, Default)]
pub struct TagDefinitions {
    entries: BTreeMap<String, Vec<TagEntryWithSource>>,
    insertion_order: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct BuiltTags<T> {
    pub tags: BTreeMap<String, Vec<T>>,
    pub missing: BTreeMap<String, Vec<TagEntryWithSource>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagBuildError {
    TooManyTags,
}
impl std::fmt::Display for TagBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooManyTags => "tag definitions exceed 8192 entries",
        })
    }
}
impl std::error::Error for TagBuildError {}
impl TagDefinitions {
    /// Apply resources in their pack priority order. A successfully decoded
    /// replace file clears earlier entries; a rejected file changes nothing.
    pub fn add(&mut self, name: &str, source: &str, file: TagFile) -> Result<(), DecodeError> {
        let name = identifier(name)?;
        if !self.entries.contains_key(&name) {
            self.insertion_order.push(name.clone());
        }
        let entries = self.entries.entry(name).or_default();
        if file.replace {
            entries.clear();
        }
        entries.extend(file.entries.into_iter().zip(file.entry_removals).map(
            |(mut entry, remove)| {
                if remove {
                    entry.required = false;
                }
                TagEntryWithSource {
                    entry,
                    source: source.into(),
                    remove,
                }
            },
        ));
        // Removals are optional: a missing id or tag removes nothing.
        entries.extend(file.remove.into_iter().map(|entry| TagEntryWithSource {
            entry: TagEntry {
                required: false,
                ..entry
            },
            source: source.into(),
            remove: true,
        }));
        Ok(())
    }
    /// A valid JSON resource with a codec error still creates an empty tag in
    /// vanilla's resource loader; existing entries are retained on that error.
    pub fn apply_json(&mut self, name: &str, source: &str, json: &str) -> Result<(), DecodeError> {
        let _: Box<RawValue> = serde_json::from_str(json)?;
        let name = identifier(name)?;
        if !self.entries.contains_key(&name) {
            self.insertion_order.push(name.clone());
            self.entries.insert(name.clone(), Vec::new());
        }
        self.add(&name, source, TagFile::from_json(json)?)
    }
    pub fn definitions(&self) -> &BTreeMap<String, Vec<TagEntryWithSource>> {
        &self.entries
    }
    /// Build the bounded worldgen table. Missing required references
    /// invalidate their own tags, not unrelated tags.
    pub fn build<T: Clone + Ord>(
        &self,
        lookup: impl FnMut(&str, bool) -> Option<T>,
    ) -> Result<BuiltTags<T>, TagBuildError> {
        if self.entries.len() > 8192 {
            return Err(TagBuildError::TooManyTags);
        }
        Ok(self.bind(lookup))
    }

    /// Bind the tags of a registry without a worldgen table size limit.
    /// Element lookup receives the entry's required flag, as in vanilla.
    pub fn bind<T: Clone + Ord>(
        &self,
        mut lookup: impl FnMut(&str, bool) -> Option<T>,
    ) -> BuiltTags<T> {
        let sorted = dependency_order(self.insertion_order.iter().map(String::as_str), |name| {
            self.entries[name].iter().filter_map(|entry| {
                let entry = &entry.entry;
                entry.tag.then_some((entry.id.as_str(), entry.required))
            })
        });
        let mut result: BuiltTags<T> = BuiltTags {
            tags: BTreeMap::new(),
            missing: BTreeMap::new(),
        };
        for name in sorted {
            let Some(entries) = self.entries.get(name) else {
                continue;
            };
            let mut seen = BTreeSet::new();
            let mut values = Vec::new();
            let mut missing = Vec::new();
            for sourced in entries {
                let entry = &sourced.entry;
                if entry.tag {
                    if let Some(children) = result.tags.get(&entry.id) {
                        for value in children {
                            apply(&mut values, &mut seen, value, sourced.remove);
                        }
                    } else if entry.required {
                        missing.push(sourced.clone());
                    }
                } else if let Some(value) = lookup(&entry.id, entry.required) {
                    apply(&mut values, &mut seen, &value, sourced.remove);
                } else if entry.required {
                    missing.push(sourced.clone());
                }
            }
            if missing.is_empty() {
                result.tags.insert(name.into(), values);
            } else {
                result.missing.insert(name.into(), missing);
            }
        }
        result
    }
}
/// One entry's value into a tag being built: added at the end, or removed
/// (`values::add` or NeoForge's `values::remove` on the tag's ordered set).
fn apply<T: Clone + Ord>(values: &mut Vec<T>, seen: &mut BTreeSet<T>, value: &T, remove: bool) {
    if !remove {
        if seen.insert(value.clone()) {
            values.push(value.clone());
        }
    } else if seen.remove(value) {
        values.retain(|v| v != value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_replacement_keeps_entries_and_codec_error_can_create_empty_tag() {
        let mut definitions = TagDefinitions::default();
        definitions
            .apply_json("a", "first", r##"{"values":["one","two","one"]}"##)
            .unwrap();
        assert!(
            definitions
                .apply_json("a", "bad", r##"{"replace":true,"values":[false]}"##)
                .is_err()
        );
        assert!(definitions.apply_json("empty", "bad", "{}").is_err());
        assert!(definitions.apply_json("syntax", "bad", "{").is_err());
        let built = definitions.build(|id, _| Some(id.to_owned())).unwrap();
        assert_eq!(
            built.tags["minecraft:a"],
            vec!["minecraft:one", "minecraft:two"]
        );
        assert_eq!(built.tags["minecraft:empty"], Vec::<String>::new());
        assert!(!built.tags.contains_key("minecraft:syntax"));
        definitions
            .apply_json(
                "a",
                "replacement",
                r##"{"replace":true,"values":["three"]}"##,
            )
            .unwrap();
        let built = definitions.build(|id, _| Some(id.to_owned())).unwrap();
        assert_eq!(built.tags["minecraft:a"], vec!["minecraft:three"]);
    }
    #[test]
    fn a_higher_packs_remove_takes_entries_and_tag_members_out() {
        let mut definitions = TagDefinitions::default();
        for (name, source, json) in [
            ("things", "a", r##"{"values":["stone","dirt","#other"]}"##),
            ("other", "a", r##"{"values":["sand"]}"##),
            (
                "things",
                "b",
                r##"{"values":[],"remove":["dirt","#other",{"id":"absent","required":true}]}"##,
            ),
        ] {
            definitions.apply_json(name, source, json).unwrap();
        }
        let built = definitions
            .build(|id, _| (!id.ends_with("absent")).then(|| id.to_owned()))
            .unwrap();
        assert_eq!(built.tags["minecraft:things"], vec!["minecraft:stone"]);
        assert!(built.missing.is_empty());
    }
    #[test]
    fn ordered_removal_objects_survive_the_worldgen_codec() {
        let mut definitions = TagDefinitions::default();
        definitions.apply_json("things", "fixture", r##"{"values":["stone", {"id":"stone","remove":true}, "stone", {"id":"stone","remove":true}, {"id":"absent","remove":true}]}"##).unwrap();
        let built = definitions
            .build(|id, _| (!id.ends_with("absent")).then(|| id.to_owned()))
            .unwrap();
        assert!(built.tags["minecraft:things"].is_empty());
        assert!(built.missing.is_empty());
    }

    #[test]
    fn required_edges_precede_optional_edges_and_missing_tags_do_not_poison_siblings() {
        let mut definitions = TagDefinitions::default();
        for (name, json) in [
            ("a", r##"{"values":[{"id":"#b","required":false},"one"]}"##),
            ("b", r##"{"values":["#a","two"]}"##),
            ("bad", r##"{"values":["absent","also_absent"]}"##),
            (
                "c",
                r##"{"values":[{"id":"#bad","required":false},"two"]}"##,
            ),
        ] {
            definitions.apply_json(name, "fixture", json).unwrap();
        }
        let built = definitions
            .build(|id, _| (!id.ends_with("absent")).then(|| id.to_owned()))
            .unwrap();
        assert_eq!(built.tags["minecraft:a"], vec!["minecraft:one"]);
        assert_eq!(
            built.tags["minecraft:b"],
            vec!["minecraft:one", "minecraft:two"]
        );
        assert_eq!(built.tags["minecraft:c"], vec!["minecraft:two"]);
        assert_eq!(
            built.missing["minecraft:bad"]
                .iter()
                .map(|e| e.entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["minecraft:absent", "minecraft:also_absent"]
        );
    }
}
