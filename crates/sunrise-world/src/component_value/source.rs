//! Persistent component codecs, without protocol state or registry-number assumptions.
use super::{
    Patch, Template, Value,
    consume_effects::{self, Admitted},
};
use sunrise_core::nbt::Tag;

type Result<T, E = String> = std::result::Result<T, E>;
/// `Identifier.CODEC`: default the namespace, never substitute a registry entry.
pub fn identifier(s: &str) -> Result<String> {
    normalized_identifier(s).map(std::borrow::Cow::into_owned)
}

/// The saved component codec's identifier admission without cloning qualified names.
#[doc(hidden)]
pub fn normalized_identifier(s: &str) -> Result<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    let (namespace, path) = s.split_once(':').unwrap_or(("", s));
    if path.is_empty()
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
        || !path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/_.-".contains(&c))
    {
        return Err(format!("invalid identifier {s}"));
    }
    Ok(if namespace.is_empty() {
        Cow::Owned(format!("minecraft:{path}"))
    } else {
        Cow::Borrowed(s)
    })
}

fn string(t: &Tag) -> Result<String> {
    if let Tag::String(s) = t {
        Ok(s.to_string_lossy())
    } else {
        Err("expected string".into())
    }
}
fn ident(t: &Tag) -> Result<Value> {
    Ok(Value::string(&identifier(&string(t)?)?))
}
fn int(t: &Tag) -> Result<Value> {
    integer(t).map(Value::Int)
}
fn integer(t: &Tag) -> Result<i32> {
    Ok(match t {
        Tag::Long(v) => *v as i32,
        _ => number(t)? as i32,
    })
}
fn number(t: &Tag) -> Result<f64> {
    Ok(match t {
        Tag::Byte(v) => *v as f64,
        Tag::Short(v) => *v as f64,
        Tag::Int(v) => *v as f64,
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => f32::from_bits(*v) as f64,
        Tag::Double(v) => f64::from_bits(*v),
        _ => return Err("expected number".into()),
    })
}
fn float(t: &Tag) -> Result<Value> {
    Ok(if let Tag::Float(v) = t {
        Value::Float(*v)
    } else {
        Value::Float((number(t)? as f32).to_bits())
    })
}
fn double(t: &Tag) -> Result<Value> {
    Ok(if let Tag::Double(v) = t {
        Value::Double(*v)
    } else {
        Value::Double(number(t)?.to_bits())
    })
}
fn boolean(t: &Tag) -> Result<Value> {
    Ok(Value::Bool(number(t)? != 0.0))
}
/// `ComponentSerialization.CODEC`: normalize the decoded contents and style before
/// projection or hashing. A component list appends its tail to the first component.
fn text(t: &Tag, source: Source) -> Result<Value> {
    if let Tag::String(s) = t {
        return Ok(Value::String(s.0.clone()));
    }
    let mut fields = text_fields(t, source)?;
    if fields.len() == 1 && fields[0].0 == Value::string("text") {
        return Ok(fields.pop().unwrap().1);
    }
    Ok(Value::Map(fields))
}

fn text_fields(t: &Tag, source: Source) -> Result<Vec<(Value, Value)>> {
    super::with_tree_stack(|| {
        match t {
            Tag::String(s) => return Ok(vec![(Value::string("text"), Value::String(s.0.clone()))]),
            Tag::List(items) => {
                let (first, siblings) = items.split_first().ok_or("empty text component list")?;
                let mut fields = text_fields(first, source)?;
                if !siblings.is_empty() {
                    let extra = Value::string("extra");
                    let siblings = siblings.iter().map(|item| text(item, source));
                    if let Some((_, Value::List(existing))) =
                        fields.iter_mut().find(|(k, _)| *k == extra)
                    {
                        for sibling in siblings {
                            existing.push(sibling?);
                        }
                    } else {
                        fields.push((extra, Value::List(siblings.collect::<Result<_>>()?)));
                    }
                }
                return Ok(fields);
            }
            Tag::Compound(_) => (),
            _ => return Err("expected text component".into()),
        }

        let mut r = Record {
            tag: t,
            fields: vec![],
        };
        if let Some(kind) = t.get("type") {
            text_contents(&mut r, &string(kind)?, source)?;
        } else {
            let mut result = Err("no matching text contents".into());
            for kind in [
                "text",
                "translatable",
                "keybind",
                "score",
                "selector",
                "nbt",
                "object",
            ] {
                let present = match kind {
                    "translatable" => t.get("translate").is_some(),
                    "object" => ["object", "sprite", "player"]
                        .into_iter()
                        .any(|k| t.get(k).is_some()),
                    _ => t.get(kind).is_some(),
                };
                if present {
                    result = text_contents(&mut r, kind, source);
                    if result.is_ok() {
                        break;
                    }
                    r.fields.clear();
                }
            }
            result?;
        }

        r.optional("extra", |v| {
            let values = list(v, |v| text(v, source))?;
            if matches!(&values, Value::List(items) if items.is_empty()) {
                return Err("empty extra text list".into());
            }
            Ok(values)
        })?;
        text_style(&mut r, source)?;
        Ok(r.fields)
    })
}

fn text_contents(r: &mut Record<'_>, kind: &str, source: Source) -> Result<()> {
    let t = r.tag;
    match kind {
        "text" => r.required("text", string_value),
        "translatable" => {
            r.required("translate", string_value)?;
            if matches!(t.get("fallback"), Some(Tag::String(_))) {
                r.required("fallback", string_value)?;
            }
            r.list("with", |v| {
                if number(v).is_ok() {
                    Ok(Value::from_nbt(v, &[]))
                } else {
                    text(v, source)
                }
            })
        }
        "keybind" => r.required("keybind", string_value),
        "score" => r.required("score", |v| {
            record(v, |r| {
                r.required("name", string_value)?;
                r.required("objective", string_value)
            })
        }),
        "selector" => {
            r.required("selector", string_value)?;
            r.optional("separator", |v| text(v, source))
        }
        "nbt" => {
            r.required("nbt", string_value)?;
            let interpret = matches!(t.get("interpret").map(boolean), Some(Ok(Value::Bool(true))));
            let plain = matches!(t.get("plain").map(boolean), Some(Ok(Value::Bool(true))));
            if interpret && plain {
                return Err("interpret and plain cannot both be enabled".into());
            }
            if interpret {
                r.fields
                    .push((Value::string("interpret"), Value::Bool(true)));
            }
            if plain {
                r.fields.push((Value::string("plain"), Value::Bool(true)));
            }
            let data = if let Some(data) = t.get("source") {
                string(data)?
            } else if matches!(t.get("entity"), Some(Tag::String(_))) {
                "entity".to_owned()
            } else if matches!(t.get("block"), Some(Tag::String(_))) {
                "block".to_owned()
            } else {
                "storage".to_owned()
            };
            match data.as_str() {
                "entity" => r.required("entity", string_value)?,
                "block" => r.required("block", string_value)?,
                "storage" => r.required("storage", text_identifier)?,
                _ => return Err("unknown text NBT source".into()),
            }
            r.optional("separator", |v| text(v, source))
        }
        "object" => {
            let atlas = |r: &mut Record<'_>| {
                r.required("sprite", text_identifier)?;
                r.id("atlas", "minecraft:blocks", text_identifier)
            };
            let player = |r: &mut Record<'_>| {
                r.required("player", profile)?;
                r.b("hat", true)
            };
            match t.get("object").map(string).transpose()?.as_deref() {
                Some("atlas") => atlas(r)?,
                Some("player") => player(r)?,
                Some(_) => return Err("unknown text object contents".into()),
                None => {
                    if atlas(r).is_err() {
                        r.fields.clear();
                        player(r)?;
                    }
                }
            }
            r.optional("fallback", |v| text(v, source))
        }
        _ => Err(format!("unknown text contents {kind}")),
    }
}

/// `Style.Serializer.MAP_CODEC`: known fields are strict optional fields.
fn text_style(r: &mut Record<'_>, source: Source) -> Result<()> {
    r.optional("color", text_color)?;
    r.optional("shadow_color", |t| {
        if let Ok(color) = int(t) {
            return Ok(color);
        }
        let value = list(t, float)?;
        let Value::List(values) = &value else {
            unreachable!()
        };
        let [
            Value::Float(red),
            Value::Float(green),
            Value::Float(blue),
            Value::Float(alpha),
        ] = values.as_slice()
        else {
            return Err("expected four shadow color channels".into());
        };
        let color = [*alpha, *red, *green, *blue].map(|bits| {
            let x = f32::from_bits(bits) * 255.0;
            let n = x as i32;
            (if x < n as f32 { n.wrapping_sub(1) } else { n }) as u8
        });
        Ok(Value::Int(i32::from_be_bytes(color)))
    })?;
    for key in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        r.optional(key, boolean)?;
    }
    r.optional("insertion", string_value)?;
    r.optional("font", text_identifier)?;
    r.optional("click_event", |v| click_event(v, source))?;
    r.optional("hover_event", |v| hover_event(v, source))
}

fn text_identifier(t: &Tag) -> Result<Value> {
    let id = sunrise_core::identifier::Identifier::parse(&string(t)?).map_err(|e| e.to_string())?;
    Ok(Value::string(&id.to_string()))
}

/// `TextColor.parseColor`: named colors or a signed Java base-16 integer in RGB range.
fn text_color(t: &Tag) -> Result<Value> {
    let name = string(t)?;
    if let Some(hex) = name.strip_prefix('#') {
        let negative = hex.starts_with('-');
        let hex = hex.strip_prefix(['+', '-']).unwrap_or(hex);
        let bad = || format!("invalid text color {name}");
        if hex.is_empty() {
            return Err(bad());
        }
        let mut value = 0;
        for unit in hex.encode_utf16() {
            let digit = sunrise_core::java_character::digit(unit, 16).ok_or_else(bad)?;
            value = (value << 4) | digit;
            if value > 0xff_ffff {
                return Err(bad());
            }
        }
        if negative && value != 0 {
            return Err(bad());
        }
        return Ok(Value::string(&format!("#{value:06X}")));
    }
    match name.as_str() {
        "black" | "dark_blue" | "dark_green" | "dark_aqua" | "dark_red" | "dark_purple"
        | "gold" | "gray" | "dark_gray" | "blue" | "green" | "aqua" | "red" | "light_purple"
        | "yellow" | "white" => Ok(Value::string(&name)),
        _ => Err(format!("invalid text color {name}")),
    }
}

