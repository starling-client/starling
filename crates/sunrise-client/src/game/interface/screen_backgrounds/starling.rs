//! Starling's part of screen_backgrounds.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Whether this frame's blurred menu may be kept (`Frame::menu_blur_cache`): a paused
    /// single-player game, whose world does not move under the menu. Then whether the frame
    /// is drawn from the kept one, without the world: only from the second frame of such a
    /// menu on, so the first one blurs the world as it is. Any other frame forgets the kept
    /// one. Adapted from Vasysik/sunrise-android (`was_menu_blurred`, `skip_world_for_blur`,
    /// its commit 77b9e17f76), MIT, copyright (c) 2026 Ludovic Marie and the Horizon
    /// contributors.
    pub(in super::super) fn menu_blur_cache(&mut self) -> (bool, bool) {
        let radius = self.menu_blur();
        let keep = radius.is_some() && self.pauses_game() && !self.shows_panorama();
        let was = std::mem::replace(&mut self.menu_blur_kept, keep);
        let Some(renderer) = self.renderer.as_mut() else {
            return (keep, false);
        };
        if !keep || !was {
            renderer.forget_menu_blur();
            return (keep, false);
        }
        (
            keep,
            radius.is_some_and(|r| renderer.has_menu_blur_cache(r)),
        )
    }
}
