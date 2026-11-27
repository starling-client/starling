//! Starling's part of create_world.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

pub(super) fn normal_world() -> crate::save::WorldPreset {
    crate::save::WorldPreset::Normal
}

/// `WorldOptions::world_type` as its registry id, for the options Starling keeps across a restart.
pub(super) mod world_type_id {
    use crate::save::WorldPreset;

    pub fn serialize<S: serde::Serializer>(preset: &WorldPreset, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(preset.id())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<WorldPreset, D::Error> {
        let id: String = serde::Deserialize::deserialize(d)?;
        Ok(WorldPreset::of_id(&id).unwrap_or(WorldPreset::Normal))
    }
}