fn click_event(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        let action = string(t.get("action").ok_or("missing click action")?)?;
        r.fields
            .push((Value::string("action"), Value::string(&action)));
        match action.as_str() {
            "open_url" => r.required("url", |v| {
                if !sunrise_core::text::untrusted_http_url(&string(v)?) {
                    return Err("invalid click URL".into());
                }
                string_value(v)
            }),
            "run_command" | "suggest_command" => r.required("command", |v| {
                let Tag::String(s) = v else {
                    return Err("expected command string".into());
                };
                if s.0
                    .iter()
                    .any(|&unit| unit < 32 || unit == 127 || unit == 167)
                {
                    return Err("disallowed click command character".into());
                }
                Ok(Value::String(s.0.clone()))
            }),
            "change_page" => r.required("page", |v| {
                let page = int(v)?;
                if matches!(page, Value::Int(n) if n > 0) {
                    Ok(page)
                } else {
                    Err("click page must be positive".into())
                }
            }),
            "copy_to_clipboard" => r.required("value", string_value),
            "custom" => {
                r.required("id", text_identifier)?;
                r.optional("payload", |v| Ok(Value::from_nbt(v, &[])))
            }
            "show_dialog" => r.required("dialog", |v| {
                if let Tag::String(_) = v {
                    return text_identifier(v);
                }
                let Tag::Compound(fields) = v else {
                    return Err("expected dialog holder or inline dialog".into());
                };
                let kind = text_identifier(v.get("type").ok_or("missing dialog type")?)?;
                if ![
                    "minecraft:notice",
                    "minecraft:server_links",
                    "minecraft:dialog_list",
                    "minecraft:multi_action",
                    "minecraft:confirmation",
                ]
                .into_iter()
                .any(|name| kind == Value::string(name))
                {
                    return Err("unknown inline dialog type".into());
                }
                // The dialog's body and actions have their own traversal. Do not
                // interpret opaque custom payloads there as text or item components.
                crate::item_defaults::validate_dialog_tag(v)?;
                let mut normalized = vec![
                    (Value::string("type"), kind),
                    (
                        Value::string("title"),
                        text(v.get("title").ok_or("missing dialog title")?, source)?,
                    ),
                ];
                for (key, value) in fields {
                    match key.to_string_lossy().as_str() {
                        "type" | "title" => (),
                        "external_title" => {
                            normalized.push((Value::string("external_title"), text(value, source)?))
                        }
                        _ => normalized
                            .push((Value::String(key.0.clone()), Value::from_nbt(value, &[]))),
                    }
                }
                Ok(Value::Map(normalized))
            }),
            _ => Err(format!("invalid click action {action}")),
        }
    })
}

fn hover_event(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        let action = string(t.get("action").ok_or("missing hover action")?)?;
        r.fields
            .push((Value::string("action"), Value::string(&action)));
        match action.as_str() {
            "show_text" => r.required("value", |v| text(v, source)),
            "show_entity" => {
                r.required("id", text_identifier)?;
                r.required("uuid", |v| {
                    if let Tag::String(s) = v {
                        let uuid = sunrise_core::text::parse_java_uuid(&s.to_string_lossy())
                            .ok_or("invalid hover entity UUID")?;
                        return Ok(Value::IntArray(
                            [96, 64, 32, 0].map(|shift| (uuid >> shift) as i32).to_vec(),
                        ));
                    }
                    let parts = integer_list(v)?;
                    if parts.len() != 4 {
                        return Err("hover entity UUID must have four integers".into());
                    }
                    Ok(Value::IntArray(parts))
                })?;
                r.optional("name", |v| text(v, source))
            }
            "show_item" => {
                // ShowItem flattens ItemStackTemplate.MAP_CODEC into the event map.
                // Text is codec NBT, even when its outer component came from datafixing.
                let mut item = template_tag(
                    t,
                    Source {
                        upgraded: false,
                        ..source
                    },
                )?;
                r.fields
                    .push((Value::string("id"), Value::string(&item.item)));
                if item.count != 1 {
                    r.fields
                        .push((Value::string("count"), Value::Int(item.count)));
                }
                if !item.patch.is_empty() {
                    let patch = std::mem::take(&mut item.patch)
                        .into_iter()
                        .map(|(id, value)| match value {
                            Some(value) => (Value::string(&id), value),
                            None => (Value::string(&format!("!{id}")), Value::Map(vec![])),
                        })
                        .collect();
                    r.fields
                        .push((Value::string("components"), Value::Map(patch)));
                }
                Ok(())
            }
            _ => Err(format!("invalid hover action {action}")),
        }
    })
}

fn list(t: &Tag, f: impl Fn(&Tag) -> Result<Value>) -> Result<Value> {
    let values = match t {
        Tag::List(v) => v.iter().map(f).collect::<Result<_>>()?,
        Tag::ByteArray(v) => v
            .iter()
            .map(|v| f(&Tag::Byte(*v as i8)))
            .collect::<Result<_>>()?,
        Tag::IntArray(v) => v.iter().map(|v| f(&Tag::Int(*v))).collect::<Result<_>>()?,
        Tag::LongArray(v) => v.iter().map(|v| f(&Tag::Long(*v))).collect::<Result<_>>()?,
        _ => return Err("expected list".into()),
    };
    Ok(Value::List(values))
}
fn integer_list(t: &Tag) -> Result<Vec<i32>> {
    match t {
        Tag::IntArray(v) => Ok(v.clone()),
        Tag::List(v) => v.iter().map(integer).collect(),
        _ => Err("expected list".into()),
    }
}

fn map(t: &Tag, f: impl Fn(&Tag) -> Result<Value>) -> Result<Value> {
    let Tag::Compound(v) = t else {
        return Err("expected map".into());
    };
    Ok(Value::Map(
        v.iter()
            .map(|(k, v)| Ok((Value::String(k.0.clone()), f(v)?)))
            .collect::<Result<_>>()?,
    ))
}
fn holder_set(t: &Tag) -> Result<Value> {
    match t {
        Tag::String(_) => {
            let s = string(t)?;
            Ok(Value::string(&if let Some(s) = s.strip_prefix('#') {
                format!("#{}", identifier(s)?)
            } else {
                identifier(&s)?
            }))
        }
        Tag::List(v) if v.len() == 1 => ident(&v[0]),
        Tag::List(_) => list(t, ident),
        _ => Err("expected registry holder set".into()),
    }
}
fn sound(t: &Tag) -> Result<Value> {
    if matches!(t, Tag::String(_)) {
        return ident(t);
    }
    record(t, |r| {
        r.required("sound_id", ident)?;
        r.optional("range", float)?;
        Ok(())
    })
}
fn holder(t: &Tag, direct: impl Fn(&Tag) -> Result<Value>) -> Result<Value> {
    if matches!(t, Tag::String(_)) {
        ident(t)
    } else {
        direct(t)
    }
}
struct Record<'a> {
    tag: &'a Tag,
    fields: Vec<(Value, Value)>,
}
impl Record<'_> {
    fn required(&mut self, name: &str, f: impl Fn(&Tag) -> Result<Value>) -> Result<()> {
        let v = self
            .tag
            .get(name)
            .ok_or_else(|| format!("missing {name}"))?;
        self.fields.push((Value::string(name), f(v)?));
        Ok(())
    }
    fn optional(&mut self, name: &str, f: impl Fn(&Tag) -> Result<Value>) -> Result<()> {
        if let Some(v) = self.tag.get(name) {
            self.fields.push((Value::string(name), f(v)?));
        }
        Ok(())
    }
    fn default(
        &mut self,
        name: &str,
        default: Value,
        f: impl Fn(&Tag) -> Result<Value>,
    ) -> Result<()> {
        if let Some(v) = self.tag.get(name) {
            let v = f(v)?;
            if v != default {
                self.fields.push((Value::string(name), v));
            }
        }
        Ok(())
    }
    fn i(&mut self, name: &str, default: i32) -> Result<()> {
        self.default(name, Value::Int(default), int)
    }
    fn f(&mut self, name: &str, default: f32) -> Result<()> {
        self.default(name, Value::Float(default.to_bits()), float)
    }
    fn b(&mut self, name: &str, default: bool) -> Result<()> {
        self.default(name, Value::Bool(default), boolean)
    }
    fn id(&mut self, name: &str, default: &str, f: impl Fn(&Tag) -> Result<Value>) -> Result<()> {
        self.default(name, Value::string(default), f)
    }
    fn list(&mut self, name: &str, f: impl Fn(&Tag) -> Result<Value>) -> Result<()> {
        self.default(name, Value::List(vec![]), |v| list(v, &f))
    }
}
fn record(t: &Tag, f: impl FnOnce(&mut Record<'_>) -> Result<()>) -> Result<Value> {
    if !matches!(t, Tag::Compound(_)) {
        return Err("expected record".into());
    }
    let mut r = Record {
        tag: t,
        fields: vec![],
    };
    f(&mut r)?;
    Ok(Value::Map(r.fields))
}
fn filterable(t: &Tag, f: impl Fn(&Tag) -> Result<Value>) -> Result<Value> {
    if t.get("raw").is_some() {
        record(t, |r| {
            r.required("raw", &f)?;
            r.optional("filtered", &f)
        })
    } else {
        Ok(Value::Map(vec![(Value::string("raw"), f(t)?)]))
    }
}
fn string_value(t: &Tag) -> Result<Value> {
    let Tag::String(s) = t else {
        return Err("expected string".into());
    };
    Ok(Value::String(s.0.clone()))
}
fn unit(_: &Tag) -> Result<Value> {
    Ok(Value::Map(vec![]))
}
fn effect_details(t: &Tag, with_id: bool) -> Result<Value> {
    super::with_tree_stack(|| {
        record(t, |r| {
            if with_id {
                r.required("id", ident)?;
            }
            r.default("amplifier", Value::Byte(0), |v| {
                let Value::Int(v) = int(v)? else {
                    unreachable!()
                };
                Ok(Value::Byte(v as i8))
            })?;
            r.i("duration", 0)?;
            r.b("ambient", false)?;
            r.b("show_particles", true)?;
            let particles = t
                .get("show_particles")
                .map(boolean)
                .transpose()?
                .unwrap_or(Value::Bool(true));
            let icon = t
                .get("show_icon")
                .map(boolean)
                .transpose()?
                .unwrap_or(particles);
            r.fields.push((Value::string("show_icon"), icon));
            r.optional("hidden_effect", |v| effect_details(v, false))
        })
    })
}
type AdmittedLookup<'a> = dyn Fn(&Tag) -> Option<Admitted> + 'a;
type ProjectedEffect<'a> = dyn Fn(&Admitted) + 'a;

#[derive(Clone, Copy)]
struct EffectProjection<'a> {
    retained: &'a AdmittedLookup<'a>,
    projected: &'a ProjectedEffect<'a>,
}

