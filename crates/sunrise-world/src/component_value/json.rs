//! `ItemStack.CODEC` / `ItemStack.OPTIONAL_CODEC` JSON encoding and decoding over `JsonOps`.
//!
//! Shares the canonical [`Template`] and [`Value`] codecs in [`super::source`]: JSON input is
//! validated against `JsonOps` type rules (booleans versus numbers, JSON object requirements for
//! `Unit` and component removals, `null` as absent in optional record fields) before delegating
//! to the shared component decoder, and encoded back directly from [`Template`] / [`Value`] so
//! booleans, integers, floats, text components, identifiers, and `"!id": {}` removals match
//! `ItemStack.CODEC` with `JsonOps`.

use super::{Patch, Template, Value, effective_patch, is_persistent, source, types};
use serde_json::{Map, Number, Value as JsonValue};
use std::collections::BTreeMap;
use sunrise_core::{
    java_number::float_to_string,
    json_nbt::{json_to_tag, tag_to_json},
    nbt::{Compound, JavaString, Tag, discard_tag, with_tree_stack},
};

/// Failure encoding or decoding an `ItemStack` JSON document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonError {
    /// Input text is not syntactically valid JSON.
    Syntax(String),
    /// A named top-level field or component key failed codec validation or encoding.
    Field { field: String, reason: String },
    /// A top-level value or codec error not tied to one named field.
    Value(String),
}

