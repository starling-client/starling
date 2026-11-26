//! Starling's part of draw.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl OptionsUi {
    #[cfg(target_os = "android")]
    pub fn display_open(&self) -> bool {
        Self::graphics_page(self.page()).is_some()
            && self.graphics.selected_page() == Some(crate::graphics_ui::Page::Display)
    }

    #[cfg(target_os = "android")]
    pub fn safe_zone_preview(
        &mut self,
        s: &Settings,
        size: (f32, f32),
        cursor: (f32, f32),
    ) -> Option<crate::android_display::SafeZone> {
        Self::graphics_page(self.page())?;
        match self.graphics.preview_row(s, size, cursor)? {
            crate::graphics_ui::Item::Display(option) => option.margin_preview(),
            _ => None,
        }
    }
}
