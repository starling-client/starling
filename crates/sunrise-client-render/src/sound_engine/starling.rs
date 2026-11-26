//! Starling's part of sound_engine.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl SoundEngine {
    /// Starling: a fresh unseeded random (`RandomSource.create()`), as Sunrise's
    /// `SoundEngine::unseeded` gave the native mods made for it.
    pub fn unseeded() -> LegacyRandom {
        LegacyRandom::new(crate::random_source::unique_seed())
    }
}
