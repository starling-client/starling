//! Starling's part of inventory_input.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: `Screen.hasAltDown`, beside the two above, which native mods made for Sunrise call.
    pub(crate) fn has_alt_down(&self) -> bool {
        self.modifiers.alt_key()
    }

    /// The open screen's window (`leftPos`, `topPos`, `imageWidth`, `imageHeight`): the touch
    /// slots' and the mods' name for `container_image`.
    pub(in super::super) fn window_image(
        &self,
        batch: &HudBatch,
    ) -> sunrise_client::container_screen::Image {
        self.container_image(batch)
    }
}
