//! Starling's part of draw.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl OptionsUi {
    /// Sunrise Settings at Starling's Display page (`graphics_ui::Page::Display`), whose margins
    /// the world behind it previews.
    #[cfg(target_os = "android")]
    pub fn display_open(&self) -> bool {
        self.page() == Page::Sunrise
            && self.sunrise.selected_page()
                == Some(crate::sunrise_ui::Tab::Graphics(
                    crate::graphics_ui::Page::Display,
                ))
    }

    #[cfg(target_os = "android")]
    pub fn safe_zone_preview(
        &mut self,
        s: &Settings,
        size: (f32, f32),
        cursor: (f32, f32),
    ) -> Option<crate::android_display::SafeZone> {
        if self.page() != Page::Sunrise {
            return None;
        }
        match self.sunrise.preview_row(s, size, cursor)? {
            crate::sunrise_ui::SunriseRow::Graphics(crate::graphics_ui::Item::Display(option)) => {
                option.margin_preview()
            }
            _ => None,
        }
    }
}
