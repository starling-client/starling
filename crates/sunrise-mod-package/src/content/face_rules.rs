//! JSON parsing and load-time validation for block neighbor face rules (`skip_rendering`,
//! `hides_neighbor_face`, `supports_external_face_hiding`).
use super::{namespaced_id, property_bool};
use mod_vocab::{FaceDirections, FaceFacts, FaceNeighbor, FaceRule};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::geometry::Direction;

fn parse_direction(name: &str, key: &str) -> Result<Direction, String> {
    match name {
        "down" => Ok(Direction::Down),
        "up" => Ok(Direction::Up),
        "north" => Ok(Direction::North),
        "south" => Ok(Direction::South),
        "west" => Ok(Direction::West),
        "east" => Ok(Direction::East),
        _ => Err(format!("{key}.faces: unknown direction {name:?}")),
    }
}

fn parse_state_map(value: &Value, context: &str) -> Result<BTreeMap<String, String>, String> {
    let Some(obj) = value.as_object() else {
        return Err(format!("{context}: expected an object of property values"));
    };
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if k.is_empty() {
            return Err(format!("{context}: property name must not be empty"));
        }
        let Some(val) = v.as_str() else {
            return Err(format!("{context}.{k}: expected a string property value"));
        };
        if val.is_empty() {
            return Err(format!("{context}.{k}: property value must not be empty"));
        }
        out.insert(k.clone(), val.to_owned());
    }
    Ok(out)
}

fn parse_neighbor(value: &Value, key: &str) -> Result<FaceNeighbor, String> {
    match value {
        Value::Null => Ok(FaceNeighbor::Any),
        Value::String(s) => {
            if s == "self" {
                Ok(FaceNeighbor::SameBlock {
                    state: BTreeMap::new(),
                })
            } else if let Some(tag) = s.strip_prefix('#') {
                if !namespaced_id(tag) {
                    return Err(format!(
                        "{key}.neighbor: invalid block tag {s:?} (expected #namespace:path)"
                    ));
                }
                Ok(FaceNeighbor::Tag {
                    id: tag.to_owned(),
                    state: BTreeMap::new(),
                })
            } else {
                if !namespaced_id(s) {
                    return Err(format!(
                        "{key}.neighbor: invalid block id {s:?} (expected namespace:path, #tag or \"self\")"
                    ));
                }
                Ok(FaceNeighbor::Block {
                    id: s.clone(),
                    state: BTreeMap::new(),
                })
            }
        }
        Value::Object(obj) => {
            for k in obj.keys() {
                if !matches!(k.as_str(), "block" | "tag" | "state") {
                    return Err(format!("{key}.neighbor: unknown key {k:?}"));
                }
            }
            let state = match obj.get("state") {
                None | Some(Value::Null) => BTreeMap::new(),
                Some(v) => parse_state_map(v, &format!("{key}.neighbor.state"))?,
            };
            match (obj.get("block"), obj.get("tag")) {
                (Some(b), None) => {
                    let Some(id) = b.as_str() else {
                        return Err(format!("{key}.neighbor.block: expected a string"));
                    };
                    if id == "self" {
                        Ok(FaceNeighbor::SameBlock { state })
                    } else {
                        if !namespaced_id(id) {
                            return Err(format!(
                                "{key}.neighbor.block: invalid block id {id:?} (expected namespace:path or \"self\")"
                            ));
                        }
                        Ok(FaceNeighbor::Block {
                            id: id.to_owned(),
                            state,
                        })
                    }
                }
                (None, Some(t)) => {
                    let Some(raw) = t.as_str() else {
                        return Err(format!("{key}.neighbor.tag: expected a string"));
                    };
                    let id = raw.strip_prefix('#').unwrap_or(raw);
                    if !namespaced_id(id) {
                        return Err(format!(
                            "{key}.neighbor.tag: invalid block tag {raw:?} (expected namespace:path)"
                        ));
                    }
                    Ok(FaceNeighbor::Tag {
                        id: id.to_owned(),
                        state,
                    })
                }
                (Some(_), Some(_)) => Err(format!(
                    "{key}.neighbor: set either \"block\" or \"tag\", not both"
                )),
                (None, None) => Err(format!(
                    "{key}.neighbor: object form requires \"block\" or \"tag\""
                )),
            }
        }
        _ => Err(format!(
            "{key}.neighbor: expected a block id, #tag, \"self\" or {{block|tag, state}}"
        )),
    }
}

fn parse_faces(value: &Value, key: &str) -> Result<FaceDirections, String> {
    match value {
        Value::Null => Ok(FaceDirections::All),
        Value::String(s) => match s.as_str() {
            "all" => Ok(FaceDirections::All),
            "horizontal" => Ok(FaceDirections::Horizontal),
            "vertical" => Ok(FaceDirections::Vertical),
            _ => Err(format!(
                "{key}.faces: unknown face set {s:?} (expected \"all\", \"horizontal\", \"vertical\" or a list of directions)"
            )),
        },
        Value::Array(items) => {
            let mut dirs = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return Err(format!("{key}.faces: expected direction names"));
                };
                dirs.push(parse_direction(name, key)?);
            }
            Ok(FaceDirections::Only(dirs))
        }
        _ => Err(format!(
            "{key}.faces: expected \"all\", \"horizontal\", \"vertical\" or a list of directions"
        )),
    }
}

