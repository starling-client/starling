//! Canonical values to the pinned stream codec, with explicit registry lookup.
use super::{Ids, builtin_component_id, component_name, is_native_component};
use crate::item::{
    Component, Stack,
    component::{BUNDLE_CONTENTS, CHARGED_PROJECTILES},
};
use crate::{Error, Result, buf::Reader};
use sunrise_core::{
    nbt::{self, Tag},
    vanilla_bytes::Writer,
};
use sunrise_world::component_value::{Template, Value, consume_effects::Admitted, effective_patch};

fn err(s: impl Into<String>) -> Error {
    Error::new(s.into())
}
fn field<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    if let Value::Map(entries) = v {
        entries.iter().rev().find_map(|(k, v)| {
            matches!(k,Value::String(k) if k.iter().copied().eq(name.encode_utf16())).then_some(v)
        })
    } else {
        None
    }
}
fn need<'a>(v: &'a Value, name: &str) -> Result<&'a Value> {
    field(v, name).ok_or_else(|| err(format!("missing component field {name}")))
}
fn string(v: &Value) -> Result<String> {
    if let Value::String(s) = v {
        String::from_utf16(s).map_err(|_| err("invalid identifier/string UTF-16"))
    } else {
        Err(err("expected component string"))
    }
}
fn number(v: &Value) -> Result<f64> {
    Ok(match v {
        Value::Byte(v) => *v as f64,
        Value::Short(v) => *v as f64,
        Value::Int(v) => *v as f64,
        Value::Long(v) => *v as f64,
        Value::Float(v) => f32::from_bits(*v) as f64,
        Value::Double(v) => f64::from_bits(*v),
        _ => return Err(err("expected component number")),
    })
}
fn int(v: &Value) -> Result<i32> {
    Ok(match v {
        Value::Long(v) => *v as i32,
        _ => number(v)? as i32,
    })
}
fn float(v: &Value) -> Result<f32> {
    if let Value::Float(v) = v {
        Ok(f32::from_bits(*v))
    } else {
        Ok(number(v)? as f32)
    }
}
fn boolean(v: &Value) -> Result<bool> {
    if let Value::Bool(v) = v {
        Ok(*v)
    } else {
        Err(err("expected component boolean"))
    }
}
fn list(v: &Value) -> Result<&[Value]> {
    match v {
        Value::List(v) | Value::Set(v) => Ok(v),
        _ => Err(err("expected component list")),
    }
}
fn map(v: &Value) -> Result<&[(Value, Value)]> {
    if let Value::Map(v) = v {
        Ok(v)
    } else {
        Err(err("expected component map"))
    }
}
fn i(v: &Value, k: &str, d: i32) -> Result<i32> {
    field(v, k).map(int).transpose().map(|v| v.unwrap_or(d))
}
fn f(v: &Value, k: &str, d: f32) -> Result<f32> {
    field(v, k).map(float).transpose().map(|v| v.unwrap_or(d))
}
fn b(v: &Value, k: &str, d: bool) -> Result<bool> {
    field(v, k).map(boolean).transpose().map(|v| v.unwrap_or(d))
}
fn named_default(v: &Value, k: &str, default: i32, names: &[&str]) -> Result<i32> {
    field(v, k)
        .map(|v| named(v, names))
        .transpose()
        .map(|v| v.unwrap_or(default))
}
fn values<'a>(v: &'a Value, k: &str) -> Result<&'a [Value]> {
    field(v, k).map(list).transpose().map(|v| v.unwrap_or(&[]))
}
fn id(ids: &dyn Ids, registry: &str, v: &Value) -> Result<i32> {
    let name = string(v)?;
    ids.id(registry, &name)
        .filter(|id| *id >= 0)
        .ok_or_else(|| err(format!("unknown {registry} entry {name}")))
}
/// A registry id as the stream's holder reference: one above the id, with 0 for an inline value.
fn holder_id(ids: &dyn Ids, registry: &str, v: &Value) -> Result<i32> {
    id(ids, registry, v)?
        .checked_add(1)
        .ok_or_else(|| err(format!("{registry} id overflows the holder reference")))
}
fn named(v: &Value, names: &[&str]) -> Result<i32> {
    let name = string(v)?;
    names
        .iter()
        .position(|n| *n == name)
        .map(|n| n as i32)
        .ok_or_else(|| err(format!("unknown enum value {name}")))
}
fn optional(
    w: &mut Writer,
    v: Option<&Value>,
    f: impl FnOnce(&mut Writer, &Value) -> Result<()>,
) -> Result<()> {
    w.bool(v.is_some());
    if let Some(v) = v {
        f(w, v)?;
    }
    Ok(())
}
fn sequence(
    w: &mut Writer,
    v: &[Value],
    mut f: impl FnMut(&mut Writer, &Value) -> Result<()>,
) -> Result<()> {
    w.varint(v.len() as i32);
    for v in v {
        f(w, v)?;
    }
    Ok(())
}
fn nbt(w: &mut Writer, v: &Value) -> Result<()> {
    write_tag(w, &v.to_tag().map_err(err)?)
}

