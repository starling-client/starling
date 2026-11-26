//! Starling's part of pack_uniforms.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The view of a shadow map drawn with `view` from `drawn`, seen from `camera`. A pack gives it
/// player space, which moves with the camera; the map stays where it was drawn.
pub(super) fn kept_shadow_view(view: Mat4, drawn: DVec3, camera: DVec3) -> Mat4 {
    view * Mat4::from_translation((camera - drawn).as_vec3())
}

impl FrameUniforms {
    /// Whether the shadow map was drawn near `camera`, so a frame may keep it: a teleport draws
    /// it again at once.
    pub fn shadow_near(&self, camera: DVec3) -> bool {
        self.shadow_drawn
            .is_some_and(|(drawn, ..)| drawn.distance_squared(camera) < 16.0 * 16.0)
    }
}