fn consume_effect(t: &Tag, source: Source<'_>) -> Result<Option<Value>> {
    let kind = identifier(&string(
        t.get("type").ok_or("missing consume effect type")?,
    )?)?;
    validate_source(kind.strip_prefix("minecraft:").unwrap_or(&kind), t)?;
    let Tag::Compound(fields) = t else {
        return Err("consume effect is not a map".into());
    };
    let registered = if let Some(projection) = source.effects {
        match (projection.retained)(t) {
            None if source.deferred_template => {
                // The registered codec reports any refusal. Keep the siblings of a
                // refused effect when projecting a live template to the network.
                let Ok(effect) = consume_effects::admit(&kind, fields) else {
                    return Ok(None);
                };
                if let Some(effect) = &effect {
                    (projection.projected)(effect);
                }
                effect
            }
            retained => retained,
        }
    } else {
        consume_effects::admit(&kind, fields)?
    };
    if let Some(effect) = registered {
        return Ok(Some(Value::RegisteredConsumeEffect(effect)));
    }
    record(t, |r| {
        r.fields.push((Value::string("type"), Value::string(&kind)));
        match kind.as_str() {
            "minecraft:apply_effects" => {
                r.required("effects", |v| list(v, |v| effect_details(v, true)))?;
                r.f("probability", 1.0)?;
            }
            "minecraft:remove_effects" => r.required("effects", holder_set)?,
            "minecraft:clear_all_effects" => (),
            "minecraft:teleport_randomly" => {
                r.f("diameter", 16.0)?;
                r.b("directional_particles", true)?;
            }
            "minecraft:play_sound" => r.required("sound", sound)?,
            _ if kind.starts_with("minecraft:") => {
                return Err(format!("unknown consume effect {kind}"));
            }
            _ => {
                let type_key = "type".into();
                r.fields
                    .extend(fields.iter().filter(|(key, _)| *key != &type_key).map(
                        |(key, value)| (Value::String(key.0.clone()), Value::from_nbt(value, &[])),
                    ));
            }
        }
        Ok(())
    })
    .map(Some)
}
fn explosion(t: &Tag) -> Result<Value> {
    validate_source("firework_explosion", t)?;
    let colors = |v: &Tag| {
        Ok(Value::List(
            integer_list(v)?.into_iter().map(Value::Int).collect(),
        ))
    };
    record(t, |r| {
        r.required("shape", string_value)?;
        r.default("colors", Value::List(vec![]), colors)?;
        r.default("fade_colors", Value::List(vec![]), colors)?;
        r.b("has_trail", false)?;
        r.b("has_twinkle", false)
    })
}
fn trim_material(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        r.required("palette_id", ident)?;
        r.required("description", |v| text(v, source))
    })
}
fn trim_pattern(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        r.required("asset_id", ident)?;
        r.required("description", |v| text(v, source))?;
        if t.get("decal").is_some() {
            r.required("decal", boolean)
        } else {
            r.fields.push((Value::string("decal"), Value::Bool(false)));
            Ok(())
        }
    })
}
fn instrument(t: &Tag, source: Source) -> Result<Value> {
    validate_source("instrument", t)?;
    record(t, |r| {
        r.required("sound_event", sound)?;
        r.required("use_duration", float)?;
        r.required("range", float)?;
        r.i("durability_damage", 0)?;
        r.required("description", |v| text(v, source))
    })
}
fn banner(t: &Tag) -> Result<Value> {
    record(t, |r| {
        r.required("asset_id", ident)?;
        r.required("translation_key", string_value)
    })
}
fn kinetic(t: &Tag) -> Result<Value> {
    record(t, |r| {
        r.required("max_duration_ticks", int)?;
        r.f("min_speed", 0.0)?;
        r.f("min_relative_speed", 0.0)
    })
}
fn resolvable(t: &Tag, f: impl Fn(&Tag) -> Result<Value>) -> Result<Value> {
    if matches!(t, Tag::String(_)) {
        ident(t)
    } else {
        f(t)
    }
}

#[derive(Clone, Copy)]
struct Source<'a> {
    upgraded: bool,
    is_native: fn(&str) -> bool,
    effects: Option<EffectProjection<'a>>,
    deferred_template: bool,
}
impl Source<'_> {
    const SAVED: Self = Self {
        upgraded: false,
        is_native: |_| false,
        effects: None,
        deferred_template: false,
    };

    fn component_type(self, id: &str) -> Result<()> {
        if super::types::builtin_component_id(id).is_some() || (self.is_native)(id) {
            Ok(())
        } else {
            Err(format!("unknown component type {id}"))
        }
    }
}

#[doc(hidden)]
pub fn validate_template_count(count: i32) -> Result<()> {
    if !(1..=99).contains(&count) {
        return Err("template count outside 1..99".into());
    }
    Ok(())
}

/// `ItemStackTemplate.CODEC`: source admission, not the signed native stream codec.
pub fn template_from_tag(t: &Tag) -> Result<Template> {
    template_tag(t, Source::SAVED)
}
fn template_tag(t: &Tag, source: Source) -> Result<Template> {
    if matches!(t, Tag::String(_)) {
        let item = identifier(&string(t)?)?;
        if item == "minecraft:air" {
            return Err("air is not a template item".into());
        }
        return Ok(Template {
            item,
            count: 1,
            patch: vec![],
        });
    }
    let item = identifier(&string(t.get("id").ok_or("missing template item")?)?)?;
    if item == "minecraft:air" {
        return Err("air is not a template item".into());
    }
    let count = t
        .get("count")
        .map(int)
        .transpose()?
        .unwrap_or(Value::Int(1));
    let Value::Int(count) = count else {
        unreachable!()
    };
    if source.upgraded {
        if count == 0 {
            return Err("native template count must be nonzero".into());
        }
    } else {
        validate_template_count(count)?;
    }
    let mut patch: Patch = vec![];
    if let Some(c) = t.get("components") {
        let Tag::Compound(c) = c else {
            return Err("template components are not a map".into());
        };
        for (name, value) in c {
            let name = name.to_string_lossy();
            let (remove, name) = name
                .strip_prefix('!')
                .map_or((false, name.as_str()), |n| (true, n));
            let name = identifier(name)?;
            let value = if remove {
                source.component_type(&name)?;
                if !source.upgraded && !super::is_persistent(&name) {
                    return Err(format!("{name} has no persistent codec"));
                }
                if !matches!(value, Tag::Compound(_)) {
                    return Err(format!("{name}: removal payload must be a map"));
                }
                None
            } else {
                Some(component_tag(&name, value, source)?)
            };
            patch.push((name, value));
        }
        super::normalize_map(&mut patch, |(name, _)| name);
    }
    Ok(Template { item, count, patch })
}
fn template(t: &Tag, source: Source) -> Result<Value> {
    Ok(Value::Template(Box::new(template_tag(t, source)?)))
}
fn predicate(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        only_fields(t, &["blocks", "state", "nbt", "components", "predicates"])?;
        r.optional("blocks", holder_set)?;
        r.optional("state", |v| {
            map(v, |v| {
                if matches!(v, Tag::String(_)) {
                    string_value(v)
                } else {
                    record(v, |r| {
                        only_fields(v, &["min", "max"])?;
                        r.optional("min", string_value)?;
                        r.optional("max", string_value)
                    })
                }
            })
        })?;
        r.optional("nbt", |v| {
            if let Tag::String(_) = v {
                let s = string(v)?;
                let (tag, n) = sunrise_core::nbt::parse_snbt(&s)?;
                if n != s.len() {
                    return Err("trailing predicate SNBT".into());
                }
                Ok(Value::string(&crate::snbt_text::to_snbt(&tag)))
            } else {
                Ok(Value::string(&crate::snbt_text::to_snbt(v)))
            }
        })?;
        r.default("components", Value::Map(vec![]), |v| {
            exact_components(v, source)
        })?;
        r.default("predicates", Value::Map(vec![]), |v| {
            partial_predicates(v, source)
        })
    })
}

/// Decode the built-in component codec and remove its Java defaults.
pub fn from_component_tag(id: &str, t: &Tag) -> Result<Value> {
    from_component_tag_with(id, t, |_| false)
}
/// Project a live component, reusing its admitted effects and reporting any effects
/// newly materialized from deferred templates to the caller.
pub fn from_admitted_component_tag(
    id: &str,
    t: &Tag,
    is_native: fn(&str) -> bool,
    admitted: &AdmittedLookup<'_>,
    projected: &ProjectedEffect<'_>,
) -> Result<Value> {
    component_tag(
        &identifier(id)?,
        t,
        Source {
            is_native,
            effects: Some(EffectProjection {
                retained: admitted,
                projected,
            }),
            ..Source::SAVED
        },
    )
}