fn write_tag(w: &mut Writer, tag: &Tag) -> Result<()> {
    let bytes = nbt::write_network(tag, nbt::Limits::default()).map_err(|e| err(e.to_string()))?;
    w.bytes(&bytes);
    Ok(())
}
fn sound(w: &mut Writer, v: &Value, ids: &dyn Ids) -> Result<()> {
    if matches!(v, Value::String(_)) {
        w.varint(holder_id(ids, "minecraft:sound_event", v)?);
    } else {
        w.varint(0);
        w.string(&string(need(v, "sound_id")?)?);
        optional(w, field(v, "range"), |w, v| {
            w.f32(float(v)?);
            Ok(())
        })?;
    }
    Ok(())
}
fn sound_default(w: &mut Writer, v: &Value, k: &str, d: &str, ids: &dyn Ids) -> Result<()> {
    if let Some(v) = field(v, k) {
        sound(w, v, ids)
    } else {
        sound(w, &Value::string(d), ids)
    }
}
fn holders(w: &mut Writer, v: &Value, registry: &str, ids: &dyn Ids) -> Result<()> {
    if let Value::String(_) = v {
        let name = string(v)?;
        if let Some(tag) = name.strip_prefix('#') {
            w.varint(0);
            w.string(tag);
        } else {
            w.varint(2);
            w.varint(id(ids, registry, v)?);
        }
    } else {
        let values = list(v)?;
        w.varint(values.len() as i32 + 1);
        for v in values {
            w.varint(id(ids, registry, v)?);
        }
    }
    Ok(())
}
fn holder(
    w: &mut Writer,
    v: &Value,
    registry: &str,
    ids: &dyn Ids,
    direct: impl FnOnce(&mut Writer, &Value) -> Result<()>,
) -> Result<()> {
    if matches!(v, Value::String(_)) {
        w.varint(holder_id(ids, registry, v)?);
    } else {
        w.varint(0);
        direct(w, v)?;
    }
    Ok(())
}
const DYES: &[&str] = &[
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
const SLOT: &[&str] = &[
    "mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle",
];
const GROUP: &[&str] = &[
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
    "saddle",
];
use sunrise_core::use_animation::NAMES as ANIMATION;
fn patch_kind(name: &str, ids: &dyn Ids) -> Result<i32> {
    if let Some(kind) = builtin_component_id(name) {
        return Ok(kind);
    }
    ids.id("minecraft:data_component_type", name)
        .filter(|kind| crate::item::custom::name(*kind) == Some(name))
        .ok_or_else(|| err(format!("unknown or unnegotiated component {name}")))
}

/// Native templates preserve their signed count and their own patch, including transients.
/// A component named twice keeps its last entry, as in the JSON form.
pub fn template_from_parts(
    item: &str,
    count: i32,
    patch: &[(String, Option<Value>)],
    ids: &dyn Ids,
) -> Result<Stack> {
    template_from_parts_at_depth(item, count, patch, ids, 0)
}

fn template_from_parts_at_depth(
    item: &str,
    count: i32,
    patch: &[(String, Option<Value>)],
    ids: &dyn Ids,
    mut depth: u32,
) -> Result<Stack> {
    struct PendingList<'a, P> {
        parent: Stack,
        patch: P,
        kind: i32,
        remaining: std::slice::Iter<'a, Value>,
        children: Vec<Stack>,
    }

    // Typed stack lists need the caller's registry, not a structural stream read with
    // default IDs. Suspend the parent on the heap while building each child in order.
    // Each suspended list adds one enclosing component, shared by all its children.
    let mut parents = Vec::new();
    let mut stack = Stack::new(template_item_id(item, count, ids)?, count);
    let mut patch = effective_patch(patch);
    loop {
        let child = if let Some((name, value)) = patch.next() {
            let kind = patch_kind(name, ids)?;
            let Some(value) = value else {
                stack.removed.push(kind);
                continue;
            };
            if !matches!(kind, CHARGED_PROJECTILES | BUNDLE_CONTENTS) {
                let component = component_from_stream_value(kind, value, ids, depth)?;
                stack.added.push((kind, component));
                continue;
            }
            let mut remaining = stack_list(value, depth)?.iter();
            let Some(first) = remaining.next() else {
                stack.added.push((kind, Component::Stacks(Vec::new())));
                continue;
            };
            parents.push(PendingList {
                parent: stack,
                patch,
                kind,
                remaining,
                children: Vec::new(),
            });
            depth += 1;
            first
        } else {
            let Some(mut pending) = parents.pop() else {
                return Ok(stack);
            };
            pending.children.push(stack);
            if let Some(next) = pending.remaining.next() {
                parents.push(pending);
                next
            } else {
                stack = pending.parent;
                patch = pending.patch;
                stack
                    .added
                    .push((pending.kind, Component::Stacks(pending.children)));
                depth -= 1;
                continue;
            }
        };
        let t = canonical_template(child)?;
        stack = Stack::new(template_item_id(&t.item, t.count, ids)?, t.count);
        patch = effective_patch(&t.patch);
    }
}

fn template_item_id(item: &str, count: i32, ids: &dyn Ids) -> Result<i32> {
    if count == 0 || item == "minecraft:air" {
        return Err(err("empty native template"));
    }
    ids.id("minecraft:item", item)
        .filter(|id| *id > 0)
        .ok_or_else(|| err(format!("unknown or empty item {item}")))
}
fn canonical_template(v: &Value) -> Result<&Template> {
    match v {
        Value::Template(t) => Ok(t),
        _ => Err(err("expected canonical template")),
    }
}
fn template(w: &mut Writer, v: &Value, ids: &dyn Ids, depth: u32) -> Result<()> {
    template_body(w, canonical_template(v)?, ids, depth)
}
/// `depth` counts the components enclosing this template, as the stream reader counts them:
/// a component inside the template is encoded at the same depth and is the one that adds a level.
fn template_body(w: &mut Writer, t: &Template, ids: &dyn Ids, depth: u32) -> Result<()> {
    w.varint(template_item_id(&t.item, t.count, ids)?);
    w.varint(t.count);
    let (added, removed) = effective_patch(&t.patch).fold((0, 0), |(added, removed), (_, v)| {
        if v.is_some() {
            (added + 1, removed)
        } else {
            (added, removed + 1)
        }
    });
    w.varint(added);
    w.varint(removed);
    for (name, value) in effective_patch(&t.patch) {
        if let Some(value) = value {
            let kind = patch_kind(name, ids)?;
            w.varint(kind);
            encode(kind, value, w, ids, depth)?;
        }
    }
    for (name, value) in effective_patch(&t.patch) {
        if value.is_none() {
            w.varint(patch_kind(name, ids)?);
        }
    }
    Ok(())
}
/// Persistent input is normalized below protocol before registry admission.
pub fn component_from_tag(kind: i32, value: &Tag, ids: &dyn Ids) -> Result<Component> {
    let name = component_name(kind).ok_or_else(|| err("unknown component type"))?;
    let value = sunrise_world::component_value::source::from_component_tag_with(
        name,
        value,
        is_native_component,
    )
    .map_err(err)?;
    component_from_value(kind, &value, ids)
}
/// Encode retained effects and project newly materialized template effects for the caller.
pub fn component_from_admitted_tag(
    kind: i32,
    value: &Tag,
    ids: &dyn Ids,
    admitted: &dyn Fn(&Tag) -> Option<Admitted>,
    projected: &dyn Fn(&Admitted),
) -> Result<Component> {
    let name = component_name(kind).ok_or_else(|| err("unknown component type"))?;
    let value = sunrise_world::component_value::source::from_admitted_component_tag(
        name,
        value,
        is_native_component,
        admitted,
        projected,
    )
    .map_err(err)?;
    component_from_value(kind, &value, ids)
}

