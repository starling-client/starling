//! Starling's part of create_world.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

pub(super) fn normal_world() -> crate::save::WorldPreset {
    crate::save::WorldPreset::Normal
}