fn effect_component(t: &Tag, consumable: bool, source: Source<'_>) -> Result<Value> {
    record(t, |r| {
        let key = if consumable {
            r.f("consume_seconds", 1.6)?;
            r.id("animation", "eat", string_value)?;
            r.id("sound", "minecraft:entity.generic.eat", sound)?;
            r.b("has_consume_particles", true)?;
            "on_consume_effects"
        } else {
            "death_effects"
        };
        r.default(key, Value::List(vec![]), |t| {
            let effects = match t {
                Tag::List(effects) => effects
                    .iter()
                    .filter_map(|effect| consume_effect(effect, source).transpose())
                    .collect::<Result<_>>()?,
                Tag::ByteArray(a) if a.is_empty() => vec![],
                Tag::IntArray(a) if a.is_empty() => vec![],
                Tag::LongArray(a) if a.is_empty() => vec![],
                _ => return Err("expected consume effect list".into()),
            };
            Ok(Value::List(effects))
        })
    })
}

/// Decode against the pinned component registry plus the owning registry's native types.
pub fn from_component_tag_with(id: &str, t: &Tag, is_native: fn(&str) -> bool) -> Result<Value> {
    component_tag(
        &identifier(id)?,
        t,
        Source {
            is_native,
            ..Source::SAVED
        },
    )
}
/// Datafix output represents native stacks, whose template counts are not bounded by CODEC.
pub fn from_upgraded_component_tag(id: &str, t: &Tag) -> Result<Value> {
    component_tag(
        &identifier(id)?,
        t,
        Source {
            upgraded: true,
            ..Source::SAVED
        },
    )
}
fn component_tag(id: &str, t: &Tag, source: Source) -> Result<Value> {
    source.component_type(id)?;
    if !source.upgraded && !super::is_persistent(id) {
        return Err(format!("{id} has no persistent codec"));
    }
    super::with_tree_stack(|| decode_component(id, t, source))
        .map_err(|error| format!("{id}: {error}"))
}
fn decode_component(id: &str, t: &Tag, source: Source) -> Result<Value> {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    validate_source(name, t)?;
    // Recursive template and predicate paths do not carry the large leaf dispatcher's frame.
    Ok(match name {
        "lock" => item_predicate(t, source)?,
        "use_remainder" | "sulfur_cube_content" => template(
            t,
            Source {
                deferred_template: true,
                ..source
            },
        )?,
        "can_place_on" | "can_break" => {
            if let Tag::List(v) = t {
                match v.len() {
                    0 => return Err("empty adventure predicate".into()),
                    1 => predicate(&v[0], source)?,
                    _ => list(t, |v| predicate(v, source))?,
                }
            } else {
                predicate(t, source)?
            }
        }
        "charged_projectiles" | "bundle_contents" => list(t, |v| template(v, source))?,
        "pot_decorations" => record(t, |r| {
            for k in ["back", "left", "right", "front"] {
                r.optional(k, |v| template(v, source))?;
            }
            Ok(())
        })?,
        "container" => {
            let Tag::List(entries) = t else {
                return Err("container is not a slot list".into());
            };
            if entries.len() > 256 {
                return Err("container source exceeds 256 entries".into());
            }
            let mut slots = vec![];
            for e in entries {
                let Value::Int(slot) = int(e.get("slot").ok_or("missing container slot")?)? else {
                    unreachable!()
                };
                if !(0..256).contains(&slot) {
                    return Err("container slot outside 0..255".into());
                }
                let item = template_tag(e.get("item").ok_or("missing container item")?, source)?;
                slots.resize_with(slots.len().max(slot as usize + 1), || None);
                slots[slot as usize] = Some(item);
            }
            Value::Container(slots)
        }
        _ => return component_leaf(name, id, t, source),
    })
}