pub fn component_from_value(kind: i32, value: &Value, ids: &dyn Ids) -> Result<Component> {
    let name = component_name(kind).ok_or_else(|| err(format!("unknown component type {kind}")))?;
    let result = if matches!(kind, CHARGED_PROJECTILES | BUNDLE_CONTENTS) {
        stack_list(value, 0).and_then(|entries| {
            let mut stacks = Vec::new();
            for entry in entries {
                let t = canonical_template(entry)?;
                stacks.push(template_from_parts_at_depth(
                    &t.item, t.count, &t.patch, ids, 1,
                )?);
            }
            Ok(Component::Stacks(stacks))
        })
    } else {
        component_from_stream_value(kind, value, ids, 0)
    };
    result.map_err(|error| err(format!("{name}: {error}")))
}

fn stack_list(value: &Value, depth: u32) -> Result<&[Value]> {
    check_component_depth(depth)?;
    list(value)
}

fn component_from_stream_value(
    kind: i32,
    value: &Value,
    ids: &dyn Ids,
    depth: u32,
) -> Result<Component> {
    if let Some(name) = crate::item::custom::name(kind) {
        check_component_depth(depth)?;
        patch_kind(name, ids)?;
        return Ok(Component::Nbt(value.to_tag().map_err(err)?));
    }
    let mut w = Writer::new();
    encode(kind, value, &mut w, ids, depth)?;
    if !matches!(kind,0..=4|6|9..=13|17..=21|30|45|47|48|50|53|55..=58|63|65|69..=72|74|75|109)
        || matches!(
            (kind, value),
            (63, Value::Map(_)) | (109, Value::StreamOnly(_))
        )
        || kind == 74
            && list(value)?
                .iter()
                .any(|v| !matches!(field(v, "pattern"), Some(Value::String(_))))
    {
        return Ok(Component::Raw(w.data));
    }
    // Interpret the bytes with the pinned codec.
    let mut r = Reader::new(&w.data);
    let component = Component::read(kind, &mut r)?;
    if r.remaining() != 0 {
        return Err(err("trailing canonical component encoding"));
    }
    Ok(component)
}
fn effect(w: &mut Writer, v: &Value, ids: &dyn Ids) -> Result<()> {
    w.varint(id(ids, "minecraft:mob_effect", need(v, "id")?)?);
    effect_details(w, v, 0)
}
fn effect_details(w: &mut Writer, v: &Value, depth: u32) -> Result<()> {
    if depth >= 64 {
        return Err(err("effect nesting exceeds 64"));
    }
    w.varint(i(v, "amplifier", 0)? as u8 as i32);
    w.varint(i(v, "duration", 0)?);
    w.bool(b(v, "ambient", false)?);
    let particles = b(v, "show_particles", true)?;
    w.bool(particles);
    w.bool(b(v, "show_icon", particles)?);
    optional(w, field(v, "hidden_effect"), |w, v| {
        effect_details(w, v, depth + 1)
    })
}
fn consume_list(w: &mut Writer, effects: &[Value], ids: &dyn Ids) -> Result<()> {
    use crate::item::consume_effects::{REGISTRY, builtin_id};

    let mut wire = Writer::new();
    let mut count = 0;
    for value in effects {
        if let Value::RegisteredConsumeEffect(effect) = value {
            if let Some(id) = ids.id(REGISTRY, &effect.name) {
                wire.varint(id);
                wire.bytes(&effect.wire);
                count += 1;
            } else {
                log::warn!(
                    "Cannot synchronize consume effect {}: no peer registry ID",
                    effect.name
                );
            }
            continue;
        }
        let name = string(need(value, "type")?)?;
        let Some(kind) = builtin_id(&name) else {
            // Gameplay-only hook kinds have no stream form. Keep their siblings.
            log::warn!("Cannot synchronize consume effect {name}: no admitted Wire value");
            continue;
        };
        wire.varint(ids.id(REGISTRY, &name).unwrap_or(kind));
        match kind {
            0 => {
                sequence(&mut wire, list(need(value, "effects")?)?, |w, v| {
                    effect(w, v, ids)
                })?;
                wire.f32(f(value, "probability", 1.0)?);
            }
            1 => holders(
                &mut wire,
                need(value, "effects")?,
                "minecraft:mob_effect",
                ids,
            )?,
            2 => (),
            3 => {
                wire.f32(f(value, "diameter", 16.0)?);
                wire.bool(b(value, "directional_particles", true)?);
            }
            4 => sound(&mut wire, need(value, "sound")?, ids)?,
            _ => unreachable!("built-in consume-effect type"),
        }
        count += 1;
    }
    w.varint(count);
    w.bytes(&wire.data);
    Ok(())
}
fn explosion(w: &mut Writer, v: &Value) -> Result<()> {
    w.varint(named(
        need(v, "shape")?,
        &["small_ball", "large_ball", "star", "creeper", "burst"],
    )?);
    for k in ["colors", "fade_colors"] {
        sequence(w, values(v, k)?, |w, v| {
            w.i32(int(v)?);
            Ok(())
        })?;
    }
    w.bool(b(v, "has_trail", false)?);
    w.bool(b(v, "has_twinkle", false)?);
    Ok(())
}
fn material(w: &mut Writer, v: &Value) -> Result<()> {
    w.string(&string(need(v, "palette_id")?)?);
    nbt(w, need(v, "description")?)
}
fn pattern(w: &mut Writer, v: &Value) -> Result<()> {
    w.string(&string(need(v, "asset_id")?)?);
    nbt(w, need(v, "description")?)?;
    w.bool(b(v, "decal", false)?);
    Ok(())
}
fn typed(w: &mut Writer, v: &Value, registry: &str, ids: &dyn Ids) -> Result<()> {
    w.varint(id(ids, registry, need(v, "id")?)?);
    let mut tag = v.to_tag().map_err(err)?;
    let Tag::Compound(entries) = &mut tag else {
        return Err(err("typed data not a map"));
    };
    entries.remove(&sunrise_core::nbt::JavaString::from("id"));
    write_tag(w, &tag)
}
fn filterable(w: &mut Writer, v: &Value, text: bool) -> Result<()> {
    let raw = need(v, "raw")?;
    if text {
        nbt(w, raw)?;
    } else {
        w.string(&string(raw)?);
    }
    optional(w, field(v, "filtered"), |w, v| {
        if text {
            nbt(w, v)
        } else {
            w.string(&string(v)?);
            Ok(())
        }
    })
}
fn condition(w: &mut Writer, v: &Value) -> Result<()> {
    w.varint(int(need(v, "max_duration_ticks")?)?);
    w.f32(f(v, "min_speed", 0.0)?);
    w.f32(f(v, "min_relative_speed", 0.0)?);
    Ok(())
}
fn resolvable(w: &mut Writer, v: &Value, float_value: bool) -> Result<()> {
    let constant = !matches!(v, Value::String(_));
    w.bool(constant);
    if !constant {
        w.string(&string(v)?);
    } else if float_value {
        w.f32(float(v)?);
    } else {
        w.i32(int(v)?);
    }
    Ok(())
}
fn predicate(w: &mut Writer, v: &Value, ids: &dyn Ids, depth: u32) -> Result<()> {
    optional(w, field(v, "blocks"), |w, v| {
        holders(w, v, "minecraft:block", ids)
    })?;
    optional(w, field(v, "state"), |w, v| {
        let entries = map(v)?;
        w.varint(entries.len() as i32);
        for (k, v) in entries {
            w.string(&string(k)?);
            let exact = matches!(v, Value::String(_));
            w.bool(exact);
            if exact {
                w.string(&string(v)?);
            } else {
                for k in ["min", "max"] {
                    optional(w, field(v, k), |w, v| {
                        w.string(&string(v)?);
                        Ok(())
                    })?;
                }
            }
        }
        Ok(())
    })?;
    optional(w, field(v, "nbt"), |w, v| {
        let s = string(v)?;
        let (tag, n) = sunrise_core::nbt::parse_snbt(&s).map_err(err)?;
        if n != s.len() {
            return Err(err("trailing predicate SNBT"));
        }
        write_tag(w, &tag)
    })?;
    let exact = field(v, "components").map(map).transpose()?.unwrap_or(&[]);
    w.varint(exact.len() as i32);
    for (k, v) in exact {
        let name = string(k)?;
        let kind = patch_kind(&name, ids)?;
        id(ids, "minecraft:data_component_type", k)?;
        w.varint(kind);
        encode(kind, v, w, ids, depth + 1)?;
    }
    let partial = field(v, "predicates").map(map).transpose()?.unwrap_or(&[]);
    if partial.len() > 64 {
        return Err(err("too many component predicates"));
    }
    w.varint(partial.len() as i32);
    for (k, v) in partial {
        let name = string(k)?;
        if let Some(id) = ids.id("minecraft:data_component_predicate_type", &name) {
            w.bool(true);
            w.varint(id);
            nbt(w, v)?;
        } else {
            let kind = patch_kind(&name, ids)?;
            map(v).map_err(|error| err(format!("{name}: {error}")))?;
            w.bool(false);
            w.varint(kind);
            write_tag(w, &Tag::Compound(Default::default()))?;
        }
    }
    Ok(())
}
fn check_component_depth(depth: u32) -> Result<()> {
    if depth >= crate::item::MAX_NESTING {
        return Err(err(format!(
            "component nesting exceeds {}",
            crate::item::MAX_NESTING
        )));
    }
    Ok(())
}
fn encode(kind: i32, v: &Value, w: &mut Writer, ids: &dyn Ids, depth: u32) -> Result<()> {
    check_component_depth(depth)?;
    if let Some(name) = crate::item::custom::name(kind) {
        patch_kind(name, ids)?;
        return nbt(w, v);
    }
    // Keep the large leaf dispatcher out of recursive template and predicate frames.
    match kind {
        14 | 15 => {
            if matches!(v, Value::List(_)) {
                sequence(w, list(v)?, |w, v| predicate(w, v, ids, depth))?;
            } else {
                w.varint(1);
                predicate(w, v, ids, depth)?;
            }
        }
        25 | 80 => template(w, v, ids, depth + 1)?,
        51 | 52 => sequence(w, list(v)?, |w, v| template(w, v, ids, depth + 1))?,
        76 => {
            for k in ["back", "left", "right", "front"] {
                optional(w, field(v, k), |w, v| template(w, v, ids, depth + 1))?;
            }
        }
        77 => {
            let Value::Container(slots) = v else {
                return Err(err("expected dense canonical container"));
            };
            if slots.len() > 256 {
                return Err(err("container exceeds 256 slots"));
            }
            w.varint(slots.len() as i32);
            for slot in slots {
                w.bool(slot.is_some());
                if let Some(t) = slot {
                    template_body(w, t, ids, depth + 1)?;
                }
            }
        }
        _ => return encode_leaf(kind, v, w, ids),
    }
    Ok(())
}

