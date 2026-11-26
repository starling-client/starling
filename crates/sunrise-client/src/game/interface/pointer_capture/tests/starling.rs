//! Starling's tests of pointer_capture.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Android's pointer capture has no grab mode: a captured mouse stays as it is, frame after
/// frame (#467, #499). A computer's window whose grab went is grabbed again, outside a run.
#[test]
fn a_captured_android_mouse_is_not_grabbed_again_every_frame() {
    assert!(!regrab_after_run(true, false, true, false));
    assert!(!regrab_after_run(true, false, false, false));
    assert!(regrab_after_run(false, false, true, false));
    assert!(regrab_after_run(false, false, false, true));
    assert!(!regrab_after_run(false, false, true, true));
    assert!(!regrab_after_run(false, true, true, false));
}