#[inline(never)]
fn component_leaf(name: &str, id: &str, t: &Tag, source: Source) -> Result<Value> {
    Ok(match name {
        "creative_slot_lock" => unit(t)?,
        "map_post_processing" | "additional_trade_cost" => int(t)?,
        "custom_data" => {
            let compound = crate::custom_data::borrow(t)?;
            let value = Value::convert_compound(&compound, &[]);
            if let std::borrow::Cow::Owned(compound) = compound {
                sunrise_core::nbt::discard_tag(Tag::Compound(compound));
            }
            value
        }
        "bucket_entity_data" => {
            let Tag::Compound(value) = t else {
                return Err("bucket entity data is not a compound".into());
            };
            Value::convert_compound(value, &[])
        }
        "intangible_projectile" => unit(t)?,
        "map_decorations" => map(t, |v| {
            record(v, |r| {
                r.required("type", ident)?;
                r.required("x", double)?;
                r.required("z", double)?;
                r.required("rotation", float)
            })
        })?,
        "debug_stick_state" => {
            let Tag::Compound(entries) = t else {
                return Err("debug stick state is not a map".into());
            };
            Value::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        Ok((
                            Value::string(&identifier(&k.to_string_lossy())?),
                            string_value(v)?,
                        ))
                    })
                    .collect::<Result<_>>()?,
            )
        }
        "recipes" => list(t, ident)?,
        "container_loot" => record(t, |r| {
            r.required("loot_table", ident)?;
            r.default("seed", Value::Long(0), |v| {
                Ok(Value::Long(if let Tag::Long(v) = v {
                    *v
                } else {
                    number(v)? as i64
                }))
            })
        })?,
        "max_stack_size"
        | "max_damage"
        | "damage"
        | "repair_cost"
        | "map_id"
        | "ominous_bottle_amplifier" => int(t)?,
        "dyed_color" => rgb(t)?,
        "minimum_attack_charge" | "potion_duration_scale" => float(t)?,
        "enchantment_glint_override" => boolean(t)?,
        "unbreakable" | "glider" | "waxed" => unit(t)?,
        "custom_name" | "item_name" => text(t, source)?,
        "lore" => list(t, |v| text(v, source))?,
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
        | "axolotl/variant" => string_value(t)?,
        "item_model"
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
        | "jukebox_playable" => ident(t)?,
        "enchantments" | "stored_enchantments" => {
            let Tag::Compound(entries) = t else {
                return Err("enchantments are not a map".into());
            };
            let mut out = Vec::with_capacity(entries.len());
            for (k, v) in entries {
                let key = identifier(&k.to_string_lossy())?;
                let level = integer(v)?;
                if !(1..=255).contains(&level) {
                    return Err("enchantment level outside 1..255".into());
                }
                out.push((key, Value::Int(level)));
            }
            super::normalize_map(&mut out, |(key, _)| key);
            Value::Map(
                out.into_iter()
                    .map(|(key, value)| (Value::string(&key), value))
                    .collect(),
            )
        }
        "use_effects" => record(t, |r| {
            r.b("can_sprint", false)?;
            r.b("interact_vibrations", true)?;
            r.f("speed_multiplier", 0.2)
        })?,
        "food" => record(t, |r| {
            r.required("nutrition", int)?;
            r.required("saturation", float)?;
            r.b("can_always_eat", false)
        })?,
        "consumable" => effect_component(t, true, source)?,
        "use_cooldown" => record(t, |r| {
            r.required("seconds", float)?;
            r.optional("cooldown_group", ident)
        })?,
        "damage_resistant" => record(t, |r| r.required("types", holder_set))?,
        "repairable" => record(t, |r| r.required("items", holder_set))?,
        "enchantable" => record(t, |r| r.required("value", int))?,
        "villager_food" => record(t, |r| r.required("nutrition", int))?,
        "tool" => record(t, |r| {
            r.required("rules", |v| {
                list(v, |v| {
                    record(v, |r| {
                        r.required("blocks", holder_set)?;
                        r.optional("speed", float)?;
                        r.optional("correct_for_drops", boolean)
                    })
                })
            })?;
            r.f("default_mining_speed", 1.0)?;
            r.i("damage_per_block", 1)?;
            r.b("can_destroy_blocks_in_creative", true)
        })?,
        "weapon" => record(t, |r| {
            r.i("item_damage_per_attack", 1)?;
            r.f("disable_blocking_for_seconds", 0.0)
        })?,
        "attack_range" => record(t, |r| {
            for (k, v) in [
                ("min_reach", 0.0),
                ("max_reach", 3.0),
                ("min_creative_reach", 0.0),
                ("max_creative_reach", 5.0),
                ("hitbox_margin", 0.3),
                ("mob_factor", 1.0),
            ] {
                r.f(k, v)?;
            }
            Ok(())
        })?,
        "equippable" => record(t, |r| {
            r.required("slot", string_value)?;
            r.id("equip_sound", "minecraft:item.armor.equip_generic", sound)?;
            r.optional("asset_id", ident)?;
            r.optional("camera_overlay", ident)?;
            r.optional("allowed_entities", holder_set)?;
            for (k, v) in [
                ("dispensable", true),
                ("swappable", true),
                ("damage_on_hurt", true),
                ("equip_on_interact", false),
                ("can_be_sheared", false),
            ] {
                r.b(k, v)?;
            }
            r.id("shearing_sound", "minecraft:item.shears.snip", sound)
        })?,
        "death_protection" => effect_component(t, false, source)?,
        "piercing_weapon" => record(t, |r| {
            r.b("deals_knockback", true)?;
            r.b("dismounts", false)?;
            r.optional("sound", sound)?;
            r.optional("hit_sound", sound)
        })?,
        "kinetic_weapon" => record(t, |r| {
            r.i("contact_cooldown_ticks", 10)?;
            r.i("delay_ticks", 0)?;
            for k in [
                "dismount_conditions",
                "knockback_conditions",
                "damage_conditions",
            ] {
                r.optional(k, kinetic)?;
            }
            r.f("forward_movement", 0.0)?;
            r.f("damage_multiplier", 1.0)?;
            r.optional("sound", sound)?;
            r.optional("hit_sound", sound)
        })?,
        "attack_animation" | "interact_animation" => record(t, |r| {
            r.id("type", "whack", string_value)?;
            r.i("duration", 6)
        })?,
        "provides_banner_patterns" => holder_set(t)?,
        "break_sound" => sound(t)?,
        "compostable" => record(t, |r| r.required("layers", |v| resolvable(v, int)))?,
        "cooking_fuel" | "brewing_fuel" => record(t, |r| {
            r.required(
                if name == "cooking_fuel" {
                    "burn_time"
                } else {
                    "uses"
                },
                |v| resolvable(v, int),
            )?;
            r.required("speed_multiplier", |v| resolvable(v, float))
        })?,
        "mob_visibility" => record(t, |r| {
            r.required("targeting_entity_types", holder_set)?;
            r.required("visibility", float)
        })?,
        "attribute_modifiers" => list(t, |v| attribute_entry(v, source))?,
        "custom_model_data" => record(t, |r| {
            r.list("floats", float)?;
            r.list("flags", boolean)?;
            r.list("strings", string_value)?;
            r.list("colors", rgb)
        })?,
        "tooltip_display" => record(t, |r| {
            r.b("hide_tooltip", false)?;
            if let Some(v) = t.get("hidden_components") {
                let Tag::List(v) = v else {
                    return Err("expected list".into());
                };
                let mut seen = std::collections::HashSet::new();
                let mut set = vec![];
                for entry in v {
                    let name = identifier(&string(entry)?)?;
                    source.component_type(&name)?;
                    if seen.insert(name.clone()) {
                        set.push(Value::string(&name));
                    }
                }
                if !set.is_empty() {
                    r.fields
                        .push((Value::string("hidden_components"), Value::Set(set)));
                }
            }
            Ok(())
        })?,
        "potion_contents" => {
            if matches!(t, Tag::String(_)) {
                Value::Map(vec![(Value::string("potion"), ident(t)?)])
            } else {
                record(t, |r| {
                    r.optional("potion", ident)?;
                    r.optional("custom_color", int)?;
                    r.list("custom_effects", |v| effect_details(v, true))?;
                    r.optional("custom_name", string_value)
                })?
            }
        }
        "suspicious_stew_effects" => list(t, |v| {
            record(v, |r| {
                r.required("id", ident)?;
                if let Some(duration) = v.get("duration").and_then(|v| int(v).ok())
                    && duration != Value::Int(160)
                {
                    r.fields.push((Value::string("duration"), duration));
                }
                Ok(())
            })
        })?,
        "writable_book_content" => record(t, |r| r.list("pages", |v| filterable(v, string_value)))?,
        "written_book_content" => record(t, |r| {
            r.required("title", |v| filterable(v, string_value))?;
            r.required("author", string_value)?;
            r.i("generation", 0)?;
            r.list("pages", |v| filterable(v, |v| text(v, source)))?;
            r.b("resolved", false)
        })?,
        "trim" => record(t, |r| {
            r.required("material", |v| holder(v, |v| trim_material(v, source)))?;
            r.required("pattern", |v| holder(v, |v| trim_pattern(v, source)))
        })?,
        "instrument" => holder(t, |v| instrument(v, source))?,
        "provides_trim_material" => holder(t, |v| trim_material(v, source))?,
        "entity_data" | "block_entity_data" => {
            let mut v = t.clone();
            let id = ident(t.get("id").ok_or("missing typed data id")?)?.to_tag()?;
            let Tag::Compound(entries) = &mut v else {
                return Err("typed data not a compound".into());
            };
            entries.insert("id".into(), id);
            Value::convert_nbt(&v, &[])
        }
        "lodestone_tracker" => record(t, |r| {
            r.optional("target", |v| {
                record(v, |r| {
                    r.required("dimension", ident)?;
                    r.required("pos", |v| {
                        let v = integer_list(v)?;
                        if v.len() != 3 {
                            return Err("position must have three coordinates".into());
                        }
                        Ok(Value::IntArray(v))
                    })
                })
            })?;
            r.b("tracked", true)
        })?,
        "firework_explosion" => explosion(t)?,
        "fireworks" => record(t, |r| {
            r.default("flight_duration", Value::Byte(0), |v| {
                let Value::Int(v) = int(v)? else {
                    unreachable!()
                };
                Ok(Value::Byte(v as i8))
            })?;
            r.list("explosions", explosion)
        })?,
        "banner_patterns" => list(t, |v| {
            record(v, |r| {
                r.required("pattern", |v| holder(v, banner))?;
                r.required("color", string_value)
            })
        })?,
        "block_state" => map(t, string_value)?,
        "bees" => list(t, |v| {
            record(v, |r| {
                r.required("entity_data", |v| {
                    component_leaf("entity_data", "minecraft:entity_data", v, source)
                })?;
                r.required("ticks_in_hive", int)?;
                r.required("min_ticks_in_hive", int)
            })
        })?,
        "sign_text_front" | "sign_text_back" => sign_text(t, source)?,
        "profile" => profile(t)?,
        "blocks_attacks" => blocks_attacks(t)?,
        _ if !id.starts_with("minecraft:") => Value::convert_nbt(t, &[]),
        _ => return Err(format!("unknown component codec {id}")),
    })
}
fn profile(t: &Tag) -> Result<Value> {
    if matches!(t, Tag::String(_)) {
        return Ok(Value::Map(vec![(Value::string("name"), string_value(t)?)]));
    }
    record(t, |r| {
        r.optional("name", string_value)?;
        r.optional("id", |v| {
            let parts = integer_list(v)?;
            if parts.len() != 4 {
                return Err("profile UUID must have four integers".into());
            }
            Ok(Value::IntArray(parts))
        })?;
        if let Some(properties) = t.get("properties") {
            let properties = if let Tag::Compound(entries) = properties {
                let mut out = vec![];
                for (k, values) in entries {
                    let Tag::List(values) = values else {
                        return Err("profile property values are not a list".into());
                    };
                    for v in values {
                        out.push(Value::Map(vec![
                            (Value::string("name"), Value::String(k.0.clone())),
                            (Value::string("value"), string_value(v)?),
                        ]));
                    }
                }
                Value::List(out)
            } else {
                list(properties, |v| {
                    record(v, |r| {
                        r.required("name", string_value)?;
                        r.required("value", string_value)?;
                        r.optional("signature", string_value)
                    })
                })?
            };
            if !matches!(&properties,Value::List(v) if v.is_empty()) {
                r.fields.push((Value::string("properties"), properties));
            }
        }
        for key in ["texture", "cape", "elytra"] {
            r.optional(key, string_value)?;
        }
        r.optional("model", string_value)
    })
}
fn blocks_attacks(t: &Tag) -> Result<Value> {
    record(t, |r| {
        r.f("block_delay_seconds", 0.0)?;
        r.f("disable_cooldown_scale", 1.0)?;
        let default_reduction = Value::List(vec![Value::Map(vec![
            (Value::string("base"), Value::Float(0f32.to_bits())),
            (Value::string("factor"), Value::Float(1f32.to_bits())),
        ])]);
        r.default("damage_reductions", default_reduction, |v| {
            list(v, |v| {
                record(v, |r| {
                    r.f("horizontal_blocking_angle", 90.0)?;
                    r.optional("type", holder_set)?;
                    r.required("base", float)?;
                    r.required("factor", float)
                })
            })
        })?;
        let default_item = Value::Map(vec![
            (Value::string("threshold"), Value::Float(1f32.to_bits())),
            (Value::string("base"), Value::Float(0f32.to_bits())),
            (Value::string("factor"), Value::Float(1f32.to_bits())),
        ]);
        r.default("item_damage", default_item, |v| {
            record(v, |r| {
                r.required("threshold", float)?;
                r.required("base", float)?;
                r.required("factor", float)
            })
        })?;
        r.optional("bypassed_by", holder_set)?;
        r.optional("block_sound", sound)?;
        r.optional("disabled_sound", sound)
    })
}
fn only_fields(t: &Tag, allowed: &[&str]) -> Result<()> {
    let Tag::Compound(entries) = t else {
        return Err("expected predicate map".into());
    };
    for k in entries.keys() {
        if !allowed
            .iter()
            .any(|a| k.0.iter().copied().eq(a.encode_utf16()))
        {
            return Err(format!("unknown predicate field {}", k.to_string_lossy()));
        }
    }
    Ok(())
}
fn sign_messages(t: &Tag, source: Source) -> Result<Value> {
    let value = list(t, |v| text(v, source))?;
    if !matches!(&value,Value::List(v) if v.len()==4) {
        return Err("sign requires four messages".into());
    }
    Ok(value)
}
fn exact_components(t: &Tag, source: Source) -> Result<Value> {
    let Tag::Compound(entries) = t else {
        return Err("expected component map".into());
    };
    let mut out = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        let name = identifier(&k.to_string_lossy())?;
        if !super::is_persistent(&name) {
            return Err(format!("{name} has no persistent codec"));
        }
        let value = component_tag(&name, v, source)?;
        out.push((name, value));
    }
    super::normalize_map(&mut out, |(key, _)| key);
    Ok(Value::Map(
        out.into_iter()
            .map(|(key, value)| (Value::string(&key), value))
            .collect(),
    ))
}
fn partial_predicates(t: &Tag, source: Source) -> Result<Value> {
    let Tag::Compound(entries) = t else {
        return Err("expected partial predicate map".into());
    };
    let mut out = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        let name = identifier(&k.to_string_lossy())?;
        // The concrete predicate registry names its codecs after the component types.
        source.component_type(&name)?;
        let value = if name == "minecraft:custom_data" {
            let tag = if matches!(v, Tag::String(_)) {
                let s = string(v)?;
                let (tag, n) = sunrise_core::nbt::parse_snbt(&s)?;
                if n != s.len() {
                    return Err("trailing predicate SNBT".into());
                }
                tag
            } else {
                v.clone()
            };
            if !matches!(tag, Tag::Compound(_)) {
                return Err("custom data predicate must be a compound".into());
            }
            Value::string(&crate::snbt_text::to_snbt(&tag))
        } else if name == "minecraft:creative_slot_lock" {
            if !matches!(v, Tag::Compound(_)) {
                return Err(format!("{name}: expected presence predicate map"));
            }
            Value::Map(Vec::new())
        } else {
            predicate_value(v, source)?
        };
        out.push((Value::string(&name), value));
    }
    Ok(Value::Map(out))
}
fn predicate_value(t: &Tag, source: Source) -> Result<Value> {
    super::with_tree_stack(|| {
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
        match t {
            Tag::Compound(entries) => Ok(Value::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        let name = k.to_string_lossy();
                        let value = if name == "components" {
                            exact_components(v, source)?
                        } else if BOOLS.contains(&name.as_str()) {
                            boolean(v)?
                        } else {
                            predicate_value(v, source)?
                        };
                        Ok((Value::String(k.0.clone()), value))
                    })
                    .collect::<Result<_>>()?,
            )),
            Tag::List(_) => list(t, |v| predicate_value(v, source)),
            _ => Ok(Value::convert_nbt(t, &[])),
        }
    })
}
fn item_predicate(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        r.optional("items", holder_set)?;
        r.default("count", Value::Map(vec![]), |v| {
            if matches!(v, Tag::Compound(_)) {
                record(v, |r| {
                    r.optional("min", int)?;
                    r.optional("max", int)
                })
            } else {
                int(v)
            }
        })?;
        r.default("components", Value::Map(vec![]), |v| {
            exact_components(v, source)
        })?;
        r.default("predicates", Value::Map(vec![]), |v| {
            partial_predicates(v, source)
        })
    })
}
/// A borrowed view used by the saved codec's constraints on both raw source tags
/// and stream values whose shape was built by their typed encoder.
#[derive(Clone, Copy)]
enum Input<'a> {
    Tag(&'a Tag),
    Value(&'a Value),
}
impl<'a> Input<'a> {
    fn field(self, name: &str) -> Option<Self> {
        match self {
            Self::Tag(tag) => tag.get(name).map(Self::Tag),
            Self::Value(Value::Map(entries)) => entries.iter().rev().find_map(|(k, value)| {
                matches!(k, Value::String(units) if units.iter().copied().eq(name.encode_utf16()))
                    .then_some(Self::Value(value))
            }),
            _ => None,
        }
    }
    fn list(self) -> impl Iterator<Item = Self> + 'a {
        let (tags, values): (&[Tag], &[Value]) = match self {
            Self::Tag(Tag::List(v)) => (v, &[]),
            Self::Value(Value::List(v) | Value::Set(v)) => (&[], v),
            _ => (&[], &[]),
        };
        tags.iter()
            .map(Self::Tag)
            .chain(values.iter().map(Self::Value))
    }
    fn string(self) -> Option<&'a [u16]> {
        match self {
            Self::Tag(Tag::String(v)) => Some(&v.0),
            Self::Value(Value::String(v)) => Some(v),
            _ => None,
        }
    }
    fn matches(self, expected: &str) -> bool {
        self.string()
            .is_some_and(|v| v.iter().copied().eq(expected.encode_utf16()))
    }
}

