//! Starling's part of recipe_book.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: the search box while it takes the keys, for the keyboard's editor on Android
    /// (touch/text_mirror.rs).
    pub(in super::super) fn recipe_search_box(&mut self) -> Option<&mut EditBox> {
        if !self.recipe_search_focused() {
            return None;
        }
        self.recipe_ui.search.as_mut()
    }
}
