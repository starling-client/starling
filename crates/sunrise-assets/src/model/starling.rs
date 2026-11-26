//! Starling's part of model.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A `neoforge:composite` (or `forge:composite`) model as one plain model: its own elements, then
/// each child's, the children's texture slots renamed `<child>_<slot>` so two children's `#0`
/// stay apart (a slot a child does not set is the composite's). Only children written out in
/// place; a child naming a parent model leaves the model as it is (None).
pub(super) fn flatten_composite(json: &str) -> Option<String> {
    use serde_json::Value;
    let mut model: serde_json::Map<String, Value> = serde_json::from_str(json).ok()?;
    let loader = model.get("loader")?.as_str()?;
    if !loader.ends_with(":composite") {
        return None;
    }
    let children = model.remove("children")?;
    model.remove("loader");
    let mut elements = match model.remove("elements") {
        Some(Value::Array(own)) => own,
        _ => Vec::new(),
    };
    let mut textures = match model.remove("textures") {
        Some(Value::Object(own)) => own,
        _ => serde_json::Map::new(),
    };
    for (name, child) in children.as_object()? {
        let child = child.as_object()?;
        if child.contains_key("parent") {
            return None;
        }
        let own = child.get("textures").and_then(Value::as_object);
        let renamed = |slot: &str| match own {
            Some(own) if own.contains_key(slot) => format!("{name}_{slot}"),
            _ => slot.to_owned(),
        };
        for (slot, value) in own.into_iter().flatten() {
            let value = match value.as_str().and_then(|v| v.strip_prefix('#')) {
                Some(target) => Value::String(format!("#{}", renamed(target))),
                None => value.clone(),
            };
            textures.insert(renamed(slot), value);
        }
        for element in child
            .get("elements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut element = element.clone();
            if let Some(faces) = element.get_mut("faces").and_then(Value::as_object_mut) {
                for face in faces.values_mut() {
                    let target = face
                        .get("texture")
                        .and_then(Value::as_str)
                        .and_then(|t| t.strip_prefix('#'))
                        .map(renamed);
                    if let (Some(target), Some(face)) = (target, face.as_object_mut()) {
                        face.insert("texture".into(), Value::String(format!("#{target}")));
                    }
                }
            }
            elements.push(element);
        }
    }
    model.insert("textures".into(), Value::Object(textures));
    model.insert("elements".into(), Value::Array(elements));
    serde_json::to_string(&model).ok()
}
