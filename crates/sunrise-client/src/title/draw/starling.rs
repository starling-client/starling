//! Starling's part of draw.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Title {
    /// `draw_with_items` without the render phases' callback, as the mods made for Sunrise's API call it
    /// (Starling).
    pub fn draw_with_items_old(
        &self,
        atlas: &HudAtlas,
        items: Option<&crate::item_render::ItemRenderer>,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        hide_splash: bool,
        hide_server_address: bool,
    ) {
        self.draw_with_items(
            atlas,
            items,
            batch,
            cursor,
            hide_splash,
            hide_server_address,
            &mut |_, _| {},
        );
    }
}