fn validate_source(name: &str, tag: &Tag) -> Result<()> {
    validate_codec_limits(name, Input::Tag(tag))
}

/// Validate the persistent constraints not enforced by the stream type itself.
/// The typed stream encoder owns the record shape; this does not project NBT.
#[doc(hidden)]
pub fn validate_stream_value(id: &str, value: &Value) -> Result<()> {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    let input = Input::Value(value);
    validate_codec_limits(name, input)?;
    match (name, value) {
        ("enchantments" | "stored_enchantments", Value::Map(entries)) => {
            for (_, level) in entries {
                if !matches!(level, Value::Int(n) if (1..=255).contains(n)) {
                    return Err("enchantment level outside 1..255".into());
                }
            }
        }
        ("consumable" | "death_protection", _) => {
            let key = if name == "consumable" {
                "on_consume_effects"
            } else {
                "death_effects"
            };
            if let Some(effects) = input.field(key) {
                for effect in effects.list() {
                    if let Some(kind) = effect.field("type") {
                        for (id, codec) in [
                            ("minecraft:apply_effects", "apply_effects"),
                            ("minecraft:teleport_randomly", "teleport_randomly"),
                        ] {
                            if kind.matches(id) {
                                validate_codec_limits(codec, effect)?;
                            }
                        }
                    }
                }
            }
        }
        ("can_place_on" | "can_break", _) => {
            let check = |predicate: Input<'_>| {
                if let Some(Input::Value(Value::Map(entries))) = predicate.field("components") {
                    for (key, value) in entries {
                        let Value::String(key) = key else {
                            return Err("invalid exact component key".into());
                        };
                        let key = String::from_utf16_lossy(key);
                        if !super::is_persistent(&key) {
                            return Err(format!("transient exact component {key}"));
                        }
                        super::with_tree_stack(|| validate_stream_value(&key, value))?;
                    }
                }
                Ok(())
            };
            if matches!(value, Value::List(_)) {
                if input.list().next().is_none() {
                    return Err("empty adventure predicate".into());
                }
                for predicate in input.list() {
                    check(predicate)?;
                }
            } else {
                check(input)?;
            }
        }
        ("custom_data" | "bucket_entity_data", value) if !matches!(value, Value::Map(_)) => {
            return Err("expected compound data".into());
        }
        ("container", Value::Container(slots)) if slots.len() > 256 => {
            return Err("container source exceeds 256 entries".into());
        }
        _ => (),
    }
    Ok(())
}

