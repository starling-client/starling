//! Starling's part of pointer_capture.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Whether `settle_after_run` grabs again: a window whose grab is the system's (a winit grab
/// mode) holds it exactly while the game has it grabbed. Android's grab is the activity's pointer
/// capture, never a grab mode (`capture` leaves `capture_mode` at None), so there the two never
/// match: grabbing again every frame asked Android for the capture some 120 times a second, threw
/// each frame's first mouse move away (`set_ignore_first_move`) and pressed the held keys again
/// (`restore_held_keys`). The view stuttered and jumped with a mouse (#467, #499), and a key held
/// down (sprint's Ctrl) was pressed anew every frame.
pub(super) fn regrab_after_run(
    android: bool,
    internal_grab: bool,
    captured: bool,
    grab_mode: bool,
) -> bool {
    !android && !internal_grab && captured != grab_mode
}

impl Game<'_> {
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
