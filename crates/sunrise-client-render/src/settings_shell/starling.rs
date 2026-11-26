//! Starling's part of settings_shell.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl<S: Screen> Shell<S> {
    /// The page the plate is headed with.
    #[cfg(test)]
    pub(crate) fn current_page(&mut self, s: &Settings, size: (f32, f32)) -> Option<S::Page> {
        let view = self.view(s, size);
        self.current(&view)
    }

    #[cfg(target_os = "android")]
    pub(crate) fn preview_row(
        &mut self,
        s: &Settings,
        size: (f32, f32),
        at: (f32, f32),
    ) -> Option<S::Row> {
        let view = self.view(s, size);
        let focused = match (self.keyed, self.focus) {
            (true, Some(Target::Row(row))) => Some(row),
            _ => None,
        };
        self.dragging
            .or_else(|| self.row_at(&view, at).map(|(row, _)| row.id).or(focused))
    }

    #[cfg(target_os = "android")]
    pub(crate) fn selected_page(&self) -> Option<S::Page> {
        self.page
    }
}