fn parse_facts(value: &Value, key: &str) -> Result<FaceFacts, String> {
    let Some(obj) = value.as_object() else {
        return Err(format!("{key}.facts: expected an object of boolean facts"));
    };
    let mut facts = FaceFacts::default();
    for (k, v) in obj {
        let Some(b) = v.as_bool() else {
            return Err(format!("{key}.facts.{k}: expected a boolean"));
        };
        match k.as_str() {
            "is_air" => facts.is_air = Some(b),
            "can_occlude" => facts.can_occlude = Some(b),
            "is_solid_render" => facts.is_solid_render = Some(b),
            "liquid" => facts.liquid = Some(b),
            _ => return Err(format!("{key}.facts: unknown fact {k:?}")),
        }
    }
    Ok(facts)
}

fn parse_face_rule(value: &Value, key: &str) -> Result<FaceRule, String> {
    let Some(obj) = value.as_object() else {
        return Err(format!("{key}: each face rule must be an object"));
    };
    for k in obj.keys() {
        if !matches!(
            k.as_str(),
            "neighbor" | "state" | "same" | "faces" | "facts"
        ) {
            return Err(format!("{key}: unknown face rule key {k:?}"));
        }
    }
    let neighbor = match obj.get("neighbor") {
        None => FaceNeighbor::Any,
        Some(v) => parse_neighbor(v, key)?,
    };
    let state = match obj.get("state") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(v) => parse_state_map(v, &format!("{key}.state"))?,
    };
    let same = match obj.get("same") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut props = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return Err(format!("{key}.same: expected property names"));
                };
                if name.is_empty() {
                    return Err(format!("{key}.same: property name must not be empty"));
                }
                props.push(name.to_owned());
            }
            props
        }
        Some(_) => return Err(format!("{key}.same: expected an array of property names")),
    };
    let faces = match obj.get("faces") {
        None => FaceDirections::All,
        Some(v) => parse_faces(v, key)?,
    };
    let facts = match obj.get("facts") {
        None | Some(Value::Null) => FaceFacts::default(),
        Some(v) => parse_facts(v, key)?,
    };
    Ok(FaceRule {
        neighbor,
        state,
        same,
        faces,
        facts,
    })
}

/// Parses a JSON array of [`FaceRule`]s (used by content block JSON and `render-definitions.json`).
pub fn parse_face_rules(value: &Value, key: &str) -> Result<Vec<FaceRule>, String> {
    let Some(items) = value.as_array() else {
        return Err(format!("{key}: expected an array of face rules"));
    };
    items.iter().map(|v| parse_face_rule(v, key)).collect()
}

pub(super) fn deserialize_skip_rendering<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<FaceRule>>, D::Error> {
    Option::<Value>::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("skip_rendering: {e}")))?
        .map(|v| parse_face_rules(&v, "skip_rendering").map_err(serde::de::Error::custom))
        .transpose()
}

pub(super) fn deserialize_hides_neighbor_face<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<FaceRule>, D::Error> {
    let value = Value::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("hides_neighbor_face: {e}")))?;
    parse_face_rules(&value, "hides_neighbor_face").map_err(serde::de::Error::custom)
}

pub(super) fn deserialize_supports_external_face_hiding<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<bool>, D::Error> {
    property_bool(deserializer, "supports_external_face_hiding")
}

/// Validates typed or JSON-parsed [`FaceRule`]s at block admission.
pub(super) fn check_face_rules(key: &str, rules: &[FaceRule]) -> Result<(), String> {
    let check_state = |map: &BTreeMap<String, String>, ctx: &str| -> Result<(), String> {
        for (k, v) in map {
            if k.is_empty() {
                return Err(format!("{ctx}: property name must not be empty"));
            }
            if v.is_empty() {
                return Err(format!("{ctx}.{k}: property value must not be empty"));
            }
        }
        Ok(())
    };
    for rule in rules {
        match &rule.neighbor {
            FaceNeighbor::Any => {}
            FaceNeighbor::SameBlock { state } => {
                check_state(state, &format!("{key}.neighbor.state"))?;
            }
            FaceNeighbor::Block { id, state } => {
                if !namespaced_id(id) {
                    return Err(format!(
                        "{key}.neighbor: invalid block id {id:?} (expected namespace:path)"
                    ));
                }
                check_state(state, &format!("{key}.neighbor.state"))?;
            }
            FaceNeighbor::Tag { id, state } => {
                if !namespaced_id(id) {
                    return Err(format!(
                        "{key}.neighbor: invalid block tag {id:?} (expected namespace:path without '#')"
                    ));
                }
                check_state(state, &format!("{key}.neighbor.state"))?;
            }
        }
        check_state(&rule.state, &format!("{key}.state"))?;
        for prop in &rule.same {
            if prop.is_empty() {
                return Err(format!("{key}.same: property name must not be empty"));
            }
        }
    }
    Ok(())
}

/// Refuses a rule whose `state` or `same` names a property the declaring block does not have, or
/// a `state` value outside that property's values. `known` maps each property of the block, the
/// template's and the declared ones, to its values. Neighbour ids and tags are not checked: they
/// may belong to optional mods.
pub fn check_face_rule_properties(
    key: &str,
    rules: &[FaceRule],
    known: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    for rule in rules {
        for (name, value) in &rule.state {
            let Some(values) = known.get(name) else {
                return Err(format!(
                    "{key}.state.{name}: the block has no such property"
                ));
            };
            if !values.contains(value) {
                return Err(format!(
                    "{key}.state.{name}: {value:?} is not a value of the property"
                ));
            }
        }
        for name in &rule.same {
            if !known.contains_key(name) {
                return Err(format!("{key}.same: the block has no property {name:?}"));
            }
        }
    }
    Ok(())
}
