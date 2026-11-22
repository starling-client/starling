//! A mod's `gui_paint` poses (`PoseOp`): a `Rotate` by a NaN or infinite angle is refused by
//! `hud_layout::pose_apply`, which leaves the pose as it was, and named here in the log.
use std::sync::atomic::{AtomicBool, Ordering};
use sunrise_client::hud_layout::{GuiDraw, pose_refused};

/// Whether `E_GUI_POSE_ROTATE_NOT_FINITE` was logged: once a run, since a mod draws each frame.
static LOGGED: AtomicBool = AtomicBool::new(false);

/// Names the first pose op of `draws` that the paint refuses, once a run. It reads the mod's
/// own buffer and allocates nothing.
pub(crate) fn report_refused_poses(draws: &[GuiDraw]) -> bool {
    let refused = draws.iter().find_map(|d| match d {
        GuiDraw::Push(ops) => pose_refused(ops),
        _ => None,
    });
    let Some(op) = refused else {
        return false;
    };
    if !LOGGED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "E_GUI_POSE_ROTATE_NOT_FINITE: gui_paint: {op:?} refused, the pose is left as it was \
             (logged once)"
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::report_refused_poses;
    use sunrise_client::hud_layout::{GuiDraw, PoseOp};

    #[test]
    fn only_a_non_finite_rotation_is_reported() {
        let push = |op| GuiDraw::Push(vec![PoseOp::Translate(1.0, 2.0), op]);
        assert!(!report_refused_poses(&[
            push(PoseOp::Rotate(1.5)),
            GuiDraw::Pop
        ]));
        assert!(!report_refused_poses(&[push(PoseOp::Scale(f32::NAN, 1.0))]));
        for angle in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(report_refused_poses(&[push(PoseOp::Rotate(angle))]));
        }
    }
}
