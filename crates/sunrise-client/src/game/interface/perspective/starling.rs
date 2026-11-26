//! Starling's part of perspective.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    pub(super) fn player_preview(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        size: f32,
        offset_y: f32,
        cursor: (f32, f32),
        spin: f32,
    ) {
        if let Some(m) = self.player_model.as_ref() {
            let data = self.res.mob_data();
            self.mob_preview(m, data, batch, rect, size, offset_y, cursor, spin, true);
        }
    }
}
