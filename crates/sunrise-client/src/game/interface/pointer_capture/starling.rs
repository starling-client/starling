//! Starling's part of pointer_capture.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    // Android's alone; the desktop build doesn't reach it.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    /// The mouse's raw motion (`DeviceEvent::MouseMotion`, or Android's from the input hook):
    /// MouseHandler.onMove with the pointer grabbed, applied once per frame.
    pub(in super::super) fn grabbed_mouse_motion(&mut self, x: f64, y: f64) {
        // A composited view renders offscreen but its keyboard/mouse owner still
        // controls a visible native window. A headless offscreen run never does.
        if self.pending_save.is_none()
            && self.captured
            && self.focused
            && !self.options.offscreen
            && (self.screen == Screen::Playing || self.mod_screen_grabs_pointer())
        {
            // Sunrise's: the camera's turn, or a grabbing mod screen's motion.
            self.grabbed_pointer_motion([x, y]);
        }
    }
}
