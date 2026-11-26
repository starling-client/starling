//! Vocabulary for GUI poses, texture blits and heart sprites.
// no wire: a pose operation is passed directly to an in-process canvas scope.

/// Keep a listener's replacement within `Player.getAttackStrengthScale`'s output range.
pub(crate) fn validate_hotbar_attack_indicator_progress(
    _accepted: &f32,
    proposed: &f32,
) -> Result<(), &'static str> {
    if (0.0..=1.0).contains(proposed) {
        Ok(())
    } else {
        Err("progress")
    }
}

/// A `Matrix3x2fStack` operation applied in [`crate::GuiCanvas::scope`] to a copy of the current pose.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
pub enum PoseOp {
    /// Move the origin along the current axes by the given GUI pixels.
    Translate(f32, f32),
    /// Scale the current axes by the given factors about the current origin.
    Scale(f32, f32),
    /// `Matrix3x2fStack.rotate(angle)`: a turn by `angle` radians about the current pose's
    /// origin, applied after the ops before it, as vanilla composes them. Positive angles turn
    /// clockwise on screen (y grows downwards). Everything drawn in the child scope follows
    /// its pose. [`crate::GuiCanvas::scope`] refuses a non-finite angle with [`crate::GuiDrawError::NotFinite`]
    /// before calling the drawing callback.
    Rotate(f32),
}

// no wire: the blend is passed directly to an in-process blit.
/// The render pipeline of a [`crate::GuiCanvas::blit`]. Only `Textured` and `Crosshair` are
/// available to [`crate::GuiCanvas::blit_sprite`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
pub enum Blend {
    /// `GUI_TEXTURED`.
    Textured,
    /// `VIGNETTE`: `dst * (1 - src)`.
    Vignette,
    /// `GUI_NAUSEA_OVERLAY`: additive.
    Nausea,
    /// `CROSSHAIR`: `BlendFunction.INVERT`, the colour under it inverted.
    Crosshair,
}
impl Blend {
    pub fn pipeline(self) -> &'static str {
        match self {
            Blend::Textured => "pipeline/gui_textured",
            Blend::Vignette => "pipeline/vignette",
            Blend::Nausea => "pipeline/gui_nausea_overlay",
            Blend::Crosshair => "pipeline/crosshair",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum HeartKind {
    Normal,
    Poisoned,
    Withered,
    Frozen,
}
