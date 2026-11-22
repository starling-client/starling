use super::LIVING_TEMPLATES;
use super::entity_sounds::EntitySounds;
use serde::Deserialize;
use std::collections::BTreeMap;

/// an entity type the mod adds: a copy of a vanilla template's. A thrown one
/// is thrown from `item` and drawn as it; a living one is its template's mob
/// with other `attributes`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntityType {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    /// one of [`super::ENTITY_TEMPLATES`].
    pub template: String,
    /// Whether a living type may spawn and remain in Peaceful. Omitted inherits the template.
    #[serde(default, deserialize_with = "deserialize_peaceful_setting")]
    pub allowed_in_peaceful: Option<bool>,
    /// Renderer shadow overrides; omitted values inherit the template.
    #[serde(default, deserialize_with = "deserialize_shadow")]
    pub shadow_radius: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_shadow")]
    pub shadow_strength: Option<f32>,
    /// a thrown type's: one of the mod's own items, which throws it. A
    /// living type has none.
    #[serde(default)]
    pub item: Option<String>,
    /// a thrown type's: what a hit deals to an entity, as `thrown` damage.
    #[serde(default)]
    pub damage: f32,
    /// a living type's default attribute values that differ from its
    /// template's, by attribute id (`minecraft:max_health`): the template's
    /// own attributes only, checked against its row in `entity-types.json`
    /// when the content root builds.
    #[serde(default)]
    pub attributes: BTreeMap<String, f64>,
    /// passenger attachment points (`passenger_attachments`).
    #[serde(default, deserialize_with = "deserialize_passenger_attachments")]
    pub passenger_attachments: Vec<[f32; 3]>,
    /// Omitted slots inherit the template selection.
    #[serde(default)]
    pub sounds: EntitySounds,
}

fn deserialize_passenger_attachments<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<[f32; 3]>, D::Error> {
    use serde_json::Value;
    match Value::deserialize(d)? {
        Value::Number(n) => Ok(vec![[0.0, n.as_f64().unwrap_or(0.0) as f32, 0.0]]),
        Value::Array(a) if a.len() == 3 && a.iter().all(|v| v.is_number()) => {
            let p = [
                a[0].as_f64().unwrap_or(0.0) as f32,
                a[1].as_f64().unwrap_or(0.0) as f32,
                a[2].as_f64().unwrap_or(0.0) as f32,
            ];
            Ok(vec![p])
        }
        Value::Array(a) => a
            .into_iter()
            .map(|v| match v {
                Value::Number(n) => Ok([0.0, n.as_f64().unwrap_or(0.0) as f32, 0.0]),
                Value::Array(coords) if coords.len() == 3 => Ok([
                    coords[0]
                        .as_f64()
                        .ok_or_else(|| serde::de::Error::custom("expected float"))?
                        as f32,
                    coords[1]
                        .as_f64()
                        .ok_or_else(|| serde::de::Error::custom("expected float"))?
                        as f32,
                    coords[2]
                        .as_f64()
                        .ok_or_else(|| serde::de::Error::custom("expected float"))?
                        as f32,
                ]),
                _ => Err(serde::de::Error::custom(
                    "expected attachment point [x, y, z] or height y",
                )),
            })
            .collect(),
        _ => Err(serde::de::Error::custom(
            "expected attachment point or list",
        )),
    }
}

impl EntityType {
    /// whether the type copies a living mob ([`LIVING_TEMPLATES`]).
    pub fn is_living(&self) -> bool {
        LIVING_TEMPLATES.contains(&self.template.as_str())
    }
}

impl EntityType {
    pub(super) fn validate_thrown_overrides(&self, name: &str) -> Result<(), String> {
        if !self.attributes.is_empty() {
            return Err(format!(
                "{name}: only a living type has attributes (its template is {})",
                self.template
            ));
        }
        if self.allowed_in_peaceful.is_some() {
            return Err(format!(
                "{name}: only a living type may set allowed_in_peaceful (its template is {})",
                self.template
            ));
        }
        if self.shadow_radius.is_some() || self.shadow_strength.is_some() {
            return Err(format!(
                "{name}: only a living type has renderer shadow overrides"
            ));
        }
        if !self.passenger_attachments.is_empty() {
            return Err(format!(
                "{name}: only a living type has passenger attachments (its template is {})",
                self.template
            ));
        }
        if self.sounds != EntitySounds::default() {
            return Err(format!(
                "{name}: only a living type has sounds (its template is {})",
                self.template
            ));
        }
        Ok(())
    }
}

fn deserialize_peaceful_setting<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<bool>, D::Error> {
    bool::deserialize(d).map(Some)
}

fn deserialize_shadow<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    let value = f32::deserialize(d)?;
    if !value.is_finite() || value < 0.0 {
        return Err(serde::de::Error::custom(
            "shadow value must be finite and nonnegative",
        ));
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use crate::{Package, content::Content};

    #[test]
    fn only_living_types_accept_an_explicit_peaceful_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peaceful.smod");
        let name = "data/bird/sunrise/entity_type/mob.json";
        let manifest = r#"id = "bird"
name = "Bird"
version = "1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
        let read = |declaration: &str| {
            std::fs::write(
                &path,
                Package::write_files(manifest, &[(name, declaration.as_bytes())]).unwrap(),
            )
            .unwrap();
            let package = Package::read(&path).unwrap();
            Content::read(&path, &package.manifest)
        };
        let living =
            read(r#"{"template":"minecraft:wandering_trader","allowed_in_peaceful":false}"#)
                .unwrap();
        assert_eq!(living.entity_types[0].allowed_in_peaceful, Some(false));
        for value in ["false", "true", "null"] {
            let error = read(&format!(
                r#"{{"template":"minecraft:snowball","item":"bird:ball","allowed_in_peaceful":{value}}}"#
            ))
            .unwrap_err();
            assert!(error.contains(name), "{error}");
            if value != "null" {
                assert!(error.contains("allowed_in_peaceful"), "{error}");
            }
        }
        for value in ["null", "\"false\""] {
            let error = read(&format!(
                r#"{{"template":"minecraft:wandering_trader","allowed_in_peaceful":{value}}}"#
            ))
            .unwrap_err();
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn shadow_admission_rejects_negative_and_nonfinite_values_with_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shadow.smod");
        let name = "data/bird/sunrise/entity_type/mob.json";
        let manifest = r#"id = "bird"
name = "Bird"
version = "1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
        for key in ["shadow_radius", "shadow_strength"] {
            for value in ["-1", "1e100", "1e999", "NaN"] {
                let file = format!(r#"{{"template":"minecraft:zombie","{key}":{value}}}"#);
                std::fs::write(
                    &path,
                    Package::write_files(manifest, &[(name, file.as_bytes())]).unwrap(),
                )
                .unwrap();
                let package = Package::read(&path).unwrap();
                let error = Content::read(&path, &package.manifest).unwrap_err();
                assert!(error.contains(name), "{error}");
            }
        }
    }
}