impl JsonError {
    pub fn field(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Field {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// Convert a `"<field>: <reason>"` diagnostic from [`super::source`] or [`Value::to_json`].
    pub fn from_codec_error(error: String) -> Self {
        if let Some((prefix, rest)) = error.split_once(": ")
            && (matches!(prefix, "id" | "count" | "components")
                || prefix.contains(':')
                || prefix.starts_with('!'))
        {
            return Self::Field {
                field: prefix.to_owned(),
                reason: rest.to_owned(),
            };
        }
        if error.contains("template count") {
            return Self::Field {
                field: "count".into(),
                reason: error,
            };
        }
        Self::Value(error)
    }
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(reason) => write!(f, "invalid JSON: {reason}"),
            Self::Field { field, reason } => write!(f, "{field}: {reason}"),
            Self::Value(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for JsonError {}

impl Value {
    /// Encode one canonical component codec value with `JsonOps` semantics:
    /// real JSON booleans, integer versus Java-formatted float numbers, strings,
    /// arrays, objects, and nested `ItemStackTemplate`s.
    pub fn to_json(&self) -> Result<JsonValue, String> {
        with_tree_stack(|| self.to_json_with_stack())
    }

    fn to_json_with_stack(&self) -> Result<JsonValue, String> {
        Ok(match self {
            Self::Empty => JsonValue::Null,
            Self::Byte(v) => JsonValue::from(*v),
            Self::Short(v) => JsonValue::from(*v),
            Self::Int(v) => JsonValue::from(*v),
            Self::Long(v) => JsonValue::from(*v),
            Self::Float(bits) => {
                let value = f32::from_bits(*bits);
                if !value.is_finite() {
                    return Err("JSON cannot represent a non-finite number".into());
                }
                let parsed = float_to_string(value)
                    .parse::<f64>()
                    .ok()
                    .and_then(Number::from_f64)
                    .ok_or_else(|| "JSON cannot represent a non-finite number".to_owned())?;
                JsonValue::Number(parsed)
            }
            Self::Double(bits) => {
                let value = f64::from_bits(*bits);
                let number = Number::from_f64(value)
                    .ok_or_else(|| "JSON cannot represent a non-finite number".to_owned())?;
                JsonValue::Number(number)
            }
            Self::Bool(v) => JsonValue::Bool(*v),
            Self::String(units) => JsonValue::String(decode_utf16(units)?),
            Self::ByteArray(values) => {
                JsonValue::Array(values.iter().map(|v| JsonValue::from(*v as i8)).collect())
            }
            Self::IntArray(values) => {
                JsonValue::Array(values.iter().copied().map(JsonValue::from).collect())
            }
            Self::LongArray(values) => {
                JsonValue::Array(values.iter().copied().map(JsonValue::from).collect())
            }
            Self::List(values) | Self::Set(values) => JsonValue::Array(
                values
                    .iter()
                    .map(Self::to_json)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::Map(entries) => {
                let effective = entries
                    .iter()
                    .map(|(key, value)| match key {
                        Self::String(units) => Ok((units, value)),
                        _ => Err("component map key is not a string".to_owned()),
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?;
                let mut out = Map::new();
                for (key, value) in effective {
                    out.insert(decode_utf16(key)?, value.to_json()?);
                }
                JsonValue::Object(out)
            }
            Self::RegisteredConsumeEffect(effect) => {
                tag_to_json(&effect.saved).map_err(|error| error.to_string())?
            }
            Self::Template(template) => template.to_template_json()?,
            Self::Container(slots) => {
                let mut out = Vec::new();
                for (slot, item) in slots.iter().enumerate() {
                    if let Some(item) = item {
                        let mut entry = Map::new();
                        entry.insert("slot".into(), JsonValue::from(slot as i32));
                        entry.insert("item".into(), item.to_template_json()?);
                        out.push(JsonValue::Object(entry));
                    }
                }
                JsonValue::Array(out)
            }
            Self::StreamOnly(_) => return Err("inline holder has no persistent encoding".into()),
        })
    }
}

impl Template {
    /// Encode as `ItemStack.CODEC` over `JsonOps`: `"id"`, always-present `"count"`,
    /// and optional `"components"` when the persistent patch is non-empty.
    pub fn to_item_stack_json(&self) -> Result<JsonValue, JsonError> {
        if self.item == "minecraft:air" {
            return Err(JsonError::field("id", "air is not a valid item"));
        }
        source::validate_template_count(self.count)
            .map_err(|error| JsonError::field("count", error))?;
        let mut out = Map::new();
        out.insert("id".into(), JsonValue::String(self.item.clone()));
        out.insert("count".into(), JsonValue::from(self.count));
        if let Some(patch) =
            template_patch_json(&self.patch).map_err(JsonError::from_codec_error)?
        {
            out.insert("components".into(), patch);
        }
        Ok(JsonValue::Object(out))
    }

    /// Encode as `ItemStackTemplate.CODEC` over `JsonOps` (omitting `"count"` when 1).
    pub fn to_template_json(&self) -> Result<JsonValue, String> {
        let mut out = Map::new();
        out.insert("id".into(), JsonValue::String(self.item.clone()));
        if self.count != 1 {
            out.insert("count".into(), JsonValue::from(self.count));
        }
        if let Some(patch) = template_patch_json(&self.patch)? {
            out.insert("components".into(), patch);
        }
        Ok(JsonValue::Object(out))
    }
}

fn template_patch_json(patch: &Patch) -> Result<Option<JsonValue>, String> {
    let mut out = Map::new();
    for (name, value) in effective_patch(patch) {
        if !is_persistent(name) {
            continue;
        }
        match value {
            Some(value) => {
                let encoded = value
                    .to_json()
                    .map_err(|error| format!("{name}: {error}"))?;
                out.insert(name.clone(), encoded);
            }
            None => {
                out.insert(format!("!{name}"), JsonValue::Object(Map::new()));
            }
        }
    }
    Ok((!out.is_empty()).then_some(JsonValue::Object(out)))
}

fn decode_utf16(units: &[u16]) -> Result<String, String> {
    char::decode_utf16(units.iter().copied())
        .collect::<Result<String, _>>()
        .map_err(|_| "JSON cannot represent an unpaired UTF-16 surrogate".to_owned())
}

/// Decode `ItemStack.OPTIONAL_CODEC` (`ExtraCodecs.optionalEmptyMap(ItemStack.CODEC)`) over
/// `JsonOps`: `{}` returns `Ok(None)`; a non-empty object validates `id`, `count` and `components`
/// against `JsonOps` rules and the caller's registries.
pub fn optional_item_stack_from_json_with(
    value: &JsonValue,
    is_native: impl Fn(&str) -> bool,
    has_item: &dyn Fn(&str) -> bool,
) -> Result<Option<Template>, JsonError> {
    let Some(map) = value.as_object() else {
        return Err(JsonError::Value("expected JSON object".into()));
    };
    if map.is_empty() {
        return Ok(None);
    }

    let raw_id = match map.get("id") {
        None | Some(JsonValue::Null) => return Err(JsonError::field("id", "missing item id")),
        Some(JsonValue::String(id)) => id,
        Some(_) => return Err(JsonError::field("id", "expected string")),
    };
    let id = source::identifier(raw_id).map_err(|error| JsonError::field("id", error))?;
    if id == "minecraft:air" {
        return Err(JsonError::field("id", "air is not a valid item"));
    }
    if !has_item(&id) {
        return Err(JsonError::field("id", format!("unknown item {id}")));
    }

    let count = match map.get("count") {
        None | Some(JsonValue::Null) => 1,
        Some(value) => decode_json_int(value)
            .map_err(|error| JsonError::field("count", error))
            .and_then(|count| {
                source::validate_template_count(count)
                    .map_err(|error| JsonError::field("count", error))?;
                Ok(count)
            })?,
    };

    let mut root = Compound::new();
    root.insert(JavaString::from("id"), Tag::String(id.as_str().into()));
    root.insert(JavaString::from("count"), Tag::Int(count));
    match map.get("components") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Object(components)) => {
            let patch = prepare_patch_json(components, &is_native)?;
            if !patch.is_empty() {
                root.insert(JavaString::from("components"), Tag::Compound(patch));
            }
        }
        Some(_) => {
            return Err(JsonError::field("components", "expected JSON object"));
        }
    }

    let tag = Tag::Compound(root);
    let result = source::item_stack_from_tag_with(&tag, &is_native, has_item)
        .map_err(JsonError::from_codec_error);
    discard_tag(tag);
    result.map(Some)
}

fn decode_json_int(value: &JsonValue) -> Result<i32, String> {
    if !value.is_number() {
        return Err("expected integer".into());
    }
    let tag = json_to_tag(value).map_err(|error| error.to_string())?;
    Ok(match tag {
        Tag::Byte(v) => i32::from(v),
        Tag::Short(v) => i32::from(v),
        Tag::Int(v) => v,
        Tag::Long(v) => v as i32,
        Tag::Float(bits) => f32::from_bits(bits) as i32,
        Tag::Double(bits) => f64::from_bits(bits) as i32,
        _ => return Err("expected integer".into()),
    })
}

fn prepare_patch_json(
    components: &Map<String, JsonValue>,
    is_native: &dyn Fn(&str) -> bool,
) -> Result<Compound, JsonError> {
    let mut out = Compound::new();
    for (raw_key, raw_value) in components {
        if let Some(removed_key) = raw_key.strip_prefix('!') {
            let id = source::identifier(removed_key)
                .map_err(|error| JsonError::field(raw_key, error))?;
            let key = format!("!{id}");
            if types::builtin_component_id(&id).is_none() && !is_native(&id) {
                return Err(JsonError::field(
                    key,
                    format!("unknown component type {id}"),
                ));
            }
            if !is_persistent(&id) {
                return Err(JsonError::field(
                    key,
                    format!("{id} has no persistent codec"),
                ));
            }
            if !raw_value.is_object() {
                return Err(JsonError::field(
                    key,
                    "removal payload must be a JSON object",
                ));
            }
            out.insert(
                JavaString::from(key.as_str()),
                Tag::Compound(Compound::new()),
            );
        } else {
            let id =
                source::identifier(raw_key).map_err(|error| JsonError::field(raw_key, error))?;
            if types::builtin_component_id(&id).is_none() && !is_native(&id) {
                return Err(JsonError::field(
                    id.clone(),
                    format!("unknown component type {id}"),
                ));
            }
            if !is_persistent(&id) {
                return Err(JsonError::field(
                    id.clone(),
                    format!("{id} has no persistent codec"),
                ));
            }
            let prepared = prepare_component_json(&id, raw_value, is_native)
                .map_err(|error| JsonError::field(id.clone(), error))?;
            out.insert(JavaString::from(id.as_str()), prepared);
        }
    }
    Ok(out)
}

fn expect_bool(v: &JsonValue) -> Result<Tag, String> {
    match v {
        JsonValue::Bool(b) => Ok(Tag::Byte(i8::from(*b))),
        _ => Err("expected boolean".into()),
    }
}

fn expect_number(v: &JsonValue) -> Result<Tag, String> {
    if !v.is_number() {
        return Err("expected number".into());
    }
    json_to_tag(v).map_err(|error| error.to_string())
}

fn expect_string(v: &JsonValue) -> Result<Tag, String> {
    match v {
        JsonValue::String(s) => Ok(Tag::String(s.as_str().into())),
        _ => Err("expected string".into()),
    }
}

fn expect_unit(v: &JsonValue) -> Result<Tag, String> {
    if v.is_object() {
        Ok(Tag::Compound(Compound::new()))
    } else {
        Err("expected JSON object".into())
    }
}

fn expect_opaque_nbt(v: &JsonValue) -> Result<Tag, String> {
    json_to_tag(v).map_err(|error| error.to_string())
}

fn expect_list(
    v: &JsonValue,
    mut elem: impl FnMut(&JsonValue) -> Result<Tag, String>,
) -> Result<Tag, String> {
    let Some(items) = v.as_array() else {
        return Err("expected JSON array".into());
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(elem(item)?);
    }
    Ok(Tag::List(out))
}

fn expect_map(
    v: &JsonValue,
    mut elem: impl FnMut(&JsonValue) -> Result<Tag, String>,
) -> Result<Tag, String> {
    let Some(entries) = v.as_object() else {
        return Err("expected JSON object".into());
    };
    let mut out = Compound::new();
    for (key, value) in entries {
        out.insert(JavaString::from(key.as_str()), elem(value)?);
    }
    Ok(Tag::Compound(out))
}

fn expect_record(
    v: &JsonValue,
    mut field: impl FnMut(&str, &JsonValue) -> Result<Option<Tag>, String>,
) -> Result<Tag, String> {
    let Some(entries) = v.as_object() else {
        return Err("expected JSON object".into());
    };
    let mut out = Compound::new();
    for (key, value) in entries {
        // JsonOps MapLike treats JSON null as an absent field for optionalFieldOf/fieldOf.
        if value.is_null() {
            continue;
        }
        if let Some(tag) = field(key, value)? {
            out.insert(JavaString::from(key.as_str()), tag);
        }
    }
    Ok(Tag::Compound(out))
}

fn expect_holder_set(v: &JsonValue) -> Result<Tag, String> {
    match v {
        JsonValue::String(_) => expect_string(v),
        JsonValue::Array(_) => expect_list(v, expect_string),
        _ => Err("expected registry holder set".into()),
    }
}

fn expect_sound(v: &JsonValue) -> Result<Tag, String> {
    if v.is_string() {
        return expect_string(v);
    }
    expect_record(v, |k, v| match k {
        "sound_id" => expect_string(v).map(Some),
        "range" => expect_number(v).map(Some),
        _ => Ok(None),
    })
}

fn expect_holder(
    v: &JsonValue,
    direct: impl FnOnce(&JsonValue) -> Result<Tag, String>,
) -> Result<Tag, String> {
    if v.is_string() {
        expect_string(v)
    } else {
        direct(v)
    }
}

fn expect_rgb(v: &JsonValue) -> Result<Tag, String> {
    if v.is_number() {
        expect_number(v)
    } else if v.is_array() {
        expect_list(v, expect_number)
    } else {
        Err("expected an RGB number or three-channel array".into())
    }
}

fn expect_resolvable_num(v: &JsonValue) -> Result<Tag, String> {
    if v.is_string() {
        expect_string(v)
    } else if v.is_number() {
        expect_number(v)
    } else {
        Err("expected identifier or number".into())
    }
}

fn expect_filterable(
    v: &JsonValue,
    mut inner: impl FnMut(&JsonValue) -> Result<Tag, String>,
) -> Result<Tag, String> {
    if let Some(obj) = v.as_object()
        && obj.contains_key("raw")
    {
        return expect_record(v, |k, v| match k {
            "raw" | "filtered" => inner(v).map(Some),
            _ => Ok(None),
        });
    }
    inner(v)
}

fn expect_effect_details(v: &JsonValue) -> Result<Tag, String> {
    with_tree_stack(|| {
        expect_record(v, |k, v| match k {
            "id" => expect_string(v).map(Some),
            "amplifier" | "duration" => expect_number(v).map(Some),
            "ambient" | "show_particles" | "show_icon" => expect_bool(v).map(Some),
            "hidden_effect" => expect_effect_details(v).map(Some),
            _ => Ok(None),
        })
    })
}

fn expect_consume_effects(v: &JsonValue) -> Result<Tag, String> {
    expect_list(v, |entry| {
        let Some(map) = entry.as_object() else {
            return Err("expected JSON object".into());
        };
        let kind = map
            .get("type")
            .and_then(JsonValue::as_str)
            .and_then(|s| source::identifier(s).ok());
        match kind.as_deref() {
            Some("minecraft:apply_effects") => expect_record(entry, |k, v| match k {
                "type" => expect_string(v).map(Some),
                "effects" => expect_list(v, expect_effect_details).map(Some),
                "probability" => expect_number(v).map(Some),
                _ => Ok(None),
            }),
            Some("minecraft:remove_effects") => expect_record(entry, |k, v| match k {
                "type" => expect_string(v).map(Some),
                "effects" => expect_holder_set(v).map(Some),
                _ => Ok(None),
            }),
            Some("minecraft:clear_all_effects") => expect_record(entry, |k, v| match k {
                "type" => expect_string(v).map(Some),
                _ => Ok(None),
            }),
            Some("minecraft:teleport_randomly") => expect_record(entry, |k, v| match k {
                "type" => expect_string(v).map(Some),
                "diameter" => expect_number(v).map(Some),
                "directional_particles" => expect_bool(v).map(Some),
                _ => Ok(None),
            }),
            Some("minecraft:play_sound") => expect_record(entry, |k, v| match k {
                "type" => expect_string(v).map(Some),
                "sound" => expect_sound(v).map(Some),
                _ => Ok(None),
            }),
            _ => expect_opaque_nbt(entry),
        }
    })
}

fn expect_explosion(v: &JsonValue) -> Result<Tag, String> {
    expect_record(v, |k, v| match k {
        "shape" => expect_string(v).map(Some),
        "colors" | "fade_colors" => expect_list(v, expect_number).map(Some),
        "has_trail" | "has_twinkle" => expect_bool(v).map(Some),
        _ => Ok(None),
    })
}

fn expect_kinetic_condition(v: &JsonValue) -> Result<Tag, String> {
    expect_record(v, |k, v| match k {
        "max_duration_ticks" | "min_speed" | "min_relative_speed" => expect_number(v).map(Some),
        _ => Ok(None),
    })
}

fn expect_profile(v: &JsonValue) -> Result<Tag, String> {
    if v.is_string() {
        return expect_string(v);
    }
    expect_record(v, |k, v| match k {
        "name" | "texture" | "cape" | "elytra" | "model" => expect_string(v).map(Some),
        "id" => expect_list(v, expect_number).map(Some),
        "properties" => {
            if v.is_object() {
                expect_map(v, |vals| expect_list(vals, expect_string)).map(Some)
            } else {
                expect_list(v, |entry| {
                    expect_record(entry, |k, v| match k {
                        "name" | "value" | "signature" => expect_string(v).map(Some),
                        _ => Ok(None),
                    })
                })
                .map(Some)
            }
        }
        _ => Ok(None),
    })
}

fn expect_text(v: &JsonValue, is_native: &dyn Fn(&str) -> bool) -> Result<Tag, String> {
    with_tree_stack(|| match v {
        JsonValue::String(_) => expect_string(v),
        JsonValue::Array(_) => expect_list(v, |item| expect_text(item, is_native)),
        JsonValue::Object(_) => expect_record(v, |k, v| match k {
            "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" | "interpret"
            | "plain" | "hat" => expect_bool(v).map(Some),
            "shadow_color" => {
                if v.is_number() {
                    expect_number(v).map(Some)
                } else if v.is_array() {
                    expect_list(v, expect_number).map(Some)
                } else {
                    Err("expected shadow color int or four-channel array".into())
                }
            }
            "extra" => expect_list(v, |item| expect_text(item, is_native)).map(Some),
            "with" => expect_list(v, |item| {
                if item.is_boolean() || item.is_number() {
                    expect_opaque_nbt(item)
                } else {
                    expect_text(item, is_native)
                }
            })
            .map(Some),
            "separator" => expect_text(v, is_native).map(Some),
            "fallback" => {
                if v.is_string() {
                    expect_string(v).map(Some)
                } else {
                    expect_text(v, is_native).map(Some)
                }
            }
            "score" => expect_record(v, |k, v| match k {
                "name" | "objective" => expect_string(v).map(Some),
                _ => Ok(None),
            })
            .map(Some),
            "player" => expect_profile(v).map(Some),
            "click_event" => expect_record(v, |k, v| match k {
                "action" | "url" | "command" | "value" | "id" => expect_string(v).map(Some),
                "page" => expect_number(v).map(Some),
                "payload" => expect_opaque_nbt(v).map(Some),
                "dialog" => {
                    if v.is_string() {
                        expect_string(v).map(Some)
                    } else {
                        expect_record(v, |k, v| match k {
                            "type" => expect_string(v).map(Some),
                            "title" | "external_title" => expect_text(v, is_native).map(Some),
                            _ => expect_opaque_nbt(v).map(Some),
                        })
                        .map(Some)
                    }
                }
                _ => Ok(None),
            })
            .map(Some),
            "hover_event" => expect_record(v, |k, v| match k {
                "action" | "id" => expect_string(v).map(Some),
                "value" | "name" => expect_text(v, is_native).map(Some),
                "uuid" => {
                    if v.is_string() {
                        expect_string(v).map(Some)
                    } else {
                        expect_list(v, expect_number).map(Some)
                    }
                }
                "count" => expect_number(v).map(Some),
                "components" => {
                    let Some(map) = v.as_object() else {
                        return Err("template components are not a map".into());
                    };
                    prepare_patch_json(map, is_native)
                        .map(|patch| Some(Tag::Compound(patch)))
                        .map_err(|error| error.to_string())
                }
                _ => Ok(None),
            })
            .map(Some),
            "type" | "text" | "translate" | "keybind" | "selector" | "nbt" | "source"
            | "entity" | "block" | "storage" | "object" | "sprite" | "atlas" | "color"
            | "insertion" | "font" => expect_string(v).map(Some),
            _ => Ok(None),
        }),
        _ => Err("expected text component".into()),
    })
}

fn expect_template(v: &JsonValue, is_native: &dyn Fn(&str) -> bool) -> Result<Tag, String> {
    with_tree_stack(|| {
        if v.is_string() {
            return expect_string(v);
        }
        expect_record(v, |k, v| match k {
            "id" => expect_string(v).map(Some),
            "count" => expect_number(v).map(Some),
            "components" => {
                let Some(map) = v.as_object() else {
                    return Err("template components are not a map".into());
                };
                prepare_patch_json(map, is_native)
                    .map(|patch| Some(Tag::Compound(patch)))
                    .map_err(|error| error.to_string())
            }
            _ => Ok(None),
        })
    })
}

fn expect_exact_components(v: &JsonValue, is_native: &dyn Fn(&str) -> bool) -> Result<Tag, String> {
    let Some(entries) = v.as_object() else {
        return Err("expected component map".into());
    };
    let mut out = Compound::new();
    for (raw_key, raw_value) in entries {
        let id = source::identifier(raw_key)?;
        if !is_persistent(&id) {
            return Err(format!("{id} has no persistent codec"));
        }
        let prepared = prepare_component_json(&id, raw_value, is_native)
            .map_err(|error| format!("{id}: {error}"))?;
        out.insert(JavaString::from(id.as_str()), prepared);
    }
    Ok(Tag::Compound(out))
}

fn expect_partial_predicates(
    v: &JsonValue,
    is_native: &dyn Fn(&str) -> bool,
) -> Result<Tag, String> {
    let Some(entries) = v.as_object() else {
        return Err("expected partial predicate map".into());
    };
    let mut out = Compound::new();
    for (raw_key, raw_value) in entries {
        let id = source::identifier(raw_key)?;
        let value = if id == "minecraft:custom_data" {
            expect_opaque_nbt(raw_value)?
        } else if id == "minecraft:creative_slot_lock" {
            expect_unit(raw_value)?
        } else {
            expect_predicate_value(raw_value, is_native)?
        };
        out.insert(JavaString::from(id.as_str()), value);
    }
    Ok(Tag::Compound(out))
}

fn expect_predicate_value(v: &JsonValue, is_native: &dyn Fn(&str) -> bool) -> Result<Tag, String> {
    const BOOLS: &[&str] = &[
        "has_twinkle",
        "has_trail",
        "resolved",
        "ambient",
        "visible",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "interpret",
        "plain",
    ];
    with_tree_stack(|| match v {
        JsonValue::Object(_) => expect_record(v, |k, v| {
            if k == "components" {
                expect_exact_components(v, is_native).map(Some)
            } else if BOOLS.contains(&k) {
                expect_bool(v).map(Some)
            } else {
                expect_predicate_value(v, is_native).map(Some)
            }
        }),
        JsonValue::Array(_) => expect_list(v, |item| expect_predicate_value(item, is_native)),
        _ => expect_opaque_nbt(v),
    })
}

fn expect_block_predicate(v: &JsonValue, is_native: &dyn Fn(&str) -> bool) -> Result<Tag, String> {
    with_tree_stack(|| {
        let Some(entries) = v.as_object() else {
            return Err("expected predicate map".into());
        };
        let mut out = Compound::new();
        for (key, value) in entries {
            if value.is_null() {
                continue;
            }
            let tag = match key.as_str() {
                "blocks" => expect_holder_set(value)?,
                "state" => expect_map(value, |v| {
                    if v.is_string() {
                        expect_string(v)
                    } else {
                        expect_record(v, |k, v| match k {
                            "min" | "max" => expect_string(v).map(Some),
                            _ => expect_opaque_nbt(v).map(Some),
                        })
                    }
                })?,
                "nbt" => expect_opaque_nbt(value)?,
                "components" => expect_exact_components(value, is_native)?,
                "predicates" => expect_partial_predicates(value, is_native)?,
                // Preserve unknown keys so `only_fields` in `source` rejects them.
                _ => expect_opaque_nbt(value)?,
            };
            out.insert(JavaString::from(key.as_str()), tag);
        }
        Ok(Tag::Compound(out))
    })
}

fn expect_item_predicate(v: &JsonValue, is_native: &dyn Fn(&str) -> bool) -> Result<Tag, String> {
    with_tree_stack(|| {
        expect_record(v, |k, v| match k {
            "items" => expect_holder_set(v).map(Some),
            "count" => {
                if v.is_number() {
                    expect_number(v).map(Some)
                } else {
                    expect_record(v, |k, v| match k {
                        "min" | "max" => expect_number(v).map(Some),
                        _ => Ok(None),
                    })
                    .map(Some)
                }
            }
            "components" => expect_exact_components(v, is_native).map(Some),
            "predicates" => expect_partial_predicates(v, is_native).map(Some),
            _ => Ok(None),
        })
    })
}

fn prepare_component_json(
    id: &str,
    v: &JsonValue,
    is_native: &dyn Fn(&str) -> bool,
) -> Result<Tag, String> {
    let Some(name) = id.strip_prefix("minecraft:") else {
        return expect_opaque_nbt(v);
    };
    with_tree_stack(|| match name {
        "custom_data" | "bucket_entity_data" | "entity_data" | "block_entity_data" => {
            expect_opaque_nbt(v)
        }
        "unbreakable" | "glider" | "waxed" | "intangible_projectile" => expect_unit(v),
        "enchantment_glint_override" => expect_bool(v),
        "max_stack_size"
        | "max_damage"
        | "damage"
        | "repair_cost"
        | "map_id"
        | "ominous_bottle_amplifier"
        | "minimum_attack_charge"
        | "potion_duration_scale" => expect_number(v),
        "dyed_color" => expect_rgb(v),
        "custom_name" | "item_name" => expect_text(v, is_native),
        "lore" => expect_list(v, |item| expect_text(item, is_native)),
        "rarity"
        | "base_color"
        | "dye"
        | "wolf/collar"
        | "tropical_fish/base_color"
        | "tropical_fish/pattern_color"
        | "cat/collar"
        | "sheep/color"
        | "shulker/color"
        | "cushion/color"
        | "fox/variant"
        | "salmon/size"
        | "parrot/variant"
        | "tropical_fish/pattern"
        | "mooshroom/variant"
        | "rabbit/variant"
        | "horse/variant"
        | "llama/variant"
        | "axolotl/variant"
        | "item_model"
        | "damage_type"
        | "tooltip_style"
        | "note_block_sound"
        | "block_transformer"
        | "villager/variant"
        | "wolf/variant"
        | "wolf/sound_variant"
        | "pig/variant"
        | "pig/sound_variant"
        | "cow/variant"
        | "cow/sound_variant"
        | "chicken/variant"
        | "chicken/sound_variant"
        | "zombie_nautilus/variant"
        | "frog/variant"
        | "cat/variant"
        | "cat/sound_variant"
        | "provides_pottery_pattern"
        | "painting/variant"
        | "jukebox_playable" => expect_string(v),
        "enchantments" | "stored_enchantments" => expect_map(v, expect_number),
        "debug_stick_state" | "block_state" => expect_map(v, expect_string),
        "recipes" => expect_list(v, expect_string),
        "provides_banner_patterns" => expect_holder_set(v),
        "break_sound" => expect_sound(v),
        "use_remainder" | "sulfur_cube_content" => expect_template(v, is_native),
        "charged_projectiles" | "bundle_contents" => {
            expect_list(v, |item| expect_template(item, is_native))
        }
        "pot_decorations" => expect_record(v, |k, v| match k {
            "back" | "left" | "right" | "front" => expect_template(v, is_native).map(Some),
            _ => Ok(None),
        }),
        "container" => expect_list(v, |entry| {
            expect_record(entry, |k, v| match k {
                "slot" => expect_number(v).map(Some),
                "item" => expect_template(v, is_native).map(Some),
                _ => Ok(None),
            })
        }),
        "can_place_on" | "can_break" => {
            if v.is_array() {
                expect_list(v, |pred| expect_block_predicate(pred, is_native))
            } else {
                expect_block_predicate(v, is_native)
            }
        }
        "lock" => expect_item_predicate(v, is_native),
        "map_decorations" => expect_map(v, |entry| {
            expect_record(entry, |k, v| match k {
                "type" => expect_string(v).map(Some),
                "x" | "z" | "rotation" => expect_number(v).map(Some),
                _ => Ok(None),
            })
        }),
        "container_loot" => expect_record(v, |k, v| match k {
            "loot_table" => expect_string(v).map(Some),
            "seed" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "use_effects" => expect_record(v, |k, v| match k {
            "can_sprint" | "interact_vibrations" => expect_bool(v).map(Some),
            "speed_multiplier" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "food" => expect_record(v, |k, v| match k {
            "nutrition" | "saturation" => expect_number(v).map(Some),
            "can_always_eat" => expect_bool(v).map(Some),
            _ => Ok(None),
        }),
        "consumable" => expect_record(v, |k, v| match k {
            "consume_seconds" => expect_number(v).map(Some),
            "animation" => expect_string(v).map(Some),
            "sound" => expect_sound(v).map(Some),
            "has_consume_particles" => expect_bool(v).map(Some),
            "on_consume_effects" => expect_consume_effects(v).map(Some),
            _ => Ok(None),
        }),
        "death_protection" => expect_record(v, |k, v| match k {
            "death_effects" => expect_consume_effects(v).map(Some),
            _ => Ok(None),
        }),
        "use_cooldown" => expect_record(v, |k, v| match k {
            "seconds" => expect_number(v).map(Some),
            "cooldown_group" => expect_string(v).map(Some),
            _ => Ok(None),
        }),
        "damage_resistant" => expect_record(v, |k, v| match k {
            "types" => expect_holder_set(v).map(Some),
            _ => Ok(None),
        }),
        "repairable" => expect_record(v, |k, v| match k {
            "items" => expect_holder_set(v).map(Some),
            _ => Ok(None),
        }),
        "enchantable" => expect_record(v, |k, v| match k {
            "value" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "villager_food" => expect_record(v, |k, v| match k {
            "nutrition" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "tool" => expect_record(v, |k, v| match k {
            "rules" => expect_list(v, |rule| {
                expect_record(rule, |k, v| match k {
                    "blocks" => expect_holder_set(v).map(Some),
                    "speed" => expect_number(v).map(Some),
                    "correct_for_drops" => expect_bool(v).map(Some),
                    _ => Ok(None),
                })
            })
            .map(Some),
            "default_mining_speed" | "damage_per_block" => expect_number(v).map(Some),
            "can_destroy_blocks_in_creative" => expect_bool(v).map(Some),
            _ => Ok(None),
        }),
        "weapon" => expect_record(v, |k, v| match k {
            "item_damage_per_attack" | "disable_blocking_for_seconds" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "attack_range" => expect_record(v, |k, v| match k {
            "min_reach" | "max_reach" | "min_creative_reach" | "max_creative_reach"
            | "hitbox_margin" | "mob_factor" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "equippable" => expect_record(v, |k, v| match k {
            "slot" | "asset_id" | "camera_overlay" => expect_string(v).map(Some),
            "equip_sound" | "shearing_sound" => expect_sound(v).map(Some),
            "allowed_entities" => expect_holder_set(v).map(Some),
            "dispensable" | "swappable" | "damage_on_hurt" | "equip_on_interact"
            | "can_be_sheared" => expect_bool(v).map(Some),
            _ => Ok(None),
        }),
        "piercing_weapon" => expect_record(v, |k, v| match k {
            "deals_knockback" | "dismounts" => expect_bool(v).map(Some),
            "sound" | "hit_sound" => expect_sound(v).map(Some),
            _ => Ok(None),
        }),
        "kinetic_weapon" => expect_record(v, |k, v| match k {
            "contact_cooldown_ticks" | "delay_ticks" | "forward_movement" | "damage_multiplier" => {
                expect_number(v).map(Some)
            }
            "dismount_conditions" | "knockback_conditions" | "damage_conditions" => {
                expect_kinetic_condition(v).map(Some)
            }
            "sound" | "hit_sound" => expect_sound(v).map(Some),
            _ => Ok(None),
        }),
        "attack_animation" | "interact_animation" => expect_record(v, |k, v| match k {
            "type" => expect_string(v).map(Some),
            "duration" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "compostable" => expect_record(v, |k, v| match k {
            "layers" => expect_resolvable_num(v).map(Some),
            _ => Ok(None),
        }),
        "cooking_fuel" => expect_record(v, |k, v| match k {
            "burn_time" | "speed_multiplier" => expect_resolvable_num(v).map(Some),
            _ => Ok(None),
        }),
        "brewing_fuel" => expect_record(v, |k, v| match k {
            "uses" | "speed_multiplier" => expect_resolvable_num(v).map(Some),
            _ => Ok(None),
        }),
        "mob_visibility" => expect_record(v, |k, v| match k {
            "targeting_entity_types" => expect_holder_set(v).map(Some),
            "visibility" => expect_number(v).map(Some),
            _ => Ok(None),
        }),
        "attribute_modifiers" => expect_list(v, |entry| {
            expect_record(entry, |k, v| match k {
                "type" | "id" | "operation" | "slot" => expect_string(v).map(Some),
                "amount" => expect_number(v).map(Some),
                "display" => expect_record(v, |k, v| match k {
                    "type" => expect_string(v).map(Some),
                    "value" => expect_text(v, is_native).map(Some),
                    _ => Ok(None),
                })
                .map(Some),
                _ => Ok(None),
            })
        }),
        "custom_model_data" => expect_record(v, |k, v| match k {
            "floats" => expect_list(v, expect_number).map(Some),
            "flags" => expect_list(v, expect_bool).map(Some),
            "strings" => expect_list(v, expect_string).map(Some),
            "colors" => expect_list(v, expect_rgb).map(Some),
            _ => Ok(None),
        }),
        "tooltip_display" => expect_record(v, |k, v| match k {
            "hide_tooltip" => expect_bool(v).map(Some),
            "hidden_components" => expect_list(v, expect_string).map(Some),
            _ => Ok(None),
        }),
        "potion_contents" => {
            if v.is_string() {
                expect_string(v)
            } else {
                expect_record(v, |k, v| match k {
                    "potion" | "custom_name" => expect_string(v).map(Some),
                    "custom_color" => expect_number(v).map(Some),
                    "custom_effects" => expect_list(v, expect_effect_details).map(Some),
                    _ => Ok(None),
                })
            }
        }
        "suspicious_stew_effects" => expect_list(v, |entry| {
            expect_record(entry, |k, v| match k {
                "id" => expect_string(v).map(Some),
                "duration" => expect_number(v).map(Some),
                _ => Ok(None),
            })
        }),
        "writable_book_content" => expect_record(v, |k, v| match k {
            "pages" => expect_list(v, |page| expect_filterable(page, expect_string)).map(Some),
            _ => Ok(None),
        }),
        "written_book_content" => expect_record(v, |k, v| match k {
            "title" => expect_filterable(v, expect_string).map(Some),
            "author" => expect_string(v).map(Some),
            "generation" => expect_number(v).map(Some),
            "pages" => expect_list(v, |page| {
                expect_filterable(page, |t| expect_text(t, is_native))
            })
            .map(Some),
            "resolved" => expect_bool(v).map(Some),
            _ => Ok(None),
        }),
        "trim" => expect_record(v, |k, v| match k {
            "material" => expect_holder(v, |m| {
                expect_record(m, |k, v| match k {
                    "palette_id" => expect_string(v).map(Some),
                    "description" => expect_text(v, is_native).map(Some),
                    _ => Ok(None),
                })
            })
            .map(Some),
            "pattern" => expect_holder(v, |p| {
                expect_record(p, |k, v| match k {
                    "asset_id" => expect_string(v).map(Some),
                    "description" => expect_text(v, is_native).map(Some),
                    "decal" => expect_bool(v).map(Some),
                    _ => Ok(None),
                })
            })
            .map(Some),
            _ => Ok(None),
        }),
        "instrument" => expect_holder(v, |entry| {
            expect_record(entry, |k, v| match k {
                "sound_event" => expect_sound(v).map(Some),
                "use_duration" | "range" | "durability_damage" => expect_number(v).map(Some),
                "description" => expect_text(v, is_native).map(Some),
                _ => Ok(None),
            })
        }),
        "provides_trim_material" => expect_holder(v, |m| {
            expect_record(m, |k, v| match k {
                "palette_id" => expect_string(v).map(Some),
                "description" => expect_text(v, is_native).map(Some),
                _ => Ok(None),
            })
        }),
        "lodestone_tracker" => expect_record(v, |k, v| match k {
            "target" => expect_record(v, |k, v| match k {
                "dimension" => expect_string(v).map(Some),
                "pos" => expect_list(v, expect_number).map(Some),
                _ => Ok(None),
            })
            .map(Some),
            "tracked" => expect_bool(v).map(Some),
            _ => Ok(None),
        }),
        "firework_explosion" => expect_explosion(v),
        "fireworks" => expect_record(v, |k, v| match k {
            "flight_duration" => expect_number(v).map(Some),
            "explosions" => expect_list(v, expect_explosion).map(Some),
            _ => Ok(None),
        }),
        "banner_patterns" => expect_list(v, |entry| {
            expect_record(entry, |k, v| match k {
                "pattern" => expect_holder(v, |p| {
                    expect_record(p, |k, v| match k {
                        "asset_id" | "translation_key" => expect_string(v).map(Some),
                        _ => Ok(None),
                    })
                })
                .map(Some),
                "color" => expect_string(v).map(Some),
                _ => Ok(None),
            })
        }),
        "bees" => expect_list(v, |entry| {
            expect_record(entry, |k, v| match k {
                "entity_data" => expect_opaque_nbt(v).map(Some),
                "ticks_in_hive" | "min_ticks_in_hive" => expect_number(v).map(Some),
                _ => Ok(None),
            })
        }),
        "sign_text_front" | "sign_text_back" => expect_record(v, |k, v| match k {
            "messages" | "filtered_messages" => {
                expect_list(v, |msg| expect_text(msg, is_native)).map(Some)
            }
            "color" => expect_string(v).map(Some),
            "has_glowing_text" => expect_bool(v).map(Some),
            _ => Ok(None),
        }),
        "profile" => expect_profile(v),
        "blocks_attacks" => expect_record(v, |k, v| match k {
            "block_delay_seconds" | "disable_cooldown_scale" => expect_number(v).map(Some),
            "damage_reductions" => expect_list(v, |entry| {
                expect_record(entry, |k, v| match k {
                    "horizontal_blocking_angle" | "base" | "factor" => expect_number(v).map(Some),
                    "type" => expect_holder_set(v).map(Some),
                    _ => Ok(None),
                })
            })
            .map(Some),
            "item_damage" => expect_record(v, |k, v| match k {
                "threshold" | "base" | "factor" => expect_number(v).map(Some),
                _ => Ok(None),
            })
            .map(Some),
            "bypassed_by" => expect_holder_set(v).map(Some),
            "block_sound" | "disabled_sound" => expect_sound(v).map(Some),
            _ => Ok(None),
        }),
        _ => Err(format!("unknown component codec {id}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_item_stack_json_roundtrips_plain_empty_and_typed_components() {
        assert_eq!(
            optional_item_stack_from_json_with(&json!({}), |_| false, &|_| true).unwrap(),
            None
        );

        let plain = json!({"id": "minecraft:stone", "count": 64});
        let decoded = optional_item_stack_from_json_with(&plain, |_| false, &|_| true)
            .unwrap()
            .unwrap();
        assert_eq!(decoded.to_item_stack_json().unwrap(), plain);

        // Absent count defaults to 1 on decode and is always written on encode (ItemStack.MAP_CODEC).
        let omitted_count = json!({"id": "stone"});
        let decoded = optional_item_stack_from_json_with(&omitted_count, |_| false, &|_| true)
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded.to_item_stack_json().unwrap(),
            json!({"id": "minecraft:stone", "count": 1})
        );

        // Typed components: text component with real JSON bool, int damage, float attack charge,
        // enchantments, custom_data, and explicit component removal.
        let expected = json!({
            "id": "minecraft:diamond_sword",
            "count": 1,
            "components": {
                "minecraft:custom_name": {"text": "Blade", "color": "gold", "bold": true},
                "minecraft:enchantments": {"minecraft:sharpness": 3},
                "minecraft:damage": 5,
                "minecraft:minimum_attack_charge": 0.5,
                "minecraft:enchantment_glint_override": true,
                "!minecraft:unbreakable": {}
            }
        });
        let decoded = optional_item_stack_from_json_with(&expected, |_| false, &|_| true)
            .unwrap()
            .unwrap();
        let encoded = decoded.to_item_stack_json().unwrap();
        assert_eq!(encoded, expected);
        // Float 1.0 stays a JSON float (1.0), while int 5 stays an integer (5).
        let int_vs_float = json!({
            "id": "minecraft:diamond_sword",
            "count": 1,
            "components": {
                "minecraft:damage": 1,
                "minecraft:minimum_attack_charge": 1
            }
        });
        let encoded = optional_item_stack_from_json_with(&int_vs_float, |_| false, &|_| true)
            .unwrap()
            .unwrap()
            .to_item_stack_json()
            .unwrap();
        assert_eq!(encoded["components"]["minecraft:damage"].to_string(), "1");
        assert_eq!(
            encoded["components"]["minecraft:minimum_attack_charge"].to_string(),
            "1.0"
        );
    }

    #[test]
    fn json_ops_enforces_strict_types_and_names_failing_fields() {
        let decode = |v: JsonValue| {
            optional_item_stack_from_json_with(&v, |_| false, &|id| id != "nomod:missing")
        };

        assert!(matches!(
            decode(json!({"count": 1})),
            Err(JsonError::Field { field, .. }) if field == "id"
        ));
        assert!(matches!(
            decode(json!({"id": "nomod:missing", "count": 1})),
            Err(JsonError::Field { field, .. }) if field == "id"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:stone", "count": true})),
            Err(JsonError::Field { field, .. }) if field == "count"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:stone", "count": 100})),
            Err(JsonError::Field { field, .. }) if field == "count"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:diamond_sword", "components": {"minecraft:damage": true}})),
            Err(JsonError::Field { field, .. }) if field == "minecraft:damage"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:diamond_sword", "components": {"minecraft:damage": -1}})),
            Err(JsonError::Field { field, .. }) if field == "minecraft:damage"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:stone", "components": {"minecraft:enchantment_glint_override": 1}})),
            Err(JsonError::Field { field, .. }) if field == "minecraft:enchantment_glint_override"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:stone", "components": {"minecraft:custom_name": {"text": "Hi", "bold": 1}}})),
            Err(JsonError::Field { field, .. }) if field == "minecraft:custom_name"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:stone", "components": {"minecraft:unbreakable": 1}})),
            Err(JsonError::Field { field, .. }) if field == "minecraft:unbreakable"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:diamond_sword", "components": {"!minecraft:damage": 1}})),
            Err(JsonError::Field { field, .. }) if field == "!minecraft:damage"
        ));
        assert!(matches!(
            decode(json!({"id": "minecraft:stone", "components": {"minecraft:damage": null}})),
            Err(JsonError::Field { field, .. }) if field == "minecraft:damage"
        ));
        // Optional record fields treat JSON null as absent.
        let with_null_optional = json!({
            "id": "minecraft:stone",
            "count": null,
            "components": {
                "minecraft:custom_name": {"text": "Hi", "color": null}
            }
        });
        let encoded = decode(with_null_optional)
            .unwrap()
            .unwrap()
            .to_item_stack_json()
            .unwrap();
        assert_eq!(
            encoded,
            json!({
                "id": "minecraft:stone",
                "count": 1,
                "components": {"minecraft:custom_name": "Hi"}
            })
        );
    }
}
