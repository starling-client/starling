//! Starling's part of layout.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl OptionsUi {
    /// The preview and the options list share the available width.
    pub(in super::super) fn wardrobe_columns(&self, width: f32) -> Option<(f32, f32, f32)> {
        if self.page() != Page::Skin || !self.wardrobe_enabled {
            return None;
        }
        let total = (width - 26.0).min(480.0);
        let preview = (total * 0.33).floor();
        Some(((width - total) / 2.0, preview, total - preview - 12.0))
    }

    pub fn wardrobe_preview_rect(&self, width: f32, height: f32) -> Option<[f32; 4]> {
        let (left, preview, _) = self.wardrobe_columns(width)?;
        let bottom = height - 59.0;
        (bottom > 33.0).then_some([left, 33.0, left + preview, bottom])
    }

    /// The calibration page's lines and buttons, from the game while it shows: the page lays out
    /// again only when one of them changed, so a tap lands on the buttons it shows.
    pub fn set_calibrate(&mut self, notes: Vec<String>, buttons: Vec<String>) {
        if self.calibrate_notes != notes || self.calibrate != buttons {
            self.calibrate_notes = notes;
            self.calibrate = buttons;
            self.relayout();
        }
    }
}