fn validate_codec_limits(name: &str, t: Input<'_>) -> Result<()> {
    fn integer(t: Input<'_>, min: i32, max: i32) -> Result<()> {
        let n = match t {
            Input::Tag(t) => super::source::integer(t)?,
            Input::Value(Value::Int(n)) => *n,
            _ => return Err("expected integer".into()),
        };
        if !(min..=max).contains(&n) {
            return Err(format!("integer outside {min}..{max}"));
        }
        Ok(())
    }
    fn real(t: Input<'_>, min: f32, max: f32, exclusive: bool) -> Result<()> {
        let bits = match t {
            Input::Tag(t) => {
                let Value::Float(n) = float(t)? else {
                    unreachable!()
                };
                n
            }
            Input::Value(Value::Float(n)) => *n,
            _ => return Err("expected float".into()),
        };
        let n = f32::from_bits(bits);
        // `Float.compareTo` order, as `ExtraCodecs.floatRange` uses.
        let below = n.total_cmp(&min);
        if !n.is_finite()
            || below.is_lt()
            || exclusive && below.is_eq()
            || n.total_cmp(&max).is_gt()
        {
            return Err("float outside component codec range".into());
        }
        Ok(())
    }
    fn field(t: Input<'_>, k: &str, f: impl FnOnce(Input<'_>) -> Result<()>) -> Result<()> {
        t.field(k).map_or(Ok(()), f)
    }
    fn enumeration(t: Input<'_>, values: &[&str]) -> Result<()> {
        let units = t.string().ok_or("expected string")?;
        if !values
            .iter()
            .any(|value| units.iter().copied().eq(value.encode_utf16()))
        {
            return Err(format!(
                "unknown enum value {}",
                String::from_utf16_lossy(units)
            ));
        }
        Ok(())
    }
    fn limit(t: Input<'_>, max: usize) -> Result<()> {
        let n = match t {
            Input::Tag(Tag::List(v)) => v.len(),
            Input::Tag(Tag::ByteArray(v)) | Input::Value(Value::ByteArray(v)) => v.len(),
            Input::Tag(Tag::IntArray(v)) | Input::Value(Value::IntArray(v)) => v.len(),
            Input::Tag(Tag::LongArray(v)) | Input::Value(Value::LongArray(v)) => v.len(),
            Input::Value(Value::List(v) | Value::Set(v)) => v.len(),
            _ => return Ok(()),
        };
        if n > max {
            return Err(format!("component list exceeds {max}"));
        }
        Ok(())
    }
    fn string_limit(t: Input<'_>, max: usize) -> Result<()> {
        if let Some(v) = t.string() {
            if v.len() > max {
                return Err(format!("component string exceeds {max}"));
            }
        } else {
            for k in ["raw", "filtered"] {
                field(t, k, |v| super::with_tree_stack(|| string_limit(v, max)))?;
            }
        }
        Ok(())
    }
    const DYE: &[&str] = &[
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    match name {
        "max_stack_size" => integer(t, 1, 99)?,
        "max_damage" => integer(t, 1, i32::MAX)?,
        "damage" | "repair_cost" => integer(t, 0, i32::MAX)?,
        "ominous_bottle_amplifier" => integer(t, 0, 4)?,
        "minimum_attack_charge" => real(t, 0.0, 1.0, false)?,
        "potion_duration_scale" => real(t, 0.0, f32::MAX, false)?,
        "rarity" => enumeration(t, &["common", "uncommon", "rare", "epic"])?,
        "base_color"
        | "dye"
        | "wolf/collar"
        | "tropical_fish/base_color"
        | "tropical_fish/pattern_color"
        | "cat/collar"
        | "sheep/color"
        | "shulker/color"
        | "cushion/color" => enumeration(t, DYE)?,
        "fox/variant" => enumeration(t, &["red", "snow"])?,
        "salmon/size" => enumeration(t, &["small", "medium", "large"])?,
        "parrot/variant" => enumeration(t, &["red_blue", "blue", "green", "yellow_blue", "gray"])?,
        "tropical_fish/pattern" => enumeration(
            t,
            &[
                "kob",
                "sunstreak",
                "snooper",
                "dasher",
                "brinely",
                "spotty",
                "flopper",
                "stripey",
                "glitter",
                "blockfish",
                "betty",
                "clayfish",
            ],
        )?,
        "mooshroom/variant" => enumeration(t, &["red", "brown"])?,
        "rabbit/variant" => enumeration(
            t,
            &[
                "brown",
                "white",
                "black",
                "white_splotched",
                "gold",
                "salt",
                "evil",
            ],
        )?,
        "horse/variant" => enumeration(
            t,
            &[
                "white",
                "creamy",
                "chestnut",
                "brown",
                "black",
                "gray",
                "dark_brown",
            ],
        )?,
        "llama/variant" => enumeration(t, &["creamy", "white", "brown", "gray"])?,
        "axolotl/variant" => enumeration(t, &["lucy", "wild", "gold", "cyan", "blue"])?,
        "use_effects" => field(t, "speed_multiplier", |v| real(v, 0.0, 1.0, false))?,
        "food" => field(t, "nutrition", |v| integer(v, 0, i32::MAX))?,
        "consumable" => {
            field(t, "consume_seconds", |v| real(v, 0.0, f32::MAX, false))?;
            field(t, "animation", |v| {
                enumeration(
                    v,
                    &[
                        "none",
                        "eat",
                        "drink",
                        "block",
                        "bow",
                        "trident",
                        "crossbow",
                        "spyglass",
                        "toot_horn",
                        "brush",
                        "bundle",
                        "spear",
                    ],
                )
            })?;
        }
        "use_cooldown" => field(t, "seconds", |v| real(v, 0.0, f32::MAX, true))?,
        "enchantable" => field(t, "value", |v| integer(v, 1, i32::MAX))?,
        "villager_food" => field(t, "nutrition", |v| integer(v, 1, i32::MAX))?,
        "tool" => {
            field(t, "damage_per_block", |v| integer(v, 0, i32::MAX))?;
            if let Some(rules) = t.field("rules") {
                for rule in rules.list() {
                    field(rule, "speed", |v| real(v, 0.0, f32::MAX, true))?;
                }
            }
        }
        "weapon" => {
            field(t, "item_damage_per_attack", |v| integer(v, 0, i32::MAX))?;
            field(t, "disable_blocking_for_seconds", |v| {
                real(v, 0.0, f32::MAX, false)
            })?;
        }
        "attack_range" => {
            for k in [
                "min_reach",
                "max_reach",
                "min_creative_reach",
                "max_creative_reach",
            ] {
                field(t, k, |v| real(v, 0.0, 64.0, false))?;
            }
            field(t, "hitbox_margin", |v| real(v, 0.0, 1.0, false))?;
            field(t, "mob_factor", |v| real(v, 0.0, 2.0, false))?;
        }
        "equippable" => field(t, "slot", |v| {
            enumeration(
                v,
                &[
                    "mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle",
                ],
            )
        })?,
        "kinetic_weapon" => {
            for k in ["contact_cooldown_ticks", "delay_ticks"] {
                field(t, k, |v| integer(v, 0, i32::MAX))?;
            }
            for k in [
                "dismount_conditions",
                "knockback_conditions",
                "damage_conditions",
            ] {
                if let Some(v) = t.field(k) {
                    field(v, "max_duration_ticks", |v| integer(v, 0, i32::MAX))?;
                }
            }
        }
        "attack_animation" | "interact_animation" => {
            field(t, "type", |v| enumeration(v, &["none", "whack", "stab"]))?;
            field(t, "duration", |v| integer(v, 0, i32::MAX))?;
        }
        "mob_visibility" => field(t, "visibility", |v| real(v, 0.0, 10.0, false))?,
        "firework_explosion" => field(t, "shape", |v| {
            enumeration(v, &["small_ball", "large_ball", "star", "creeper", "burst"])
        })?,
        "instrument" => {
            field(t, "use_duration", |v| real(v, 0.0, f32::MAX, false))?;
            field(t, "range", |v| real(v, 0.0, f32::MAX, true))?;
            field(t, "durability_damage", |v| integer(v, 0, i32::MAX))?;
        }
        "apply_effects" => field(t, "probability", |v| real(v, 0.0, 1.0, false))?,
        "teleport_randomly" => field(t, "diameter", |v| real(v, 0.0, f32::MAX, true))?,
        "lore" => limit(t, 256)?,
        "fireworks" => field(t, "explosions", |v| limit(v, 256))?,
        "writable_book_content" => {
            field(t, "pages", |v| {
                limit(v, 100)?;
                for v in v.list() {
                    string_limit(v, 1024)?;
                }
                Ok(())
            })?;
        }
        "written_book_content" => {
            field(t, "title", |v| string_limit(v, 32))?;
            field(t, "generation", |v| integer(v, 0, 3))?;
        }
        "blocks_attacks" => {
            for k in ["block_delay_seconds", "disable_cooldown_scale"] {
                field(t, k, |v| real(v, 0.0, f32::MAX, false))?;
            }
            if let Some(reductions) = t.field("damage_reductions") {
                for v in reductions.list() {
                    field(v, "horizontal_blocking_angle", |v| {
                        real(v, 0.0, f32::MAX, true)
                    })?;
                }
            }
        }
        _ => (),
    }
    Ok(())
}
fn attribute_entry(t: &Tag, source: Source) -> Result<Value> {
    fn enumeration(t: &Tag, values: &[&str]) -> Result<Value> {
        let name = string(t)?;
        if !values.contains(&name.as_str()) {
            return Err(format!("unknown enum value {name}"));
        }
        string_value(t)
    }
    record(t, |r| {
        r.required("type", ident)?;
        r.required("id", ident)?;
        r.required("amount", double)?;
        r.required("operation", |v| {
            enumeration(
                v,
                &["add_value", "add_multiplied_base", "add_multiplied_total"],
            )
        })?;
        r.id("slot", "any", |v| {
            enumeration(
                v,
                &[
                    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor",
                    "body", "saddle",
                ],
            )
        })?;
        if let Some(display) = t.get("display") {
            let name = string(
                display
                    .get("type")
                    .ok_or("missing attribute display type")?,
            )?;
            if name != "default" {
                let display = record(display, |r| {
                    r.required("type", |v| enumeration(v, &["hidden", "override"]))?;
                    if name == "override" {
                        r.required("value", |v| text(v, source))?;
                    }
                    Ok(())
                })?;
                r.fields.push((Value::string("display"), display));
            }
        }
        Ok(())
    })
}
fn sign_text(t: &Tag, source: Source) -> Result<Value> {
    record(t, |r| {
        r.required("messages", |v| sign_messages(v, source))?;
        if let Some(filtered) = t
            .get("filtered_messages")
            .and_then(|v| sign_messages(v, source).ok())
            && Some(&filtered) != r.fields.first().map(|(_, v)| v)
        {
            r.fields
                .push((Value::string("filtered_messages"), filtered));
        }
        r.fields.push((
            Value::string("color"),
            t.get("color")
                .map(string_value)
                .transpose()?
                .unwrap_or_else(|| Value::string("black")),
        ));
        r.fields.push((
            Value::string("has_glowing_text"),
            t.get("has_glowing_text")
                .map(boolean)
                .transpose()?
                .unwrap_or(Value::Bool(false)),
        ));
        Ok(())
    })
}
fn rgb(t: &Tag) -> Result<Value> {
    sunrise_core::color::rgb_color_from_tag(t)
        .map(Value::Int)
        .ok_or_else(|| "expected an RGB number or three-channel vector".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> Tag {
        sunrise_core::nbt::parse_snbt(s).unwrap().0
    }
    #[test]
    fn source_alternatives_and_always_present_defaults_use_codec_types() {
        assert_eq!(
            from_component_tag("minecraft:potion_contents", &parse(r#""healing""#))
                .unwrap()
                .to_tag()
                .unwrap(),
            parse(r#"{potion:"minecraft:healing"}"#)
        );
        assert_eq!(
            from_component_tag("minecraft:profile", &parse(r#""Player""#))
                .unwrap()
                .to_tag()
                .unwrap(),
            parse(r#"{name:"Player"}"#)
        );
        let effects = from_component_tag(
            "minecraft:potion_contents",
            &parse(r#"{custom_effects:[{id:"speed",amplifier:255,show_particles:0b}]}"#),
        )
        .unwrap();
        assert_eq!(
            effects.to_tag().unwrap(),
            parse(
                r#"{custom_effects:[{id:"minecraft:speed",amplifier:-1b,show_particles:0b,show_icon:0b}]}"#
            )
        );
        let sign = from_component_tag(
            "minecraft:sign_text_front",
            &parse(r#"{messages:["a","b","c","d"],filtered_messages:["bad"]}"#),
        )
        .unwrap();
        assert_eq!(
            sign.to_tag().unwrap(),
            parse(r#"{messages:["a","b","c","d"],color:"black",has_glowing_text:0b}"#)
        );
    }
    #[test]
    fn numeric_source_alternatives_follow_java_conversion_not_tag_identity() {
        assert_eq!(
            from_component_tag("minecraft:map_id", &Tag::Long(0x1_0000_0001)).unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            from_component_tag("minecraft:dyed_color", &parse("[1.0f,0.0f,0.5f]")).unwrap(),
            Value::Int(0xffff007fu32 as i32)
        );
        let value = from_component_tag(
            "minecraft:custom_model_data",
            &parse("{floats:[I;1,2],flags:[B;0b,1b]}"),
        )
        .unwrap();
        assert_eq!(
            value.to_tag().unwrap(),
            parse("{floats:[1.0f,2.0f],flags:[0b,1b]}")
        );
    }
    #[test]
    fn upgraded_native_templates_do_not_use_persistent_count_bounds() {
        for count in [i32::MIN, -7, 0, 1, 99, 100, 127, i32::MAX] {
            let item = Tag::compound([
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(count)),
            ]);
            let tag = Tag::List(vec![item]);
            let source = from_component_tag("minecraft:bundle_contents", &tag);
            let native = from_upgraded_component_tag("minecraft:bundle_contents", &tag);
            if count == 0 {
                assert!(source.is_err());
                assert!(native.is_err());
                continue;
            }
            let native = native.unwrap();
            if (1..=99).contains(&count) {
                assert_eq!(source.unwrap(), native);
            } else {
                assert!(source.is_err());
                assert!(native.to_tag().is_err());
            }
            assert!(
                matches!(&native,Value::List(values) if matches!(&values[0],Value::Template(item) if item.count==count))
            );
        }
    }
    #[test]
    fn persistent_input_refuses_transient_components_by_name_but_native_stacks_keep_them() {
        for name in [
            "creative_slot_lock",
            "map_post_processing",
            "additional_trade_cost",
        ] {
            let value = if name == "creative_slot_lock" {
                Tag::Compound(Default::default())
            } else {
                Tag::Int(3)
            };
            let error = from_component_tag(&format!("minecraft:{name}"), &value).unwrap_err();
            assert!(error.contains(name), "{error}");
            assert!(from_upgraded_component_tag(&format!("minecraft:{name}"), &value).is_ok());
        }
        let removal =
            parse(r#"{id:"minecraft:stone",components:{"!minecraft:map_post_processing":{}}}"#);
        assert!(
            template_from_tag(&removal)
                .unwrap_err()
                .contains("map_post_processing")
        );
        let native = from_upgraded_component_tag("minecraft:use_remainder", &removal).unwrap();
        let Some(native) = native.into_template() else {
            panic!("use_remainder holds a template");
        };
        assert_eq!(
            native.patch,
            vec![("minecraft:map_post_processing".into(), None)]
        );

        let hidden = parse(r#"{hidden_components:["minecraft:map_post_processing"]}"#);
        assert!(from_component_tag("minecraft:tooltip_display", &hidden).is_ok());
    }
    #[test]
    fn float_ranges_follow_java_ordering_and_refuse_negative_zero_at_a_zero_minimum() {
        let float = |bits: u32| Tag::Float(bits);
        assert!(from_component_tag("minecraft:potion_duration_scale", &float(0)).is_ok());
        assert!(
            from_component_tag("minecraft:potion_duration_scale", &float(0x8000_0000)).is_err()
        );
        assert!(
            from_component_tag(
                "minecraft:potion_duration_scale",
                &float(f32::NAN.to_bits())
            )
            .is_err()
        );
        assert!(
            from_component_tag(
                "minecraft:potion_duration_scale",
                &float(f32::INFINITY.to_bits())
            )
            .is_err()
        );
        let cooldown = |bits: u32| Tag::compound([("seconds", float(bits))]);
        assert!(from_component_tag("minecraft:use_cooldown", &cooldown(1.0f32.to_bits())).is_ok());
        assert!(from_component_tag("minecraft:use_cooldown", &cooldown(0)).is_err());
        assert!(from_component_tag("minecraft:use_cooldown", &cooldown(0x8000_0000)).is_err());
        for (id, field) in [
            ("minecraft:use_effects", "speed_multiplier"),
            ("minecraft:attack_range", "min_reach"),
        ] {
            assert!(from_component_tag(id, &Tag::compound([(field, float(0))])).is_ok());
            assert!(from_component_tag(id, &Tag::compound([(field, float(0x8000_0000))])).is_err());
        }
    }
    #[test]
    fn saved_component_keys_resolve_membership_before_payloads() {
        let valid = parse(r#"{id:"minecraft:stone",components:{"!minecraft:damage":{ignored:7}}}"#);
        assert_eq!(
            template_from_tag(&valid).unwrap().patch,
            vec![("minecraft:damage".into(), None)]
        );
        for (source, missing) in [
            (
                r#"{id:"minecraft:stone",components:{"!minecraft:damage":7}}"#,
                "minecraft:damage",
            ),
            (
                r#"{id:"minecraft:stone",components:{"!minecraft:not_a_component":{}}}"#,
                "minecraft:not_a_component",
            ),
            (
                r#"{id:"minecraft:stone",components:{"mod:unknown":{}}}"#,
                "mod:unknown",
            ),
        ] {
            assert!(
                template_from_tag(&parse(source))
                    .unwrap_err()
                    .contains(missing)
            );
        }
        let hidden = parse(r#"{hidden_components:["minecraft:not_a_component"]}"#);
        assert!(
            from_component_tag("tooltip_display", &hidden)
                .unwrap_err()
                .contains("minecraft:not_a_component")
        );
        assert!(
            from_component_tag("damage", &Tag::Int(-1))
                .unwrap_err()
                .contains("minecraft:damage")
        );
    }

    #[test]
    fn saved_text_rejects_non_components_on_input_and_projection() {
        for tag in [
            Tag::Int(4),
            Tag::List(vec![]),
            parse("{text:4}"),
            parse(r#"{text:"hi",extra:[4]}"#),
        ] {
            assert!(
                from_component_tag("custom_name", &tag)
                    .unwrap_err()
                    .contains("minecraft:custom_name")
            );
            assert!(
                Value::from_nbt(&tag, &[])
                    .to_component_tag("minecraft:custom_name")
                    .is_err()
            );
        }
        let text = parse(r#"{text:"hi",extra:["!"]}"#);
        assert_eq!(
            from_component_tag("custom_name", &text)
                .unwrap()
                .to_component_tag("custom_name")
                .unwrap(),
            text
        );
    }

    #[test]
    fn saved_text_refuses_malformed_style_fields_on_decode_and_projection() {
        for style in [
            r#"color:7"#,
            r#"color:"reset""#,
            r##"color:"#1000000""##,
            r#"font:"..:uniform""#,
            r#"font:{id:"uniform"}"#,
            r#"click_event:{action:"open_file",path:"x"}"#,
            r#"click_event:{action:"open_url",url:"https://example.org/%zz"}"#,
            r#"click_event:{action:"run_command",value:"/say hi"}"#,
            r#"click_event:{action:"suggest_command",command:"/say §hi"}"#,
            r#"click_event:{action:"change_page",page:0}"#,
            r#"click_event:{action:"show_dialog",dialog:7}"#,
            r#"hover_event:{action:"show_text",contents:"legacy"}"#,
            r#"hover_event:{action:"show_entity",id:"pig",uuid:[I;1,2]}"#,
            r#"hover_event:{action:"show_item",id:"stone",count:100}"#,
            r#"hover_event:{action:"show_item",id:"stone",components:{damage:-1}}"#,
        ] {
            let tag = parse(&format!(r#"{{text:"hi",{style}}}"#));
            assert!(from_component_tag("custom_name", &tag).is_err(), "{style}");
            assert!(
                Value::from_nbt(&tag, &[])
                    .to_component_tag("custom_name")
                    .is_err(),
                "{style}"
            );
        }
    }

    #[test]
    fn saved_text_refuses_nested_hover_transient_sets_and_removals() {
        for tag in [
            parse(
                r#"{text:"item",hover_event:{action:"show_item",id:"stone",components:{creative_slot_lock:{}}}}"#,
            ),
            parse(
                r#"{text:"outer",hover_event:{action:"show_text",value:{text:"item",hover_event:{action:"show_item",id:"stone",components:{"!creative_slot_lock":{}}}}}}"#,
            ),
        ] {
            for error in [
                from_component_tag("custom_name", &tag).unwrap_err(),
                Value::from_nbt(&tag, &[])
                    .to_component_tag("custom_name")
                    .unwrap_err(),
            ] {
                assert!(error.contains("minecraft:custom_name"));
                assert!(error.contains("minecraft:creative_slot_lock"));
            }
        }
    }

    #[test]
    fn saved_text_projects_canonical_style_and_the_hover_items_own_patch() {
        let input = parse(
            r##"{
            type:"text",text:"item",color:"#+Ａ",font:":uniform",bold:2s,
            click_event:{action:"change_page",page:2s},
            hover_event:{action:"show_item",id:"stone",count:1b,components:{
                damage:2s,
                "!unbreakable":{ignored:1},
                custom_name:{type:"text",text:"inner"},
                custom_data:{bold:7b,creative_slot_lock:{}}
            }}
        }"##,
        );
        let output = parse(
            r##"{
            text:"item",color:"#00000A",font:"minecraft:uniform",bold:1b,
            click_event:{action:"change_page",page:2},
            hover_event:{action:"show_item",id:"minecraft:stone",components:{
                "minecraft:damage":2,
                "!minecraft:unbreakable":{},
                "minecraft:custom_name":"inner",
                "minecraft:custom_data":{bold:7b,creative_slot_lock:{}}
            }}
        }"##,
        );
        let value = from_component_tag("custom_name", &input).unwrap();
        assert_eq!(value.to_component_tag("custom_name").unwrap(), output);
        assert_eq!(from_component_tag("custom_name", &output).unwrap(), value);
    }

    #[test]
    fn saved_custom_data_accepts_flattened_snbt_as_the_same_compound() {
        let compound = parse("{x:1}");
        let value = from_component_tag("custom_data", &Tag::String("{x:1}  ".into())).unwrap();
        assert_eq!(value, from_component_tag("custom_data", &compound).unwrap());
        assert_eq!(value.to_component_tag("custom_data").unwrap(), compound);
        for invalid in ["{x:1} junk", "[1]"] {
            assert!(from_component_tag("custom_data", &Tag::String(invalid.into())).is_err());
        }
    }

    #[test]
    fn saved_and_upgraded_trees_can_exceed_the_binary_read_depth() {
        // Go far enough past 512 to exercise stack growth on smaller workers.
        std::thread::Builder::new()
            .stack_size(512 << 10)
            .spawn(|| {
                let mut tag = Tag::Int(1);
                let levels = 1200;
                for _ in 0..levels {
                    tag = Tag::compound([("a", tag)]);
                }
                let flattened = |n: usize| {
                    Tag::String(
                        format!("{}1{}", "{a:".repeat(n), "}".repeat(n))
                            .as_str()
                            .into(),
                    )
                };
                assert!(from_component_tag("custom_data", &flattened(512)).is_ok());
                assert!(from_component_tag("custom_data", &flattened(513)).is_err());
                for value in [
                    from_component_tag("custom_data", &tag).unwrap(),
                    from_upgraded_component_tag("custom_data", &tag).unwrap(),
                ] {
                    let output = value.to_component_tag("custom_data").unwrap();
                    let mut leaf = &output;
                    for _ in 0..levels {
                        leaf = leaf.get("a").unwrap();
                    }
                    assert_eq!(leaf, &Tag::Int(1));
                    sunrise_core::nbt::discard_tag(output);
                }
                sunrise_core::nbt::discard_tag(tag);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
