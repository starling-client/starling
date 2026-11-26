//! Declarative sound selection; omission remains distinct from explicit silence.
use serde::{Deserialize, Serialize, de::Error as _};
use serde_json::value::RawValue;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntitySounds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<SoundOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hurt: Option<SoundOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death: Option<SoundOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<StepOverride>,
    /// `LivingEntity.getSoundVolume`: the ambient, hurt and death sounds' volume.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_volume"
    )]
    pub volume: Option<f32>,
    /// The base of `LivingEntity.getVoicePitch`, in place of the adult's 1.0.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_voice_pitch"
    )]
    pub voice_pitch: Option<f32>,
    /// `Mob.getAmbientSoundInterval`, in ticks. Any int: vanilla negates and
    /// compares whatever the method returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient_interval: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum SoundOverride {
    Mute(Mute),
    Id(String),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum Mute {
    #[serde(rename = "none")]
    None,
}
/// The step slot also takes `playStepSound`'s own `playSound(sound, volume, pitch)`.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(untagged)]
pub enum StepOverride {
    Mute(Mute),
    /// Played at the block step's volume 0.15 and pitch 1.0.
    Id(String),
    Sound(StepSound),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StepSound {
    pub sound: String,
    #[serde(deserialize_with = "deserialize_step_volume")]
    pub volume: f32,
    #[serde(deserialize_with = "deserialize_step_pitch")]
    pub pitch: f32,
}
impl<'de> Deserialize<'de> for StepOverride {
    /// A string or the object form only: a derived struct would also read a
    /// positional array, and an untagged enum would hide which key is wrong.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, MapAccess, Visitor, value::MapAccessDeserializer};
        use std::{cell::Cell, fmt};

        struct StepVisitor<'a>(&'a Cell<bool>);
        impl<'de> Visitor<'de> for StepVisitor<'_> {
            type Value = StepOverride;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a sound id, \"none\" or {\"sound\", \"volume\", \"pitch\"}")
            }

            fn visit_str<E: Error>(self, id: &str) -> Result<Self::Value, E> {
                if id == "none" {
                    Ok(StepOverride::Mute(Mute::None))
                } else {
                    Ok(StepOverride::Id(id.to_owned()))
                }
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                // Forward the original map; each error already identifies the member it came from.
                self.0.set(true);
                StepSound::deserialize(MapAccessDeserializer::new(map)).map(StepOverride::Sound)
            }
        }

        // The parser can fail before entering the visitor (for example on a numeric overflow).
        // Name the step in that case, without relabeling an error from a member of the map.
        let entered_map = Cell::new(false);
        d.deserialize_any(StepVisitor(&entered_map)).map_err(|e| {
            if entered_map.get() {
                e
            } else {
                D::Error::custom(format_args!("sounds.step: {e}"))
            }
        })
    }
}

/// A float as Java reads one into a `float`, refused unless finite, with its key.
fn finite<'de, D: serde::Deserializer<'de>>(d: D, key: &str) -> Result<f32, D::Error> {
    // Decode the JSON token itself: even serde_json's f32 visitor can round an
    // integer outside i64 through f64 before handing it over.
    let raw = Box::<RawValue>::deserialize(d)
        .map_err(|e| D::Error::custom(format_args!("sounds.{key}: {e}")))?;
    let value = raw
        .get()
        .parse::<f32>()
        .map_err(|_| D::Error::custom(format_args!("sounds.{key}: expected a number")))?;
    if !value.is_finite() {
        return Err(D::Error::custom(format_args!(
            "sounds.{key}: number out of range"
        )));
    }
    Ok(value)
}
fn deserialize_volume<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    finite(d, "volume").map(Some)
}
fn deserialize_voice_pitch<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<f32>, D::Error> {
    finite(d, "voice_pitch").map(Some)
}
fn deserialize_step_volume<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    finite(d, "step.volume")
}
fn deserialize_step_pitch<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    finite(d, "step.pitch")
}
impl EntitySounds {
    pub fn validate(&self, entity: &str) -> Result<(), String> {
        let step = match &self.step {
            Some(StepOverride::Id(id)) | Some(StepOverride::Sound(StepSound { sound: id, .. })) => {
                Some(id)
            }
            _ => None,
        };
        fn id(value: &Option<SoundOverride>) -> Option<&String> {
            match value {
                Some(SoundOverride::Id(id)) => Some(id),
                _ => None,
            }
        }
        for (slot, id) in [
            ("ambient", id(&self.ambient)),
            ("hurt", id(&self.hurt)),
            ("death", id(&self.death)),
            ("step", step),
        ] {
            let Some(id) = id else { continue };
            let valid = id.split_once(':').is_some_and(|(n, p)| {
                !n.is_empty()
                    && !p.is_empty()
                    && n.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b)
                    })
                    && p.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-/".contains(&b)
                    })
            });
            if !valid {
                return Err(format!(
                    "{entity}: sounds.{slot}: invalid full sound id {id:?}"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "entity_sounds_tests.rs"]
mod tests;
