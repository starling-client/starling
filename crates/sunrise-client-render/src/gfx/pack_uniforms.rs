//! The client's `Frame` as the shadow light reads it: a `UniformFrameView` of the camera, the
//! sky, the fog and the lights, and the shadow matrices a post pass needs. The uniforms themselves
//! live in `sunrise_render_shaderpack::pack_uniforms`; they take the view this file builds.

use super::*;
use sunrise_render_core::{FogView, SkyView, UniformFrameView};
// The client's `Frame` carries the game's values by this name.
pub use sunrise_render_shaderpack::pack_uniforms::{PackInputs, PlayerStatus, cloud_height};
// The shadow matrices and timings its own tests build here, beside the `Frame`s they read; the
// frame timing is the game's too (a screenshot sequence replays a fixed clock through it).
pub use sunrise_render_shaderpack::pack_uniforms::FrameTiming;
#[cfg(test)]
pub use sunrise_render_shaderpack::pack_uniforms::{shadow_camera, shadow_culling_at};

/// The frame as the shadow light reads it: the camera, the sky and the eye, and no fog, lights or
/// entities (a shadow pass's uniforms read none of those).
fn view<'a>(frame: &'a Frame<'_>, lights: bool) -> UniformFrameView<'a> {
    UniformFrameView {
        camera: frame.camera,
        yaw: frame.yaw,
        pitch: frame.pitch,
        fov_degrees: frame.fov_degrees,
        eye: frame.eye,
        eye_rotation: frame.eye_rotation,
        first_person: frame.first_person,
        bob: frame.bob,
        spin: frame.spin,
        sky: SkyView {
            sun_angle: frame.sky.sun_angle,
            moon_angle: frame.sky.moon_angle,
            sky_color: frame.sky.sky_color,
            fog_color: frame.sky.fog_color,
        },
        distance: frame.distance,
        detail_distance: frame.detail_distance,
        depth_far: frame.depth_far,
        fog: match &frame.fog {
            Some(f) if lights => Some(FogView {
                color: [f.color[0], f.color[1], f.color[2]],
                environmental_start: f.environmental_start,
                environmental_end: f.environmental_end,
            }),
            _ => None,
        },
        night_vision: frame.night_vision,
        darkness: frame.darkness,
        dynamic_lights: if lights { frame.dynamic_lights } else { &[] },
        entities: if lights { frame.entities } else { &[] },
        // The shadow light reads no bolt position.
        lightning_bolt: None,
    }
}

pub fn shadow_light(frame: &Frame<'_>, shadow: &sunrise_shaders::Shadow) -> glam::Vec3 {
    sunrise_render_shaderpack::pack_uniforms::shadow_light(&view(frame, false), shadow)
}

/// `lightningBoltPosition` (spec 5.2) for the frame's bolts (`Frame::lightning.bolts`, the
/// draw list): the last one, in player space, w = 1; zeros when the list is empty.
pub fn lightning_bolt_position(
    bolts: &[crate::gfx::lightning::Bolt],
    camera: glam::DVec3,
) -> [f32; 4] {
    sunrise_render_shaderpack::pack_uniforms::lightning_bolt_position(
        bolts.last().map(|bolt| glam::DVec3::from_array(bolt.at)),
        camera,
    )
}

pub fn shadow_culling(frame: &Frame<'_>, shadow: &sunrise_shaders::Shadow) -> glam::Mat4 {
    sunrise_render_shaderpack::pack_uniforms::shadow_culling(&view(frame, false), shadow)
}
