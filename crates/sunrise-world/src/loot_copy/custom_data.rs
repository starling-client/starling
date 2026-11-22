//! The shared copy_custom_data operation; adapters supply only source and stack data.
use crate::nbt_path::{NbtPath, merge_compound};
use serde_json::Value;
use std::borrow::Cow;
use sunrise_core::nbt::{Compound, Tag};

#[derive(Clone, Debug)]
pub enum NbtSource<'a> {
    Context(&'a str),
    Storage(sunrise_core::identifier::Identifier),
}

fn provider(value: &Value) -> Option<NbtSource<'_>> {
    match value {
        Value::String(target) => Some(NbtSource::Context(target)),
        Value::Object(object) => {
            let kind = object.get("type")?.as_str()?;
            match kind.strip_prefix("minecraft:").unwrap_or(kind) {
                "context" => Some(NbtSource::Context(object.get("target")?.as_str()?)),
                "storage" => Some(NbtSource::Storage(
                    sunrise_core::identifier::Identifier::parse(object.get("source")?.as_str()?)
                        .ok()?,
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Strategy {
    Replace,
    Append,
    Merge,
}
struct Operation {
    source: NbtPath,
    target: NbtPath,
    strategy: Strategy,
}

/// Return a new custom-data compound only when an operation actually changed it.
/// Both evaluators use the same path parser, matching rules and write operations.
pub fn copy_custom_data<'a>(
    function: &Value,
    original: Option<&Compound>,
    source: impl FnOnce(NbtSource<'_>) -> Option<Cow<'a, Tag>>,
) -> Option<Compound> {
    let source = source(provider(&function["source"])?)?;
    let operations = function["ops"]
        .as_array()?
        .iter()
        .map(|entry| {
            Some(Operation {
                source: NbtPath::parse(entry["source"].as_str()?).ok()?,
                target: NbtPath::parse(entry["target"].as_str()?).ok()?,
                strategy: match entry["op"].as_str()? {
                    "replace" => Strategy::Replace,
                    "append" => Strategy::Append,
                    "merge" => Strategy::Merge,
                    _ => return None,
                },
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let mut edited = None;
    for operation in operations {
        let Ok(values) = operation.source.get(&source) else {
            continue;
        };
        let Some(last) = values.last() else { continue };
        // Like set/insert, reject an over-depth write before creating any parents.
        if matches!(operation.strategy, Strategy::Append | Strategy::Merge)
            && operation.target.check_depth(&values).is_err()
        {
            continue;
        }
        let target =
            edited.get_or_insert_with(|| Tag::Compound(original.cloned().unwrap_or_default()));
        match operation.strategy {
            Strategy::Replace => {
                let _ = operation.target.set(target, last);
            }
            Strategy::Append => {
                let _ = operation.target.get_or_create(
                    target,
                    &|| Tag::List(Vec::new()),
                    &mut |target| {
                        if let Tag::List(list) = target {
                            list.extend(values.iter().cloned());
                        }
                    },
                );
            }
            Strategy::Merge => {
                let _ = operation.target.get_or_create(
                    target,
                    &|| Tag::Compound(Compound::new()),
                    &mut |target| {
                        if let Tag::Compound(compound) = target {
                            for value in &values {
                                if let Tag::Compound(source) = value {
                                    merge_compound(compound, source);
                                }
                            }
                        }
                    },
                );
            }
        }
    }
    let Tag::Compound(edited) = edited? else {
        return None;
    };
    let changed = original.map_or(!edited.is_empty(), |before| before != &edited);
    changed.then_some(edited)
}
