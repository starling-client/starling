//! Pure frame views and input structures for uniforms and shader packs.

use glam::{DVec3, Mat4};

/// Pure sky parameters needed for lighting, sun/moon angles, and uniforms.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkyView {
    pub sun_angle: f32,
    pub moon_angle: f32,
    pub sky_color: [f32; 3],
    pub fog_color: [f32; 3],
}

/// Pure fog parameters needed for shader pack environmental fog.
#[derive(Clone, Copy, Debug, Default)]
pub struct FogView {
    pub color: [f32; 3],
    pub environmental_start: f32,
    pub environmental_end: f32,
}

/// A decoupled view of frame inputs needed by shader pack uniforms.
pub struct UniformFrameView<'a> {
    pub camera: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub fov_degrees: f32,
    pub eye: DVec3,
    pub eye_rotation: [f32; 2],
    pub first_person: bool,
    pub bob: Mat4,
    pub spin: Mat4,
    pub sky: SkyView,
    pub distance: f32,
    pub detail_distance: f32,
    pub depth_far: f32,
    pub fog: Option<FogView>,
    pub night_vision: f32,
    pub darkness: (f32, f32),
    pub dynamic_lights: &'a [[f32; 4]],
    pub entities: &'a [crate::entity::EntityVertex],
    /// The last bolt of the frame's lightning draw list, where it struck, for
    /// `lightningBoltPosition`; `None` when no bolt is drawn.
    pub lightning_bolt: Option<DVec3>,
}

/// Maximum number of dynamic lights supported in shader passes.
pub const MAX_DYNAMIC_LIGHTS: usize = 16;