#[inline(never)]
fn encode_leaf(kind: i32, v: &Value, w: &mut Writer, ids: &dyn Ids) -> Result<()> {
    match kind {
        // These codecs travel as their persistent NBT, including named registry references.
        0 | 22 | 49 | 59 | 61 | 68 | 81 | 82 => nbt(w, v)?,
        1..=3 | 19 | 42 | 48 | 50 | 65 => w.varint(int(v)?),
        4 | 20 | 34 | 120 => {
            map(v)?;
        }
        5 => {
            w.bool(b(v, "can_sprint", false)?);
            w.bool(b(v, "interact_vibrations", true)?);
            w.f32(f(v, "speed_multiplier", 0.2)?);
        }
        6 | 9 => nbt(w, v)?,
        7 | 54 => w.f32(float(v)?),
        8 | 43 | 88..=90 | 100..=107 | 112 | 113 | 117 => {
            let registry = match kind {
                8 => "damage_type",
                43 => "block_transformer",
                88 => "villager_type",
                89 => "wolf_variant",
                90 => "wolf_sound_variant",
                100 => "pig_variant",
                101 => "pig_sound_variant",
                102 => "cow_variant",
                103 => "cow_sound_variant",
                104 => "chicken_variant",
                105 => "chicken_sound_variant",
                106 => "zombie_nautilus_variant",
                107 => "frog_variant",
                112 => "cat_variant",
                113 => "cat_sound_variant",
                117 => "decorated_pot_pattern",
                _ => unreachable!(),
            };
            w.varint(id(ids, &format!("minecraft:{registry}"), v)?);
        }
        10 | 35 | 73 => w.string(&string(v)?),
        11 => sequence(w, list(v)?, nbt)?,
        12 => w.varint(named(v, &["common", "uncommon", "rare", "epic"])?),
        13 | 45 => {
            let entries = map(v)?;
            w.varint(entries.len() as i32);
            for (k, v) in entries {
                w.varint(id(ids, "minecraft:enchantment", k)?);
                w.varint(int(v)?);
            }
        }
        16 => sequence(w, list(v)?, |w, v| {
            w.varint(id(ids, "minecraft:attribute", need(v, "type")?)?);
            w.string(&string(need(v, "id")?)?);
            w.f64(number(need(v, "amount")?)?);
            w.varint(named(
                need(v, "operation")?,
                &["add_value", "add_multiplied_base", "add_multiplied_total"],
            )?);
            w.varint(named_default(v, "slot", 0, GROUP)?);
            if let Some(display) = field(v, "display") {
                let kind = named(need(display, "type")?, &["default", "hidden", "override"])?;
                w.varint(kind);
                if kind == 2 {
                    nbt(w, need(display, "value")?)?;
                }
            } else {
                w.varint(0);
            }
            Ok(())
        })?,
        17 => {
            sequence(w, values(v, "floats")?, |w, v| {
                w.f32(float(v)?);
                Ok(())
            })?;
            sequence(w, values(v, "flags")?, |w, v| {
                w.bool(boolean(v)?);
                Ok(())
            })?;
            sequence(w, values(v, "strings")?, |w, v| {
                w.string(&string(v)?);
                Ok(())
            })?;
            sequence(w, values(v, "colors")?, |w, v| {
                w.i32(int(v)?);
                Ok(())
            })?;
        }
        18 => {
            w.bool(b(v, "hide_tooltip", false)?);
            sequence(w, values(v, "hidden_components")?, |w, v| {
                let name = string(v)?;
                w.varint(patch_kind(&name, ids)?);
                Ok(())
            })?;
        }
        21 => w.bool(boolean(v)?),
        23 => {
            w.varint(int(need(v, "nutrition")?)?);
            w.f32(float(need(v, "saturation")?)?);
            w.bool(b(v, "can_always_eat", false)?);
        }
        24 => {
            w.f32(f(v, "consume_seconds", 1.6)?);
            w.varint(named_default(v, "animation", 1, &ANIMATION)?);
            sound_default(w, v, "sound", "minecraft:entity.generic.eat", ids)?;
            w.bool(b(v, "has_consume_particles", true)?);
            consume_list(w, values(v, "on_consume_effects")?, ids)?;
        }
        26 => {
            w.f32(float(need(v, "seconds")?)?);
            optional(w, field(v, "cooldown_group"), |w, v| {
                w.string(&string(v)?);
                Ok(())
            })?;
        }
        27 => holders(w, need(v, "types")?, "minecraft:damage_type", ids)?,
        33 => holders(w, need(v, "items")?, "minecraft:item", ids)?,
        28 => {
            sequence(w, list(need(v, "rules")?)?, |w, v| {
                holders(w, need(v, "blocks")?, "minecraft:block", ids)?;
                optional(w, field(v, "speed"), |w, v| {
                    w.f32(float(v)?);
                    Ok(())
                })?;
                optional(w, field(v, "correct_for_drops"), |w, v| {
                    w.bool(boolean(v)?);
                    Ok(())
                })
            })?;
            w.f32(f(v, "default_mining_speed", 1.0)?);
            w.varint(i(v, "damage_per_block", 1)?);
            w.bool(b(v, "can_destroy_blocks_in_creative", true)?);
        }
        29 => {
            w.varint(i(v, "item_damage_per_attack", 1)?);
            w.f32(f(v, "disable_blocking_for_seconds", 0.0)?);
        }
        30 => {
            for (k, d) in [
                ("min_reach", 0.0),
                ("max_reach", 3.0),
                ("min_creative_reach", 0.0),
                ("max_creative_reach", 5.0),
                ("hitbox_margin", 0.3),
                ("mob_factor", 1.0),
            ] {
                w.f32(f(v, k, d)?);
            }
        }
        31 | 44 => w.varint(int(need(
            v,
            if kind == 31 { "value" } else { "nutrition" },
        )?)?),
        32 => {
            w.varint(named(need(v, "slot")?, SLOT)?);
            sound_default(
                w,
                v,
                "equip_sound",
                "minecraft:item.armor.equip_generic",
                ids,
            )?;
            for k in ["asset_id", "camera_overlay"] {
                optional(w, field(v, k), |w, v| {
                    w.string(&string(v)?);
                    Ok(())
                })?;
            }
            optional(w, field(v, "allowed_entities"), |w, v| {
                holders(w, v, "minecraft:entity_type", ids)
            })?;
            for (k, d) in [
                ("dispensable", true),
                ("swappable", true),
                ("damage_on_hurt", true),
                ("equip_on_interact", false),
                ("can_be_sheared", false),
            ] {
                w.bool(b(v, k, d)?);
            }
            sound_default(w, v, "shearing_sound", "minecraft:item.shears.snip", ids)?;
        }
        36 => consume_list(w, values(v, "death_effects")?, ids)?,
        37 => blocks(w, v, ids)?,
        38 => {
            w.bool(b(v, "deals_knockback", true)?);
            w.bool(b(v, "dismounts", false)?);
            for k in ["sound", "hit_sound"] {
                optional(w, field(v, k), |w, v| sound(w, v, ids))?;
            }
        }
        39 => {
            w.varint(i(v, "contact_cooldown_ticks", 10)?);
            w.varint(i(v, "delay_ticks", 0)?);
            for k in [
                "dismount_conditions",
                "knockback_conditions",
                "damage_conditions",
            ] {
                optional(w, field(v, k), condition)?;
            }
            w.f32(f(v, "forward_movement", 0.0)?);
            w.f32(f(v, "damage_multiplier", 1.0)?);
            for k in ["sound", "hit_sound"] {
                optional(w, field(v, k), |w, v| sound(w, v, ids))?;
            }
        }
        40 | 41 => {
            w.varint(named_default(v, "type", 1, &["none", "whack", "stab"])?);
            w.varint(i(v, "duration", 6)?);
        }
        46 | 75 | 91 | 96 | 97 | 114..=116 | 121 => w.varint(named(v, DYES)?),
        47 => w.i32(int(v)?),
        53 => {
            optional(w, field(v, "potion"), |w, v| {
                w.varint(id(ids, "minecraft:potion", v)?);
                Ok(())
            })?;
            optional(w, field(v, "custom_color"), |w, v| {
                w.i32(int(v)?);
                Ok(())
            })?;
            sequence(w, values(v, "custom_effects")?, |w, v| effect(w, v, ids))?;
            optional(w, field(v, "custom_name"), |w, v| {
                w.string(&string(v)?);
                Ok(())
            })?;
        }
        55 => sequence(w, list(v)?, |w, v| {
            w.varint(id(ids, "minecraft:mob_effect", need(v, "id")?)?);
            w.varint(i(v, "duration", 160)?);
            Ok(())
        })?,
        56 => sequence(w, values(v, "pages")?, |w, v| filterable(w, v, false))?,
        57 => {
            filterable(w, need(v, "title")?, false)?;
            w.string(&string(need(v, "author")?)?);
            w.varint(i(v, "generation", 0)?);
            sequence(w, values(v, "pages")?, |w, v| filterable(w, v, true))?;
            w.bool(b(v, "resolved", false)?);
        }
        58 => {
            holder(
                w,
                need(v, "material")?,
                "minecraft:trim_material",
                ids,
                material,
            )?;
            holder(
                w,
                need(v, "pattern")?,
                "minecraft:trim_pattern",
                ids,
                pattern,
            )?;
        }
        60 | 62 => typed(
            w,
            v,
            if kind == 60 {
                "minecraft:entity_type"
            } else {
                "minecraft:block_entity_type"
            },
            ids,
        )?,
        63 => holder(w, v, "minecraft:instrument", ids, |w, v| {
            sound(w, need(v, "sound_event")?, ids)?;
            w.f32(float(need(v, "use_duration")?)?);
            w.f32(float(need(v, "range")?)?);
            w.varint(i(v, "durability_damage", 0)?);
            nbt(w, need(v, "description")?)
        })?,
        64 => holder(w, v, "minecraft:trim_material", ids, material)?,
        66 => {
            if let Value::StreamOnly(v) = v {
                w.varint(0);
                sound(w, need(v, "sound_event")?, ids)?;
                nbt(w, need(v, "description")?)?;
                w.f32(float(need(v, "length_in_seconds")?)?);
                w.varint(int(need(v, "comparator_output")?)?);
            } else {
                w.varint(holder_id(ids, "minecraft:jukebox_song", v)?);
            }
        }
        67 => holders(w, v, "minecraft:banner_pattern", ids)?,
        69 => {
            optional(w, field(v, "target"), |w, v| {
                w.string(&string(need(v, "dimension")?)?);
                let pos = need(v, "pos")?;
                let coords = match pos {
                    Value::IntArray(v) => v.clone(),
                    _ => list(pos)?.iter().map(int).collect::<Result<Vec<_>>>()?,
                };
                let [x, y, z] = coords.as_slice() else {
                    return Err(err("invalid block position"));
                };
                w.i64(
                    ((*x as i64 & 0x3ffffff) << 38)
                        | ((*z as i64 & 0x3ffffff) << 12)
                        | (*y as i64 & 0xfff),
                );
                Ok(())
            })?;
            w.bool(b(v, "tracked", true)?);
        }
        70 => explosion(w, v)?,
        71 => {
            w.varint(i(v, "flight_duration", 0)? as u8 as i32);
            sequence(w, values(v, "explosions")?, explosion)?;
        }
        72 => profile(w, v)?,
        74 => sequence(w, list(v)?, |w, v| {
            holder(
                w,
                need(v, "pattern")?,
                "minecraft:banner_pattern",
                ids,
                |w, v| {
                    w.string(&string(need(v, "asset_id")?)?);
                    w.string(&string(need(v, "translation_key")?)?);
                    Ok(())
                },
            )?;
            w.varint(named(need(v, "color")?, DYES)?);
            Ok(())
        })?,
        78 => {
            let entries = map(v)?;
            w.varint(entries.len() as i32);
            for (k, v) in entries {
                w.string(&string(k)?);
                w.string(&string(v)?);
            }
        }
        79 => sequence(w, list(v)?, |w, v| {
            typed(w, need(v, "entity_data")?, "minecraft:entity_type", ids)?;
            w.varint(int(need(v, "ticks_in_hive")?)?);
            w.varint(int(need(v, "min_ticks_in_hive")?)?);
            Ok(())
        })?,
        83 => sound(w, v, ids)?,
        84 => resolvable(w, need(v, "layers")?, false)?,
        85 | 86 => {
            resolvable(
                w,
                need(v, if kind == 85 { "burn_time" } else { "uses" })?,
                false,
            )?;
            resolvable(w, need(v, "speed_multiplier")?, true)?;
        }
        87 => {
            holders(
                w,
                need(v, "targeting_entity_types")?,
                "minecraft:entity_type",
                ids,
            )?;
            w.f32(float(need(v, "visibility")?)?);
        }
        92 | 93 | 94 | 98 | 99 | 108 | 110 | 111 => {
            let table: &[&str] = match kind {
                92 => &["red", "snow"],
                93 => &["small", "medium", "large"],
                94 => &["red_blue", "blue", "green", "yellow_blue", "gray"],
                98 => &["red", "brown"],
                99 => &["brown", "white", "black", "white_splotched", "gold", "salt"],
                108 => &[
                    "white",
                    "creamy",
                    "chestnut",
                    "brown",
                    "black",
                    "gray",
                    "dark_brown",
                ],
                110 => &["creamy", "white", "brown", "gray"],
                111 => &["lucy", "wild", "gold", "cyan", "blue"],
                _ => unreachable!(),
            };
            w.varint(if kind == 99 && string(v)? == "evil" {
                99
            } else {
                named(v, table)?
            });
        }
        95 => {
            let name = string(v)?;
            let names = [
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
            ];
            let n = names
                .iter()
                .position(|n| *n == name)
                .ok_or_else(|| err("unknown tropical fish pattern"))?;
            w.varint(if n < 6 {
                (n as i32) << 8
            } else {
                1 | (((n - 6) as i32) << 8)
            });
        }
        109 => {
            if let Value::StreamOnly(v) = v {
                w.varint(0);
                w.varint(int(need(v, "width")?)?);
                w.varint(int(need(v, "height")?)?);
                w.string(&string(need(v, "asset_id")?)?);
                for k in ["title", "author"] {
                    optional(w, field(v, k), nbt)?;
                }
            } else {
                w.varint(holder_id(ids, "minecraft:painting_variant", v)?);
            }
        }
        118 | 119 => {
            let messages = list(need(v, "messages")?)?;
            if messages.len() != 4 {
                return Err(err("sign requires four messages"));
            }
            for v in messages {
                nbt(w, v)?;
            }
            optional(w, field(v, "filtered_messages"), |w, v| {
                let messages = list(v)?;
                if messages.len() != 4 {
                    return Err(err("sign requires four filtered messages"));
                }
                for v in messages {
                    nbt(w, v)?;
                }
                Ok(())
            })?;
            w.varint(named(need(v, "color")?, DYES)?);
            w.bool(boolean(need(v, "has_glowing_text")?)?);
        }
        _ => return Err(err(format!("unknown component codec {kind}"))),
    }
    Ok(())
}
fn profile(w: &mut Writer, v: &Value) -> Result<()> {
    w.u8(0);
    optional(w, field(v, "name"), |w, v| {
        w.string(&string(v)?);
        Ok(())
    })?;
    optional(w, field(v, "id"), |w, v| {
        let parts = if let Value::IntArray(v) = v {
            v.clone()
        } else {
            list(v)?.iter().map(int).collect::<Result<_>>()?
        };
        if parts.len() != 4 {
            return Err(err("invalid profile UUID"));
        }
        let id = parts
            .iter()
            .fold(0u128, |n, v| (n << 32) | u128::from(*v as u32));
        w.uuid(id);
        Ok(())
    })?;
    sequence(w, values(v, "properties")?, |w, v| {
        w.string(&string(need(v, "name")?)?);
        w.string(&string(need(v, "value")?)?);
        optional(w, field(v, "signature"), |w, v| {
            w.string(&string(v)?);
            Ok(())
        })
    })?;
    for k in ["texture", "cape", "elytra"] {
        optional(w, field(v, k), |w, v| {
            w.string(&string(v)?);
            Ok(())
        })?;
    }
    optional(w, field(v, "model"), |w, v| {
        w.bool(match string(v)?.as_str() {
            "slim" => true,
            "wide" => false,
            _ => return Err(err("unknown skin model")),
        });
        Ok(())
    })
}
fn blocks(w: &mut Writer, v: &Value, ids: &dyn Ids) -> Result<()> {
    w.f32(f(v, "block_delay_seconds", 0.0)?);
    w.f32(f(v, "disable_cooldown_scale", 1.0)?);
    if let Some(reductions) = field(v, "damage_reductions") {
        sequence(w, list(reductions)?, |w, v| {
            w.f32(f(v, "horizontal_blocking_angle", 90.0)?);
            optional(w, field(v, "type"), |w, v| {
                holders(w, v, "minecraft:damage_type", ids)
            })?;
            w.f32(float(need(v, "base")?)?);
            w.f32(float(need(v, "factor")?)?);
            Ok(())
        })?;
    } else {
        w.varint(1);
        w.f32(90.0);
        w.bool(false);
        w.f32(0.0);
        w.f32(1.0);
    }
    if let Some(item) = field(v, "item_damage") {
        for k in ["threshold", "base", "factor"] {
            w.f32(float(need(item, k)?)?);
        }
    } else {
        w.f32(1.0);
        w.f32(0.0);
        w.f32(1.0);
    }
    optional(w, field(v, "bypassed_by"), |w, v| {
        holders(w, v, "minecraft:damage_type", ids)
    })?;
    for k in ["block_sound", "disabled_sound"] {
        optional(w, field(v, k), |w, v| sound(w, v, ids))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{
        NoNames,
        persistent::{NameLookup, component_value},
    };

    fn registry_id(registry: &str, name: &str) -> Option<i32> {
        if registry != "minecraft:attribute" {
            return None;
        }
        ["minecraft:armor", "minecraft:luck", "minecraft:scale"]
            .iter()
            .position(|n| *n == name)
            .map(|n| n as i32)
    }
    fn registry_name(registry: &str, id: i32) -> Option<String> {
        if registry != "minecraft:attribute" {
            return None;
        }
        ["minecraft:armor", "minecraft:luck", "minecraft:scale"]
            .get(usize::try_from(id).ok()?)
            .map(|s| (*s).into())
    }
    fn entry(w: &mut Writer, attribute: i32, id: &str, amount: f64, operation: i32, slot: i32) {
        w.varint(attribute);
        w.string(id);
        w.f64(amount);
        w.varint(operation);
        w.varint(slot);
    }
    fn parse(s: &str) -> Tag {
        sunrise_core::nbt::parse_snbt(s).unwrap().0
    }

    /// Migrated from the retired client writer: all display variants and slot defaults.
    #[test]
    fn modifiers_keep_the_complete_saved_list_and_stream() {
        let mut w = Writer::new();
        w.varint(3);
        entry(&mut w, 0, "minecraft:armor.chestplate", 6.0, 0, 6);
        w.varint(0);
        entry(&mut w, 1, "test:charm", 0.25, 2, 0);
        w.varint(1);
        entry(&mut w, 2, "test:tall", -0.5, 1, 8);
        w.varint(2);
        w.nbt(&Tag::String("Taller".into()));
        let expected = parse(
            r#"[{type:"minecraft:armor",id:"minecraft:armor.chestplate",amount:6.0d,operation:"add_value",slot:"chest"},{type:"minecraft:luck",id:"test:charm",amount:0.25d,operation:"add_multiplied_total",display:{type:"hidden"}},{type:"minecraft:scale",id:"test:tall",amount:-0.5d,operation:"add_multiplied_base",slot:"armor",display:{type:"override",value:"Taller"}}]"#,
        );
        let value = component_value(
            16,
            &Component::Raw(w.data.clone()),
            &NameLookup::new(&registry_name),
        )
        .unwrap();
        assert_eq!(value.to_tag().unwrap(), expected);
        assert_eq!(
            component_from_value(16, &value, &registry_id).unwrap(),
            Component::Raw(w.data)
        );
        let saved = parse(
            r#"[{type:"armor",id:"plate",amount:3,operation:"add_value",display:{type:"default"}}]"#,
        );
        let mut w = Writer::new();
        w.varint(1);
        entry(&mut w, 0, "minecraft:plate", 3.0, 0, 0);
        w.varint(0);
        assert_eq!(
            component_from_tag(16, &saved, &registry_id).unwrap(),
            Component::Raw(w.data)
        );
    }
    #[test]
    fn invalid_modifier_values_refuse_the_whole_component() {
        let one = |attribute, operation, slot, display| {
            let mut w = Writer::new();
            w.varint(1);
            entry(&mut w, attribute, "test:x", 1.0, operation, slot);
            w.varint(display);
            w.data
        };
        let names = NameLookup::new(&registry_name);
        for bytes in [
            one(3, 0, 0, 0),
            one(0, 3, 0, 0),
            one(0, 0, 11, 0),
            one(0, 0, 0, 3),
        ] {
            assert!(component_value(16, &Component::Raw(bytes), &names).is_err());
        }
        let mut trailing = one(0, 0, 0, 0);
        trailing.push(0);
        assert!(component_value(16, &Component::Raw(trailing), &names).is_err());
        assert!(component_value(16, &Component::Raw(one(0, 0, 0, 0)), &NoNames).is_err());
        for source in [
            r#"[{type:"gravity",id:"test:x",amount:1d,operation:"add_value",slot:"any"}]"#,
            r#"[{type:"armor",id:"test:x",amount:1d,operation:"multiply"}]"#,
            r#"[{type:"armor",id:"test:x",amount:1d,operation:"add_value",slot:"pocket"}]"#,
            r#"[{type:"armor",id:"test:x",amount:1d,operation:"add_value",display:{type:"loud"}}]"#,
            r#"[{type:"armor",id:"test:x",amount:1d,operation:"add_value",display:{type:"override"}}]"#,
            r#"{modifiers:[]}"#,
            r#"[{type:"armor",id:"test:first",amount:1d,operation:"add_value"},{type:"gravity",id:"test:second",amount:1d,operation:"add_value"}]"#,
        ] {
            assert!(
                component_from_tag(16, &parse(source), &registry_id).is_err(),
                "{source}"
            );
        }
    }
    #[test]
    fn negative_kinetic_sound_holders_have_no_source_projection() {
        for id in [-1, i32::MIN] {
            let mut w = Writer::new();
            w.data
                .extend([10, 0, 0, 0, 0, 0, 0, 0, 0, 63, 128, 0, 0, 1]);
            w.varint(id);
            w.bool(false);
            assert!(component_value(39, &Component::Raw(w.data), &NoNames).is_err());
        }
    }
    #[test]
    fn unsigned_persistent_effect_and_flight_bytes_encode_unsigned_varints() {
        let ids = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:mob_effect", "minecraft:speed") => Some(0),
            _ => None,
        };
        let potion = component_from_tag(
            53,
            &parse(r#"{custom_effects:[{id:"speed",amplifier:-1b}]}"#),
            &ids,
        )
        .unwrap();
        let Component::Potion { rest, .. } = potion else {
            panic!("potion")
        };
        let mut r = Reader::new(&rest);
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.varint().unwrap(), 0);
        assert_eq!(r.varint().unwrap(), 255);
        let fireworks = component_from_tag(71, &parse("{flight_duration:-1b}"), &ids).unwrap();
        assert!(matches!(
            fireworks,
            Component::Fireworks { flight: 255, .. }
        ));
    }
}
