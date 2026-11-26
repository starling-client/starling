//! Starling's part of batch.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Which take of a letter a hand-written font draws at the `i`th character of text at `x, y`:
/// the same each frame for text that stays put.
pub(in super::super) fn take_seed(x: f32, y: f32, i: usize) -> u32 {
    let mut h = (x.round() as i32 as u32).wrapping_mul(0x9e37_79b9)
        ^ (y.round() as i32 as u32).wrapping_mul(0x85eb_ca6b)
        ^ (i as u32).wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h.wrapping_mul(0x2c1b_3c6d) >> 7
}

impl HudBatch {
    /// Starling: the one-argument constructor native mods made for Sunrise before each view had
    /// its own scale settings, laid out with the last batch's (`HudBatch::new`); auto before any.
    /// `native_mod_adapt.py` points their `HudBatch::new(size)` here.
    pub fn new_old(window: [u32; 2]) -> Self {
        let bits = LAST_GUI_SCALE.load(std::sync::atomic::Ordering::Relaxed);
        Self::new(
            window,
            GuiScale {
                requested: bits & 0x7fff_ffff,
                unicode: bits >> 31 == 1,
            },
        )
    }
}

/// Starling: the scale inputs of the last batch laid out (`HudBatch::new`), the requested scale in
/// the low bits and Force Unicode in the top one, for [`HudBatch::new_old`].
pub(super) static LAST_GUI_SCALE: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);
