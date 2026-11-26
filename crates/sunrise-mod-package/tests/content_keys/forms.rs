//! Serde buffers internally tagged and untagged enums and replays from its own
//! deserializer; the manifest's custom policy visitors likewise accept shapes
//! without naming variants. Decode those paths with the real enum, then record
//! the chosen form and its fields.
//! Keep the matches exhaustive so a new Rust variant needs a coverage rule.

use crate::{Input, Probe};
use serde::Deserialize;
use serde_json::Value;
use sunrise_mod_package::{
    content::{
        GateSound, RandomTicks, StateProperty,
        entity_sounds::{Mute, SoundOverride, StepOverride},
    },
    manifest::{Dependency, DependencyDetails, Property, Rule, RuleDetails},
};

pub fn probe(probe: &Probe<'_>, value: &Value) -> serde_json::Result<()> {
    let forms: &[&str];
    let seen = match probe.path.as_str() {
        "manifest.properties{value}" => {
            forms = &["Boolean", "Integer", "Float", "String"];
            match Property::deserialize(value)? {
                Property::Boolean(_) => "Boolean",
                Property::Integer(_) => "Integer",
                Property::Float(_) => "Float",
                Property::String(_) => "String",
            }
        }
        "manifest.dependencies{value}" => {
            forms = &["Version", "Detailed"];
            match Dependency::deserialize(value)? {
                Dependency::Version(_) => "Version",
                Dependency::Detailed(_) => {
                    DependencyDetails::deserialize(probe.at("::Detailed", Input::Value(value)))?;
                    "Detailed"
                }
            }
        }
        "manifest.incompatible{value}" | "manifest.discouraged{value}" => {
            forms = &["Version", "Detailed"];
            match Rule::deserialize(value)? {
                Rule::Version(_) => "Version",
                Rule::Detailed(_) => {
                    RuleDetails::deserialize(probe.at("::Detailed", Input::Value(value)))?;
                    "Detailed"
                }
            }
        }
        "block.properties.random_ticks" => {
            forms = &["All", "States"];
            match RandomTicks::deserialize(value)? {
                RandomTicks::All(_) => "All",
                RandomTicks::States {
                    property: _,
                    values: _,
                } => {
                    buffered_fields(probe, value, "States", &["property", "values"]);
                    "States"
                }
            }
        }
        "block.open_sound" | "block.close_sound" => {
            forms = &["All", "Sites"];
            match GateSound::deserialize(value)? {
                GateSound::All(_) => "All",
                GateSound::Sites {
                    on_use: _,
                    on_explosion: _,
                    on_redstone: _,
                } => {
                    buffered_fields(probe, value, "Sites", &["use", "explosion", "redstone"]);
                    "Sites"
                }
            }
        }
        "block.state_properties{value}" => {
            forms = &["boolean", "enum"];
            match StateProperty::deserialize(value)? {
                StateProperty::Boolean => {
                    buffered_fields(probe, value, "boolean", &["type"]);
                    "boolean"
                }
                StateProperty::Enum { values: _ } => {
                    buffered_fields(probe, value, "enum", &["type", "values"]);
                    "enum"
                }
            }
        }
        "entity_type.sounds.ambient" | "entity_type.sounds.hurt" | "entity_type.sounds.death" => {
            forms = &["none", "id"];
            match SoundOverride::deserialize(value)? {
                SoundOverride::Mute(Mute::None) => "none",
                SoundOverride::Id(_) => "id",
            }
        }
        "entity_type.sounds.step" => {
            forms = &["none", "id", "sound"];
            match StepOverride::deserialize(value)? {
                StepOverride::Mute(Mute::None) => "none",
                StepOverride::Id(_) => "id",
                StepOverride::Sound(_) => {
                    buffered_fields(probe, value, "sound", &["sound", "volume", "pitch"]);
                    "sound"
                }
            }
        }
        "entity_type.passenger_attachments" => {
            // The reader accepts a height, a single point, or a list mixing both.
            forms = &["height", "point", "list of heights", "list of points"];
            let seen = match value {
                Value::Number(_) => [Some("height"), None],
                Value::Array(a) if a.len() == 3 && a.iter().all(Value::is_number) => {
                    [Some("point"), None]
                }
                Value::Array(a) => [
                    a.iter().any(Value::is_number).then_some("list of heights"),
                    a.iter().any(Value::is_array).then_some("list of points"),
                ],
                _ => unreachable!("admitted attachment: {value}"),
            };
            probe.coverage.borrow_mut().variants(
                &probe.path,
                forms.iter().copied(),
                seen.into_iter().flatten(),
            );
            return Ok(());
        }
        _ => return Ok(()),
    };
    probe
        .coverage
        .borrow_mut()
        .variants(&probe.path, forms.iter().copied(), [seen]);
    Ok(())
}

fn buffered_fields(probe: &Probe<'_>, value: &Value, variant: &str, fields: &[&str]) {
    probe.coverage.borrow_mut().keys(
        &format!("{}::{variant}", probe.path),
        fields,
        value.as_object().expect("an admitted struct variant"),
    );
}
