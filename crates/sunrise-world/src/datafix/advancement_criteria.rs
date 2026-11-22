//! The criteria object in an advancements JSON file. DFU reads a typed
//! criteria map as a `CompoundList`: it maps the names in JSON encounter order,
//! then `JsonOps.createMap` keeps the last value written at a duplicate key.
//! Keep the pair list at the JSON boundary so NBT's HashMap order cannot choose
//! which progress value survives a rename.
use super::files::{AdvancementJson, json_to_tag, tag_to_json};
use super::json::Json;
use sunrise_core::nbt::{JavaString, Tag};

pub(super) fn read(value: &Json) -> Tag {
    match value {
        Json::Obj(entries) => Tag::List(vec![
            Tag::LongArray(Vec::new()),
            Tag::List(
                entries
                    .iter()
                    .map(|(k, v)| {
                        Tag::List(vec![
                            Tag::String(JavaString(k.clone())),
                            json_to_tag(v, AdvancementJson::Other),
                        ])
                    })
                    .collect(),
            ),
        ]),
        other => json_to_tag(other, AdvancementJson::Other),
    }
}

// JSON arrays become lists, not LongArrays, so the marker cannot appear in
// decoded JSON. Non-object criteria stay opaque to the key transform.
fn pairs(criteria: &mut Tag) -> Option<&mut [Tag]> {
    if let Tag::List(marker) = criteria
        && let [Tag::LongArray(_), Tag::List(pairs)] = marker.as_mut_slice()
    {
        Some(pairs)
    } else {
        None
    }
}

pub(super) fn map_keys(criteria: &mut Tag, mut rename: impl FnMut(JavaString) -> JavaString) {
    if let Some(pairs) = pairs(criteria) {
        for entry in pairs {
            if let Tag::List(parts) = entry
                && let [Tag::String(key), _] = parts.as_mut_slice()
            {
                *key = rename(std::mem::take(key));
            }
        }
    } else if let Tag::Compound(c) = criteria {
        for (key, value) in std::mem::take(c) {
            c.insert(rename(key), value);
        }
    }
}

pub(super) fn write(value: &Tag) -> Json {
    if let Tag::List(marker) = value
        && let [Tag::LongArray(_), Tag::List(pairs)] = marker.as_slice()
    {
        let mut entries = Vec::with_capacity(pairs.len());
        for entry in pairs {
            if let Tag::List(parts) = entry
                && let [Tag::String(key), value] = parts.as_slice()
            {
                Json::insert(
                    &mut entries,
                    key.0.clone(),
                    tag_to_json(value, AdvancementJson::Other),
                );
            }
        }
        Json::Obj(entries)
    } else {
        tag_to_json(value, AdvancementJson::Other)
    }
}
