//! Starling's part of construction.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// How many frames one shadow map serves: `SUNRISE_SHADER_SHADOW_EVERY`, else two on a phone. The
/// map costs the same wherever the player looks, and what casts shadows barely moves in a frame.
pub(super) fn shadow_every() -> u32 {
    std::env::var("SUNRISE_SHADER_SHADOW_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(target_os = "android") { 2 } else { 1 })
        .max(1)
}

/// Whether the next pack draws terrain's depth first (`set_prepass`).
pub(super) static PREPASS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The game's choice for the packs it loads from now on (`prepass_wanted`).
pub fn set_prepass(on: bool) {
    PREPASS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The Auto profile, or `SUNRISE_SHADER_PACK_PREPASS=1`: solid terrain draws its depth first with
/// the program's own vertex stage and only the atlas' alpha test, then the program shades over it
/// (writing depth too, for the cutout edges the prepass left out), so a phone's GPU can drop what is hidden before running the pack's fragment
/// stage (grass and leaves stacked in front of each other otherwise run it once per layer).
pub fn prepass_wanted() -> bool {
    PREPASS.load(std::sync::atomic::Ordering::Relaxed)
        || std::env::var_os("SUNRISE_SHADER_PACK_PREPASS").is_some()
}
